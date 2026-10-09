//! UDP session: token handshake, keep-alive and request/response matching.
//!
//! The device only accepts control commands that carry the token returned by
//! `udp_sess_new`. Yeelight documents that the session must be refreshed with
//! `udp_sess_keep_alive`; the official Chroma connector sends it every 3 s and
//! treats three unanswered keep-alives as a lost session.

use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{debug, trace, warn};

use super::packets::{Call, Incoming, ReplyOutcome, Token, encode_request, parse_incoming, redact};
use super::transport::{MAX_DATAGRAM, Transport};
use super::{ProtocolError, Result};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// How long to wait for a reply to one request.
    pub reply_timeout: Duration,
    /// `udp_sess_new` attempts before giving up.
    pub handshake_attempts: u32,
    /// How often [`Session::keep_alive_due`] asks for a keep-alive.
    pub keep_alive_every: Duration,
    /// Value announced in `udp_sess_keep_alive` (`keeplive_interval`, seconds).
    pub announced_interval_s: u32,
    /// Consecutive unanswered keep-alives after which the session is dropped.
    pub max_missed_keep_alives: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            reply_timeout: Duration::from_millis(600),
            handshake_attempts: 3,
            keep_alive_every: Duration::from_secs(3),
            announced_interval_s: 10,
            max_missed_keep_alives: 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub sent: u64,
    pub received: u64,
    /// Datagrams that did not match what we were waiting for.
    pub ignored: u64,
    pub handshakes: u64,
    pub last_error: Option<String>,
}

pub struct Session<T: Transport> {
    transport: T,
    cfg: SessionConfig,
    token: Option<Token>,
    next_id: u64,
    last_keep_alive: Option<Instant>,
    missed_keep_alives: u32,
    stats: SessionStats,
    buf: Vec<u8>,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T, cfg: SessionConfig) -> Self {
        Self {
            transport,
            cfg,
            token: None,
            next_id: 1,
            last_keep_alive: None,
            missed_keep_alives: 0,
            stats: SessionStats::default(),
            buf: vec![0; MAX_DATAGRAM],
        }
    }

    pub fn is_open(&self) -> bool {
        self.token.is_some()
    }

    pub fn stats(&self) -> &SessionStats {
        &self.stats
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Performs the `udp_sess_new` handshake, replacing any previous token.
    pub fn open(&mut self) -> Result<()> {
        self.token = None;
        let call = Call::session_new();
        for attempt in 1..=self.cfg.handshake_attempts.max(1) {
            let id = self.take_id();
            self.send_raw(&encode_request(id, &call, None)?)?;
            let deadline = Instant::now() + self.cfg.reply_timeout;
            // The official example answers `id:1` with `id:2`, so the handshake
            // reply is matched by method rather than by id.
            let found = self.wait_for(deadline, |msg| match msg {
                Incoming::SessionToken { token, .. } => Some(token),
                _ => None,
            })?;
            if let Some(token) = found {
                debug!(attempt, "UDP session established");
                self.token = Some(token);
                self.stats.handshakes += 1;
                self.missed_keep_alives = 0;
                self.last_keep_alive = None;
                // The token proves the session exists; an unanswered first
                // keep-alive only counts as a miss (see `keep_alive`).
                return match self.keep_alive() {
                    Err(ProtocolError::Timeout) => Ok(()),
                    other => other,
                };
            }
            debug!(attempt, "no udp_sess_token reply");
        }
        self.fail(ProtocolError::Timeout)
    }

    /// Forgets the token locally. The protocol has no documented close command.
    pub fn close(&mut self) {
        self.token = None;
    }

    pub fn keep_alive_due(&self, now: Instant) -> bool {
        self.is_open() && self.last_keep_alive.is_none_or(|t| now.duration_since(t) >= self.cfg.keep_alive_every)
    }

    /// Sends `udp_sess_keep_alive`. Drops the session after too many misses.
    pub fn keep_alive(&mut self) -> Result<()> {
        self.last_keep_alive = Some(Instant::now());
        let call = Call::keep_alive(self.cfg.announced_interval_s);
        match self.request(&call) {
            Ok(_) => {
                self.missed_keep_alives = 0;
                Ok(())
            }
            Err(ProtocolError::Timeout) => {
                self.missed_keep_alives += 1;
                warn!(missed = self.missed_keep_alives, "keep-alive unanswered");
                if self.missed_keep_alives >= self.cfg.max_missed_keep_alives {
                    self.token = None;
                    return self.fail(ProtocolError::NoSession);
                }
                Err(ProtocolError::Timeout)
            }
            Err(e) => Err(e),
        }
    }

    /// Sends a call and waits for the reply with the same id.
    pub fn request(&mut self, call: &Call) -> Result<Vec<Value>> {
        let token = self.token.clone().ok_or(ProtocolError::NoSession)?;
        let id = self.take_id();
        self.send_raw(&encode_request(id, call, Some(&token))?)?;
        let deadline = Instant::now() + self.cfg.reply_timeout;
        let reply = self.wait_for(deadline, |msg| match msg {
            Incoming::Reply { id: rid, outcome, token: rtoken } if rid == id => {
                if rtoken.as_ref().is_some_and(|t| *t != token) {
                    return None;
                }
                Some(outcome)
            }
            _ => None,
        })?;
        match reply {
            Some(ReplyOutcome::Ok(values)) => Ok(values),
            Some(ReplyOutcome::Error { code, message }) => Err(self.device_error(code, message)),
            None => self.fail(ProtocolError::Timeout),
        }
    }

    /// Sends a call without waiting (used for LED frames), then drains any
    /// replies already queued so the socket buffer cannot grow. Fails with
    /// `NoSession` if a drained reply reported that the session is gone.
    pub fn send(&mut self, call: &Call) -> Result<u64> {
        let token = self.token.clone().ok_or(ProtocolError::NoSession)?;
        let id = self.take_id();
        self.send_raw(&encode_request(id, call, Some(&token))?)?;
        self.drain()?;
        if !self.is_open() {
            return Err(ProtocolError::NoSession);
        }
        Ok(id)
    }

    /// Reads and logs everything currently queued, without blocking.
    pub fn drain(&mut self) -> Result<usize> {
        let mut n = 0;
        while n < 64 {
            match self.transport.recv(&mut self.buf, Duration::ZERO)? {
                Some(len) => {
                    n += 1;
                    self.stats.received += 1;
                    let text = String::from_utf8_lossy(&self.buf[..len]);
                    trace!(packet = %redact(text.trim_end()), "recv (drained)");
                    if let Some(Ok(Incoming::Reply { outcome: ReplyOutcome::Error { code, message }, .. })) =
                        text.lines().find(|l| !l.trim().is_empty()).map(parse_incoming)
                    {
                        warn!(code, %message, "device rejected a fire-and-forget command");
                        self.device_error(code, message);
                    }
                }
                None => break,
            }
        }
        Ok(n)
    }

    /// Records a device error; drops the session if it looks like an expired
    /// token. Assumption: errors mentioning the token/session mean it expired.
    fn device_error(&mut self, code: i64, message: String) -> ProtocolError {
        let lower = message.to_ascii_lowercase();
        if lower.contains("token") || lower.contains("session") {
            self.token = None;
        }
        let e = ProtocolError::Device { code, message };
        self.stats.last_error = Some(e.to_string());
        e
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        // Keep ids small and positive; the device echoes them back.
        self.next_id = if self.next_id >= 999_999 { 1 } else { self.next_id + 1 };
        id
    }

    fn send_raw(&mut self, packet: &str) -> Result<()> {
        trace!(packet = %redact(packet.trim_end()), "send");
        self.transport.send(packet.as_bytes()).or_else(|e| self.fail(e.into()))?;
        self.stats.sent += 1;
        Ok(())
    }

    /// Receives until `pick` accepts a message or the deadline passes.
    fn wait_for<R>(&mut self, deadline: Instant, mut pick: impl FnMut(Incoming) -> Option<R>) -> Result<Option<R>> {
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let Some(len) = self.transport.recv(&mut self.buf, deadline - now)? else {
                // Transports return None only after waiting (or on an ICMP error).
                return Ok(None);
            };
            self.stats.received += 1;
            let text = String::from_utf8_lossy(&self.buf[..len]).into_owned();
            trace!(packet = %redact(text.trim_end()), "recv");
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                match parse_incoming(line) {
                    Ok(msg) => {
                        if let Some(r) = pick(msg) {
                            return Ok(Some(r));
                        }
                        self.stats.ignored += 1;
                    }
                    Err(e) => {
                        debug!(error = %e, "ignoring malformed datagram");
                        self.stats.ignored += 1;
                    }
                }
            }
        }
    }

    fn fail<R>(&mut self, e: ProtocolError) -> Result<R> {
        self.stats.last_error = Some(e.to_string());
        Err(e)
    }
}
