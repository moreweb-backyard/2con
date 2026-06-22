use url::Url;

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
}

impl ProxyConfig {
    pub fn parse(link: &str) -> Option<Self> {
        if link.starts_with("vless://") {
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
            
            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "host" => hostname = value.to_string(),
                    "path" => path = value.to_string(),
                    "security" => tls = value.to_string(),
                    "sni" => sni = value.to_string(),
                    _ => {}
                }
            }
            
            Some(ProxyConfig {
                protocol: "vless".to_string(),
                addresses,
                port,
                uuid,
                hostname,
                path,
                tls,
                sni,
            })
        } else {
            None
        }
    }
}
