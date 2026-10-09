//! JSON packet encoding/decoding for the Yeelight UDP protocol.
//!
//! Requests are single-line JSON objects terminated by `\r\n`:
//! `{"id":1,"method":"set_bright","params":[10,"smooth",500],"token":"..."}`.
//! The device replies with `{"id":1,"result":["ok"],...}` or
//! `{"id":1,"error":{"code":-1,"message":"..."}}`; the session handshake reply is
//! `{"id":N,"method":"udp_sess_token","params":{"token":"..."}}`.

use std::borrow::Cow;
use std::fmt;

use serde_json::{Value, json};

use super::{ProtocolError, Result, Rgb};

/// Maximum number of addressable LEDs we allow in one `update_leds` frame.
/// 120 is the channel limit used by the SignalRGB driver for Chameleon2; it is
/// not a confirmed physical LED count.
pub const MAX_LEDS: usize = 120;

/// Upper bound for an outgoing datagram, kept below a typical Ethernet MTU so
/// the packet is never IP-fragmented.
pub const MAX_PACKET: usize = 1400;

pub const SESSION_TOKEN_METHOD: &str = "udp_sess_token";

/// UDP session token. Never printed: `Debug`/`Display` are redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn parse(s: &str) -> Result<Self> {
        let ok = (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric());
        if ok { Ok(Self(s.to_owned())) } else { Err(ProtocolError::InvalidResponse("malformed session token".into())) }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(***)")
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// Transition passed to power/brightness/colour commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Sudden,
    /// Smooth transition in milliseconds (the classic LAN spec requires >= 30).
    Smooth(u32),
}

impl Effect {
    fn params(self) -> (&'static str, u32) {
        match self {
            Self::Sudden => ("sudden", 0),
            Self::Smooth(ms) => ("smooth", ms.max(30)),
        }
    }
}

/// A method call without id/token; [`encode_request`] turns it into a packet.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: Cow<'static, str>,
    pub params: Value,
}

impl Call {
    pub fn raw(method: impl Into<String>, params: Value) -> Self {
        Self { method: Cow::Owned(method.into()), params }
    }

    pub fn session_new() -> Self {
        Self::fixed("udp_sess_new", json!([]))
    }

    /// Keep-alive in the form used by Yeelight's own UDP spec/connector.
    pub fn keep_alive(interval_s: u32) -> Self {
        Self::fixed("udp_sess_keep_alive", json!(["keeplive_interval", interval_s.to_string()]))
    }

    pub fn set_power(on: bool, effect: Effect) -> Self {
        let (e, ms) = effect.params();
        Self::fixed("set_power", json!([if on { "on" } else { "off" }, e, ms]))
    }

    pub fn set_bright(percent: u8, effect: Effect) -> Result<Self> {
        if !(1..=100).contains(&percent) {
            return Err(ProtocolError::InvalidInput(format!("brightness {percent} not in 1..=100")));
        }
        let (e, ms) = effect.params();
        Ok(Self::fixed("set_bright", json!([percent, e, ms])))
    }

    /// Black cannot be expressed with `set_rgb`; use `set_power(false)` instead.
    pub fn set_rgb(color: Rgb, effect: Effect) -> Result<Self> {
        if color == Rgb::BLACK {
            return Err(ProtocolError::InvalidInput("set_rgb cannot set black; power off instead".into()));
        }
        let (e, ms) = effect.params();
        Ok(Self::fixed("set_rgb", json!([color.to_u24(), e, ms])))
    }

    pub fn activate_fx_mode(mode: &str) -> Self {
        Self::fixed("activate_fx_mode", json!([{ "mode": mode }]))
    }

    /// Firmware 36 answers `ok` to any params; we mirror `activate_fx_mode`.
    pub fn deactivate_fx_mode(mode: &str) -> Self {
        Self::fixed("deactivate_fx_mode", json!([{ "mode": mode }]))
    }

    pub fn update_leds(colors: &[Rgb]) -> Result<Self> {
        Ok(Self::fixed("update_leds", json!([encode_leds(colors)?])))
    }

    fn fixed(method: &'static str, params: Value) -> Self {
        Self { method: Cow::Borrowed(method), params }
    }
}

/// Serialises a request. Keys are emitted in the order id, method, params, token.
pub fn encode_request(id: u64, call: &Call, token: Option<&Token>) -> Result<String> {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(id));
    obj.insert("method".into(), json!(call.method));
    obj.insert("params".into(), call.params.clone());
    if let Some(t) = token {
        obj.insert("token".into(), json!(t.as_str()));
    }
    let mut s = serde_json::to_string(&Value::Object(obj)).map_err(|e| ProtocolError::InvalidInput(e.to_string()))?;
    s.push_str("\r\n");
    if s.len() > MAX_PACKET {
        return Err(ProtocolError::InvalidInput(format!("packet is {} bytes, limit is {MAX_PACKET}", s.len())));
    }
    Ok(s)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Packs LED colours for `update_leds`: the RGB bytes of all LEDs, in order,
/// encoded as standard Base64. Three bytes per LED means four characters per LED
/// and never any padding.
pub fn encode_leds(colors: &[Rgb]) -> Result<String> {
    if colors.is_empty() || colors.len() > MAX_LEDS {
        return Err(ProtocolError::InvalidInput(format!("LED count {} not in 1..={MAX_LEDS}", colors.len())));
    }
    let mut out = String::with_capacity(colors.len() * 4);
    for c in colors {
        let v = c.to_u24();
        for shift in [18, 12, 6, 0] {
            out.push(B64[((v >> shift) & 0x3F) as usize] as char);
        }
    }
    Ok(out)
}

/// Inverse of [`encode_leds`]; used by tests and diagnostics.
pub fn decode_leds(s: &str) -> Option<Vec<Rgb>> {
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let idx = |b: u8| B64.iter().position(|&x| x == b).map(|p| p as u32);
    s.as_bytes()
        .chunks(4)
        .map(|ch| {
            let v = ch.iter().try_fold(0u32, |acc, &b| Some((acc << 6) | idx(b)?))?;
            Some(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
        })
        .collect()
}

/// A decoded datagram from the device.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    SessionToken {
        id: Option<u64>,
        token: Token,
    },
    Reply {
        id: u64,
        outcome: ReplyOutcome,
        token: Option<Token>,
    },
    /// Any other message carrying a `method` (e.g. property notifications).
    Notification {
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReplyOutcome {
    Ok(Vec<Value>),
    Error { code: i64, message: String },
}

/// Parses one JSON message (surrounding whitespace / `\r\n` allowed).
pub fn parse_incoming(text: &str) -> Result<Incoming> {
    let v: Value =
        serde_json::from_str(text.trim()).map_err(|e| ProtocolError::InvalidResponse(format!("not JSON: {e}")))?;
    let obj = v.as_object().ok_or_else(|| ProtocolError::InvalidResponse("not a JSON object".into()))?;
    let id = obj.get("id").and_then(Value::as_u64);

    if let Some(method) = obj.get("method").and_then(Value::as_str) {
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        if method == SESSION_TOKEN_METHOD {
            let token = params
                .get("token")
                .and_then(Value::as_str)
                .ok_or_else(|| ProtocolError::InvalidResponse("udp_sess_token without token".into()))?;
            return Ok(Incoming::SessionToken { id, token: Token::parse(token)? });
        }
        return Ok(Incoming::Notification { method: method.to_owned(), params });
    }

    let id = id.ok_or_else(|| ProtocolError::InvalidResponse("reply without numeric id".into()))?;
    let token = match obj.get("token").and_then(Value::as_str) {
        Some(t) => Some(Token::parse(t)?),
        None => None,
    };
    let outcome = if let Some(err) = obj.get("error") {
        ReplyOutcome::Error {
            code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: err.get("message").and_then(Value::as_str).unwrap_or("").to_owned(),
        }
    } else if let Some(result) = obj.get("result") {
        match result {
            Value::Array(a) => ReplyOutcome::Ok(a.clone()),
            other => ReplyOutcome::Ok(vec![other.clone()]),
        }
    } else {
        return Err(ProtocolError::InvalidResponse("reply has neither result nor error".into()));
    };
    Ok(Incoming::Reply { id, outcome, token })
}

/// Replaces token values in raw packet text so it can be logged.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("\"token\"") {
        let (head, tail) = rest.split_at(pos + "\"token\"".len());
        out.push_str(head);
        // Skip `:` and whitespace up to the opening quote of the value.
        let Some(q) = tail.find('"') else {
            rest = tail;
            break;
        };
        out.push_str(&tail[..=q]);
        let value = &tail[q + 1..];
        match value.find('"') {
            Some(end) => {
                out.push_str("***");
                rest = &value[end..];
            }
            None => {
                out.push_str("***");
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
