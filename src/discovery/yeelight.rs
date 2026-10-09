//! Parsing of Yeelight SSDP-like discovery messages (UDP port 1982).

use std::net::Ipv4Addr;

use crate::protocol::device::MODEL as CHAMELEON2;

/// Search request; the `ST: wifi_bulb` target is what Yeelight devices answer to.
pub const SEARCH_REQUEST: &str =
    "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1982\r\nMAN: \"ssdp:discover\"\r\nST: wifi_bulb\r\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    /// Stable device id (e.g. `0x000000000649f51e`). Use this, not the IP, as identity.
    pub id: String,
    pub model: String,
    /// Address the response came from.
    pub ip: Ipv4Addr,
    /// Raw `Location` header (`yeelight://ip:55443`, the classic TCP port).
    pub location: String,
    pub name: String,
    pub fw_ver: Option<String>,
    pub support: Vec<String>,
    pub power: Option<String>,
    /// Light state advertised in the response (`bright`, `color_mode`, `rgb`, ...),
    /// in arrival order. Used for diagnostics only.
    pub state: Vec<(String, String)>,
}

impl DiscoveredDevice {
    pub fn is_chameleon2(&self) -> bool {
        self.model.eq_ignore_ascii_case(CHAMELEON2)
    }

    pub fn supports(&self, method: &str) -> bool {
        self.support.iter().any(|m| m == method)
    }

    /// The methods this application relies on for per-LED control.
    pub fn supports_rgbic_udp(&self) -> bool {
        self.supports("udp_sess_new") && self.supports("update_leds")
    }

    /// Name for display: the user-assigned name, falling back to the model.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { &self.model } else { &self.name }
    }
}

/// Parses a search response (`HTTP/1.1 200 OK`) or an advertisement (`NOTIFY`).
/// Returns `None` for anything that is not a well-formed Yeelight message.
pub fn parse_response(text: &str, source: Ipv4Addr) -> Option<DiscoveredDevice> {
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let start = lines.next()?.trim();
    if !(start.starts_with("HTTP/1.1 200") || start.starts_with("NOTIFY")) {
        return None;
    }

    let mut id = None;
    let mut model = None;
    let mut location = None;
    let mut name = String::new();
    let mut fw_ver = None;
    let mut support = Vec::new();
    let mut power = None;
    let mut state = Vec::new();

    for line in lines {
        // Split on the first colon only: Location values contain colons too.
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "id" => id = Some(value.to_owned()),
            "model" => model = Some(value.to_owned()),
            "location" => location = Some(value.to_owned()),
            "name" => name = value.to_owned(),
            "fw_ver" => fw_ver = Some(value.to_owned()),
            "support" => support = value.split_ascii_whitespace().map(str::to_owned).collect(),
            "power" => power = Some(value.to_owned()),
            k @ ("bright" | "color_mode" | "ct" | "rgb" | "hue" | "sat") => {
                state.push((k.to_owned(), value.to_owned()))
            }
            _ => {}
        }
    }

    let location = location.filter(|l| l.to_ascii_lowercase().starts_with("yeelight://"))?;
    let id = id.filter(|s| !s.is_empty())?;
    let model = model.filter(|s| !s.is_empty())?;
    Some(DiscoveredDevice { id, model, ip: source, location, name, fw_ver, support, power, state })
}
