//! Yeelight UDP control protocol (port 55444) as used by RGBIC devices.
//!
//! Layers, bottom to top:
//! - [`transport`]: datagram I/O behind a trait so tests can use a mock.
//! - [`packets`]: JSON command encoding, response parsing, LED packing.
//! - [`session`]: `udp_sess_new` token handshake, keep-alive, request/response matching.
//! - [`device`]: typed operations for the Chameleon2 strip.
//!
//! See `docs/protocol.md` for what is confirmed and what is still an assumption.

pub mod device;
pub mod packets;
pub mod session;
pub mod transport;

use std::fmt;
use std::io;

/// UDP port of the Yeelight RGBIC control protocol.
pub const CONTROL_PORT: u16 = 55444;

/// 24-bit RGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Value used by `set_rgb`: `r * 65536 + g * 256 + b`.
    pub const fn to_u24(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Parses `#RRGGBB` or `RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    /// No matching reply arrived before the deadline.
    Timeout,
    /// The operation needs an established session.
    NoSession,
    /// The device answered with an `error` object.
    Device {
        code: i64,
        message: String,
    },
    /// A datagram could not be interpreted as an expected reply.
    InvalidResponse(String),
    /// The caller passed data the protocol cannot carry.
    InvalidInput(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Timeout => f.write_str("device did not reply in time"),
            Self::NoSession => f.write_str("no active UDP session"),
            Self::Device { code, message } => write!(f, "device error {code}: {message}"),
            Self::InvalidResponse(m) => write!(f, "invalid response: {m}"),
            Self::InvalidInput(m) => write!(f, "invalid input: {m}"),
        }
    }
}

impl std::error::Error for ProtocolError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

impl serde::Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}")))
    }
}
