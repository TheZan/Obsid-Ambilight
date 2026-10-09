//! Finding Yeelight devices on the local network.
//!
//! A search request is sent through every usable IPv4 adapter to the SSDP
//! multicast group and, optionally, to the adapter's subnet broadcast address;
//! devices answer with a unicast datagram. Sending per adapter matters because
//! VPN/TUN adapters often hold the default route. The request is repeated a few
//! times because UDP may drop packets and because the Windows firewall only lets
//! unicast replies to a multicast/broadcast request through for a few seconds
//! after each send.

pub mod interfaces;
pub mod yeelight;

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use tracing::{debug, trace};

pub use interfaces::LocalInterface;
pub use yeelight::{DiscoveredDevice, SEARCH_REQUEST, parse_response};

pub const DISCOVERY_PORT: u16 = 1982;
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);

#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Total time to collect answers.
    pub timeout: Duration,
    /// Interval between repeated search requests.
    pub resend_every: Duration,
    /// Also send to each subnet's broadcast address (helps where multicast is filtered).
    pub broadcast: bool,
}

impl Default for DiscoveryOptions {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(3), resend_every: Duration::from_secs(1), broadcast: true }
    }
}

/// Discovers all Yeelight devices that answer within the timeout, deduplicated
/// by device id. Filter with [`DiscoveredDevice::is_chameleon2`].
pub fn discover(opts: &DiscoveryOptions) -> io::Result<Vec<DiscoveredDevice>> {
    let adapters = interfaces::local_interfaces().unwrap_or_else(|e| {
        debug!(error = %e, "adapter enumeration failed; using the default route only");
        Vec::new()
    });
    if adapters.is_empty() {
        return discover_via(None, opts);
    }

    // One socket and thread per adapter; the number of adapters is small.
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = adapters.iter().map(|a| s.spawn(move || (a, discover_via(Some(a), opts)))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut merged: HashMap<String, DiscoveredDevice> = HashMap::new();
    let mut last_err = None;
    let mut any_ok = false;
    for (adapter, result) in results {
        match result {
            Ok(list) => {
                any_ok = true;
                for d in list {
                    debug!(id = %d.id, ip = %d.ip, via = %adapter.name, "found device");
                    merged.insert(d.id.clone(), d);
                }
            }
            Err(e) => {
                debug!(adapter = %adapter.name, error = %e, "discovery failed on adapter");
                last_err = Some(e);
            }
        }
    }
    if let (false, Some(e)) = (any_ok, last_err) {
        return Err(e);
    }
    let mut list: Vec<_> = merged.into_values().collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

/// Searches through one adapter, or through the default route when `None`.
fn discover_via(adapter: Option<&LocalInterface>, opts: &DiscoveryOptions) -> io::Result<Vec<DiscoveredDevice>> {
    let local = adapter.map_or(Ipv4Addr::UNSPECIFIED, |a| a.ip);
    let socket = UdpSocket::bind(SocketAddrV4::new(local, 0))?;
    socket.set_broadcast(opts.broadcast)?;
    socket.set_multicast_loop_v4(false)?;
    if let Some(a) = adapter {
        interfaces::set_multicast_interface(&socket, a.ip)?;
    }
    let mut targets = vec![SocketAddrV4::new(MULTICAST_GROUP, DISCOVERY_PORT)];
    if opts.broadcast {
        let bcast = adapter.and_then(LocalInterface::broadcast).unwrap_or(Ipv4Addr::BROADCAST);
        targets.push(SocketAddrV4::new(bcast, DISCOVERY_PORT));
    }
    collect(&socket, &targets, opts)
}

/// Sends a unicast search request to one address. Lets a saved IP be checked
/// *and* matched against the stored device id.
pub fn probe(ip: Ipv4Addr, timeout: Duration) -> io::Result<Option<DiscoveredDevice>> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))?;
    let opts = DiscoveryOptions { timeout, resend_every: timeout / 2, broadcast: false };
    let found = collect(&socket, &[SocketAddrV4::new(ip, DISCOVERY_PORT)], &opts)?;
    Ok(found.into_iter().find(|d| d.ip == ip))
}

fn collect(socket: &UdpSocket, targets: &[SocketAddrV4], opts: &DiscoveryOptions) -> io::Result<Vec<DiscoveredDevice>> {
    let start = Instant::now();
    let deadline = start + opts.timeout;
    let resend_every = opts.resend_every.max(Duration::from_millis(100));
    let mut next_send = start;
    let mut found: HashMap<String, DiscoveredDevice> = HashMap::new();
    let mut buf = [0u8; 2048];

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if now >= next_send {
            for t in targets {
                // A failing target (e.g. broadcast blocked) must not stop the others.
                if let Err(e) = socket.send_to(SEARCH_REQUEST.as_bytes(), t) {
                    debug!(target = %t, error = %e, "search send failed");
                }
            }
            next_send = now + resend_every;
        }
        let wait = deadline.min(next_send).saturating_duration_since(now);
        socket.set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        match socket.recv_from(&mut buf) {
            Ok((len, SocketAddr::V4(src))) => {
                let text = String::from_utf8_lossy(&buf[..len]);
                trace!(from = %src, %text, "discovery reply");
                if let Some(dev) = parse_response(&text, *src.ip()) {
                    found.insert(dev.id.clone(), dev);
                }
            }
            Ok((_, SocketAddr::V6(_))) => {}
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::ConnectionReset => {}
                _ => return Err(e),
            },
        }
    }

    let mut list: Vec<_> = found.into_values().collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}
