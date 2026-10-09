//! Enumeration of local IPv4 interfaces for per-adapter discovery.
//!
//! VPN/TUN adapters often own the default route, so a multicast request sent
//! from an unbound socket can leave through the tunnel instead of the LAN. We
//! therefore send the search through every usable adapter explicitly.

use std::io;
use std::net::{Ipv4Addr, UdpSocket};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInterface {
    pub name: String,
    pub ip: Ipv4Addr,
    pub prefix_len: u8,
}

impl LocalInterface {
    /// Subnet-directed broadcast address, if the subnet has one.
    pub fn broadcast(&self) -> Option<Ipv4Addr> {
        if self.prefix_len == 0 || self.prefix_len >= 31 {
            return None;
        }
        let host_mask = u32::MAX >> self.prefix_len;
        Some(Ipv4Addr::from(u32::from(self.ip) | host_mask))
    }
}

/// Addresses that can reach LAN devices: not loopback, not link-local
/// (169.254/16 means "no DHCP lease"), not unspecified.
pub fn is_usable(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast())
}

/// Makes `socket` send multicast through the interface owning `ip`.
#[cfg(windows)]
pub fn set_multicast_interface(socket: &UdpSocket, ip: Ipv4Addr) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{IP_MULTICAST_IF, IPPROTO_IP, SOCKET_ERROR, setsockopt};

    // IP_MULTICAST_IF takes an IN_ADDR in network byte order.
    let addr = u32::from_ne_bytes(ip.octets());
    // SAFETY: the socket handle is valid for the lifetime of `socket`, and the
    // option value points to a live u32 of the advertised length.
    let rc = unsafe {
        setsockopt(
            socket.as_raw_socket() as _,
            IPPROTO_IP,
            IP_MULTICAST_IF,
            (&raw const addr).cast(),
            size_of::<u32>() as i32,
        )
    };
    if rc == SOCKET_ERROR { Err(io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(not(windows))]
pub fn set_multicast_interface(_socket: &UdpSocket, _ip: Ipv4Addr) -> io::Result<()> {
    Ok(())
}

/// Operational IPv4 interfaces with usable addresses.
#[cfg(windows)]
pub fn local_interfaces() -> io::Result<Vec<LocalInterface>> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
        IF_TYPE_SOFTWARE_LOOPBACK, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    // u64 storage keeps the buffer aligned for IP_ADAPTER_ADDRESSES_LH.
    let mut buf: Vec<u64> = vec![0; 2048];
    let mut rc = ERROR_BUFFER_OVERFLOW;
    for _ in 0..3 {
        let mut size = (buf.len() * size_of::<u64>()) as u32;
        // SAFETY: `buf` is writable for `size` bytes and suitably aligned.
        rc = unsafe {
            GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null(), buf.as_mut_ptr().cast(), &mut size)
        };
        if rc != ERROR_BUFFER_OVERFLOW {
            break;
        }
        buf = vec![0; (size as usize).div_ceil(size_of::<u64>())];
    }
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }

    let mut out = Vec::new();
    let mut adapter = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    // SAFETY: on success the buffer holds a linked list of adapters (and their
    // unicast address lists) that stays valid while `buf` is alive.
    unsafe {
        while let Some(a) = adapter.as_ref() {
            adapter = a.Next;
            if a.OperStatus != IfOperStatusUp || a.IfType == IF_TYPE_SOFTWARE_LOOPBACK {
                continue;
            }
            let name = wide_to_string(a.FriendlyName);
            let mut ua = a.FirstUnicastAddress;
            while let Some(u) = ua.as_ref() {
                ua = u.Next;
                let sa = u.Address.lpSockaddr;
                if sa.is_null() || (*sa).sa_family != AF_INET {
                    continue;
                }
                let sin = &*(sa as *const SOCKADDR_IN);
                let ip = Ipv4Addr::from(sin.sin_addr.S_un.S_addr.to_ne_bytes());
                if is_usable(ip) {
                    out.push(LocalInterface { name: name.clone(), ip, prefix_len: u.OnLinkPrefixLength });
                }
            }
        }
    }
    Ok(out)
}

#[cfg(windows)]
unsafe fn wide_to_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: caller passes a NUL-terminated UTF-16 string.
    unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

#[cfg(not(windows))]
pub fn local_interfaces() -> io::Result<Vec<LocalInterface>> {
    Ok(Vec::new())
}
