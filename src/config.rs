use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub raw_link: String,
    #[serde(default = "default_sub_group")]
    pub sub_group: String,
}

fn default_sub_group() -> String {
    "Personal".to_string()
}

pub fn robust_base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let mut cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    cleaned = cleaned.replace("%3D", "=").replace("%3d", "=");
    cleaned = cleaned.replace('-', "+").replace('_', "/");
    while cleaned.ends_with('=') {
        cleaned.pop();
    }
    while cleaned.len() % 4 != 0 {
        cleaned.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|e| e.to_string())
}

fn parse_reserved(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() {
        return None;
    }
    if s.contains(',') {
        let bytes: Result<Vec<u8>, _> =
            s.split(',').map(|part| part.trim().parse::<u8>()).collect();
        if let Ok(b) = bytes {
            return Some(b);
        }
    }
    if let Ok(num) = s.trim().parse::<u8>() {
        return Some(vec![num]);
    }
    if let Ok(bytes) = robust_base64_decode(s) {
        return Some(bytes);
    }
    None
}

#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub protocol: String,
    pub addresses: Vec<String>,
    pub port: u16,
    pub uuid: String,
    pub hostname: String,
    pub path: String,
    pub tls: String,
    pub sni: String,
    pub transport: String,
    pub pbk: String,
    pub sid: String,
    pub fp: String,
    pub flow: String,
    pub public_key: String,
    pub local_address: Vec<String>,
    pub mtu: Option<u32>,
    pub reserved: Option<Vec<u8>>,
}

impl ProxyConfig {
    pub fn parse(link: &str) -> Option<Self> {
        if link.starts_with("vmess://") {
            let b64 = &link[8..];
            if let Ok(decoded) = robust_base64_decode(b64) {
                if let Ok(json_str) = String::from_utf8(decoded) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        return Some(ProxyConfig {
                            protocol: "vmess".to_string(),
                            addresses: vec![val["add"].as_str().unwrap_or("").to_string()],
                            port: val["port"]
                                .as_u64()
                                .or_else(|| val["port"].as_str().and_then(|s| s.parse().ok()))
                                .unwrap_or(443) as u16,
                            uuid: val["id"].as_str().unwrap_or("").to_string(),
                            hostname: val["host"].as_str().unwrap_or("").to_string(),
                            path: val["path"].as_str().unwrap_or("").to_string(),
                            tls: val["tls"].as_str().unwrap_or("").to_string(),
                            sni: val["sni"]
                                .as_str()
                                .or(val["host"].as_str())
                                .unwrap_or("")
                                .to_string(),
                            transport: val["net"].as_str().unwrap_or("tcp").to_string(),
                            pbk: String::new(),
                            sid: String::new(),
                            fp: String::new(),
                            flow: String::new(),
                            public_key: String::new(),
                            local_address: Vec::new(),
                            mtu: None,
                            reserved: None,
                        });
                    }
                }
            }
            return None;
        } else if link.starts_with("ss://") {
            let mut host_part = String::new();
            let mut port_part = String::new();
            let mut method_pass = String::new();

            if let Some(at_idx) = link.find('@') {
                // ss://BASE64@host:port#name
                let b64 = &link[5..at_idx];
                if let Ok(decoded) = robust_base64_decode(b64) {
                    if let Ok(decoded_str) = String::from_utf8(decoded) {
                        method_pass = decoded_str;
                    }
                }

                let remainder = &link[at_idx + 1..];
                let end_idx = remainder.find('#').unwrap_or(remainder.len());
                let host_port = &remainder[..end_idx];

                if let Some(colon) = host_port.rfind(':') {
                    host_part = host_port[..colon].to_string();
                    port_part = host_port[colon + 1..].to_string();
                }
            } else {
                // ss://BASE64#name
                let end_idx = link.find('#').unwrap_or(link.len());
                let b64 = &link[5..end_idx];
                if let Ok(decoded) = robust_base64_decode(b64) {
                    if let Ok(decoded_str) = String::from_utf8(decoded) {
                        if let Some(at_idx) = decoded_str.rfind('@') {
                            method_pass = decoded_str[..at_idx].to_string();
                            let host_port = &decoded_str[at_idx + 1..];
                            if let Some(colon) = host_port.rfind(':') {
                                host_part = host_port[..colon].to_string();
                                port_part = host_port[colon + 1..].to_string();
                            }
                        }
                    }
                }
            }

            if !host_part.is_empty() && !port_part.is_empty() {
                return Some(ProxyConfig {
                    protocol: "shadowsocks".to_string(),
                    addresses: vec![host_part],
                    port: port_part.parse().unwrap_or(443),
                    uuid: method_pass, // we use uuid to store method:password for simplicity
                    hostname: "".to_string(),
                    path: "".to_string(),
                    tls: "".to_string(),
                    sni: "".to_string(),
                    transport: "tcp".to_string(),
                    pbk: String::new(),
                    sid: String::new(),
                    fp: String::new(),
                    flow: String::new(),
                    public_key: String::new(),
                    local_address: Vec::new(),
                    mtu: None,
                    reserved: None,
                });
            }
            return None;
        } else if link.starts_with("vless://") || link.starts_with("trojan://") {
            let protocol = if link.starts_with("vless://") {
                "vless"
            } else {
                "trojan"
            };
            let url = Url::parse(link).ok()?;
            let uuid = url.username().to_string();

            // Host could be multiple addresses separated by comma
            let host_str = url.host_str()?;
            let addresses: Vec<String> = host_str.split(',').map(|s| s.to_string()).collect();

            let port = url.port().unwrap_or(443);

            let mut hostname = String::new();
            let mut path = String::new();
            let mut tls = String::new();
            let mut sni = String::new();
            let mut transport = String::new();
            let mut pbk = String::new();
            let mut sid = String::new();
            let mut fp = String::new();
            let mut flow = String::new();

            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "host" => hostname = value.to_string(),
                    "path" => path = value.to_string(),
                    "security" => tls = value.to_string(),
                    "sni" => sni = value.to_string(),
                    "type" => transport = value.to_string(),
                    "pbk" => pbk = value.to_string(),
                    "sid" => sid = value.to_string(),
                    "fp" => fp = value.to_string(),
                    "flow" => flow = value.to_string(),
                    _ => {}
                }
            }

            if transport.is_empty() {
                transport = "tcp".to_string();
            }

            Some(ProxyConfig {
                protocol: protocol.to_string(),
                addresses,
                port,
                uuid,
                hostname,
                path,
                tls,
                sni,
                transport,
                pbk,
                sid,
                fp,
                flow,
                public_key: String::new(),
                local_address: Vec::new(),
                mtu: None,
                reserved: None,
            })
        } else if link.starts_with("wg://") {
            let url = Url::parse(link).ok()?;
            let uuid = url.username().to_string();

            let host_str = url.host_str().unwrap_or("");
            let addresses = if host_str.is_empty() {
                Vec::new()
            } else {
                vec![host_str.to_string()]
            };
            let port = url.port().unwrap_or(51820);

            let mut public_key = String::new();
            let mut local_address = Vec::new();
            let mut mtu = None;
            let mut reserved = None;

            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "publickey" | "public_key" => public_key = value.to_string(),
                    "address" | "ip" => {
                        local_address = value.split(',').map(|s| s.trim().to_string()).collect();
                    }
                    "mtu" => mtu = value.parse().ok(),
                    "reserved" => reserved = parse_reserved(&value),
                    _ => {}
                }
            }

            Some(ProxyConfig {
                protocol: "wireguard".to_string(),
                addresses,
                port,
                uuid,
                hostname: String::new(),
                path: String::new(),
                tls: String::new(),
                sni: String::new(),
                transport: String::new(),
                pbk: String::new(),
                sid: String::new(),
                fp: String::new(),
                flow: String::new(),
                public_key,
                local_address,
                mtu,
                reserved,
            })
        } else {
            None
        }
    }
}

pub fn load_profiles() -> Vec<Profile> {
    let path = "configs.json";
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(profiles) = serde_json::from_str(&content) {
            return profiles;
        }
    }
    Vec::new()
}

pub fn save_profiles(profiles: &[Profile]) {
    let path = "configs.json";
    if let Ok(content) = serde_json::to_string_pretty(profiles) {
        let _ = std::fs::write(path, content);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub socks_port: u16,
    pub mux_enabled: bool,
    pub log_level: String,
    pub docking: String, // "None", "Top Left", "Top Right", "Bottom Left", "Bottom Right"

    // Advanced Core
    #[serde(default = "default_true")]
    pub enable_udp: bool,
    #[serde(default = "default_false")]
    pub enable_sniffing: bool,
    #[serde(default = "default_sniffing_types")]
    pub sniffing_types: Vec<String>,
    #[serde(default = "default_false")]
    pub allow_lan: bool,
    #[serde(default = "default_false")]
    pub enable_fragment: bool,

    // Routing
    #[serde(default = "default_bypass_list")]
    pub bypass_list: String,

    // General
    #[serde(default = "default_false")]
    pub start_on_boot: bool,
    #[serde(default)]
    pub auto_update_geo: u32,

    // DNS
    #[serde(default = "default_domestic_dns")]
    pub domestic_dns: String,
    #[serde(default = "default_remote_dns")]
    pub remote_dns: String,
    #[serde(default = "default_bootstrap_dns")]
    pub bootstrap_dns: String,
    #[serde(default = "default_false")]
    pub enable_fakeip: bool,
    #[serde(default = "default_true")]
    pub block_svcb: bool,
    #[serde(default = "default_true")]
    pub add_common_dns: bool,
    #[serde(default)]
    pub dns_hosts: String,
    #[serde(default)]
    pub custom_dns_json: String,
}

fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}
fn default_domestic_dns() -> String {
    "8.8.8.8".to_string()
}
fn default_remote_dns() -> String {
    "tcp://8.8.8.8".to_string()
}
fn default_bootstrap_dns() -> String {
    "8.8.8.8".to_string()
}
fn default_sniffing_types() -> Vec<String> {
    vec!["http".to_string(), "tls".to_string()]
}
fn default_bypass_list() -> String {
    "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            socks_port: 10808,
            mux_enabled: false,
            log_level: "warning".to_string(),
            docking: "None".to_string(),
            enable_udp: true,
            enable_sniffing: false,
            sniffing_types: default_sniffing_types(),
            allow_lan: false,
            enable_fragment: false,
            bypass_list: default_bypass_list(),
            start_on_boot: false,
            auto_update_geo: 0,

            domestic_dns: default_domestic_dns(),
            remote_dns: default_remote_dns(),
            bootstrap_dns: default_bootstrap_dns(),
            enable_fakeip: false,
            block_svcb: true,
            add_common_dns: true,
            dns_hosts: String::new(),
            custom_dns_json: String::new(),
        }
    }
}

pub fn load_settings() -> AppSettings {
    let path = "settings.json";
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(settings) = serde_json::from_str(&content) {
            return settings;
        }
    }
    AppSettings::default()
}

pub fn save_settings(settings: &AppSettings) {
    let path = "settings.json";
    if let Ok(content) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, content);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub url: String,
    pub last_updated: String,
}

pub fn load_subscriptions() -> Vec<Subscription> {
    let path = "subscriptions.json";
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(subs) = serde_json::from_str(&content) {
            return subs;
        }
    }
    Vec::new()
}

pub fn save_subscriptions(subs: &[Subscription]) {
    let path = "subscriptions.json";
    if let Ok(content) = serde_json::to_string_pretty(subs) {
        let _ = std::fs::write(path, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_robust_decode() {
        let input = "eyJhZGQiOiIxMi4zNC41Ni43OCIsInBvcnQiOjQ0MywiaWQiOiJ1dWlkMSIsImhvc3QiOiJ2bWVzcy5leGFtcGxlLmNvbSIsInBhdGgiOiIvIiwidGxzIjoidGxzIiwic25pIjoidm1lc3MuZXhwYW1wbGUuY29tIiwibmV0IjoidGNwIn0==";
        let res = robust_base64_decode(input);
        assert!(res.is_ok());
    }
}
