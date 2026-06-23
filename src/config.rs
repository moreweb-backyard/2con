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
}

impl ProxyConfig {
    pub fn parse(link: &str) -> Option<Self> {
        if link.starts_with("vless://") || link.starts_with("trojan://") {
            let protocol = if link.starts_with("vless://") { "vless" } else { "trojan" };
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
            
            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "host" => hostname = value.to_string(),
                    "path" => path = value.to_string(),
                    "security" => tls = value.to_string(),
                    "sni" => sni = value.to_string(),
                    "type" => transport = value.to_string(),
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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            socks_port: 10808,
            mux_enabled: false,
            log_level: "warning".to_string(),
            docking: "None".to_string(),
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
