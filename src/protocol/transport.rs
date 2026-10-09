//! Datagram transport used by [`Session`](super::session::Session).

use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

/// Largest datagram we accept from the device.
pub const MAX_DATAGRAM: usize = 2048;

pub trait Transport {
    fn send(&mut self, data: &[u8]) -> io::Result<()>;

    /// Waits up to `timeout` for one datagram. `Ok(None)` means nothing arrived.
    fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<usize>>;
}

/// UDP socket connected to a single device.
pub struct UdpTransport {
    socket: UdpSocket,
    peer: SocketAddr,
}

impl UdpTransport {
    pub fn connect(ip: Ipv4Addr, port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
        let peer = SocketAddr::V4(SocketAddrV4::new(ip, port));
        socket.connect(peer)?;
        Ok(Self { socket, peer })
    }

    pub fn peer(&self) -> SocketAddr {
        self.peer
    }
}

impl Transport for UdpTransport {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        let sent = self.socket.send(data)?;
        if sent != data.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "datagram truncated on send"));
        }
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        // A zero timeout means "block forever" for std sockets.
        self.socket.set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;
        match self.socket.recv(buf) {
            Ok(n) => Ok(Some(n)),
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Ok(None),
                // Windows reports an ICMP "port unreachable" from an earlier send as
                // WSAECONNRESET on a connected UDP socket. It is not fatal: treat it as
                // "no reply" and let the caller's timeout/retry logic decide.
                io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionRefused => Ok(None),
                _ => Err(e),
            },
        }
    }
}

/// Maps one sent datagram to the datagrams the fake device sends back.
pub type Responder = Box<dyn FnMut(&str) -> Vec<String> + Send>;

/// Scripted in-memory transport for tests and offline experiments.
///
/// Every sent datagram is recorded and passed to the responder, whose returned
/// datagrams are queued for subsequent `recv` calls. An empty queue behaves like
/// an immediate timeout, so tests never sleep.
pub struct MockTransport {
    responder: Responder,
    inbox: VecDeque<Vec<u8>>,
    pub sent: Vec<String>,
}

impl MockTransport {
    pub fn new(responder: impl FnMut(&str) -> Vec<String> + Send + 'static) -> Self {
        Self { responder: Box::new(responder), inbox: VecDeque::new(), sent: Vec::new() }
    }

    /// A transport that never answers.
    pub fn silent() -> Self {
        Self::new(|_| Vec::new())
    }

    /// Queues an unsolicited datagram.
    pub fn push_incoming(&mut self, datagram: impl Into<String>) {
        self.inbox.push_back(datagram.into().into_bytes());
    }
}

impl Transport for MockTransport {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        let text = String::from_utf8_lossy(data).into_owned();
        for reply in (self.responder)(&text) {
            self.inbox.push_back(reply.into_bytes());
        }
        self.sent.push(text);
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8], _timeout: Duration) -> io::Result<Option<usize>> {
        match self.inbox.pop_front() {
            Some(d) => {
                let n = d.len().min(buf.len());
                buf[..n].copy_from_slice(&d[..n]);
                Ok(Some(n))
            }
            None => Ok(None),
        }
    }
}
