use serde_json::{json, Value};
use crate::config::AppSettings;
use std::process::Stdio;
use tokio::process::{Child, Command};
use std::path::Path;

pub struct ProxyRunner {
    child: Option<Child>,
}

impl ProxyRunner {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub async fn start(&mut self, config: Value) -> Result<(), String> {
        self.stop().await;

        // Write config to temp file
        let config_path = "xray_config.json";
        std::fs::write(config_path, serde_json::to_string_pretty(&config).unwrap())
            .map_err(|e| format!("Failed to write config: {}", e))?;

        let xray_bin = if cfg!(windows) { "xray.exe" } else { "./xray" };
        
        if !Path::new(xray_bin).exists() {
            return Err("Xray binary not found. Please download it.".to_string());
        }

        let child = Command::new(xray_bin)
            .arg("-c")
            .arg(config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start xray: {}", e))?;

        self.child = Some(child);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
    }
}

pub fn generate_xray_config(protocol: &str, address: &str, port: u16, uuid: &str, sni: &str, host: &str, path: &str, app_settings: &AppSettings) -> Value {
    let settings = if protocol == "trojan" {
        json!({
            "servers": [
                {
                    "address": address,
                    "port": port,
                    "password": uuid
                }
            ]
        })
    } else {
        json!({
            "vnext": [
                {
                    "address": address,
                    "port": port,
                    "users": [
                        {
                            "id": uuid,
                            "encryption": "none",
                            "flow": ""
                        }
                    ]
                }
            ]
        })
    };

    let stream_settings = if path.is_empty() {
        json!({
            "network": "tcp",
            "security": "tls",
            "tlsSettings": {
                "serverName": sni,
                "allowInsecure": false
            }
        })
    } else {
        json!({
            "network": "ws",
            "security": "tls",
            "tlsSettings": {
                "serverName": sni,
                "allowInsecure": false
            },
            "wsSettings": {
                "path": path,
                "headers": {
                    "Host": host
                }
            }
        })
    };

    let mut outbound = json!({
        "protocol": protocol,
        "settings": settings,
        "streamSettings": stream_settings
    });

    if app_settings.mux_enabled {
        outbound.as_object_mut().unwrap().insert("mux".to_string(), json!({
            "enabled": true,
            "concurrency": 8
        }));
    }

    json!({
        "log": {
            "loglevel": app_settings.log_level
        },
        "inbounds": [
            {
                "port": app_settings.socks_port,
                "listen": "127.0.0.1",
                "protocol": "socks",
                "settings": {
                    "udp": true
                }
            },
            {
                "port": app_settings.socks_port + 1,
                "listen": "127.0.0.1",
                "protocol": "http"
            }
        ],
        "outbounds": [
            outbound
        ]
    })
}
