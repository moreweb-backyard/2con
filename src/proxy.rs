use crate::config::AppSettings;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

use reqwest::Client;
use std::io::Cursor;

pub async fn download_xray_core() -> Result<(), String> {
    let client = Client::new();
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64-v8a"
    } else {
        "64"
    };

    let tag_url = "https://api.github.com/repos/XTLS/Xray-core/releases/latest";
    let release_info: serde_json::Value = client
        .get(tag_url)
        .header("User-Agent", "2con-client")
        .send()
        .await
        .map_err(|e| format!("Failed to get Xray releases: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    let tag_name = release_info["tag_name"]
        .as_str()
        .ok_or("No tag_name found")?;

    let zip_name = format!("Xray-{}-{}.zip", os, arch);
    let download_url = format!(
        "https://github.com/XTLS/Xray-core/releases/download/{}/{}",
        tag_name, zip_name
    );

    let response = client
        .get(&download_url)
        .header("User-Agent", "2con-client")
        .send()
        .await
        .map_err(|e| format!("Failed to download Xray zip: {}", e))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read zip bytes: {}", e))?;
    let cursor = Cursor::new(bytes);

    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip: {}", e))?;

    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("Failed to read zip file {}: {}", i, e))?;
        let outpath = match file.enclosed_name() {
            Some(path) => path.to_owned(),
            None => continue,
        };

        let filename = outpath.file_name().unwrap_or_default().to_string_lossy();
        if filename.starts_with("xray") || filename == "geoip.dat" || filename == "geosite.dat" {
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create {}: {}", filename, e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract {}: {}", filename, e))?;

            #[cfg(unix)]
            if filename.starts_with("xray") {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = std::fs::metadata(&outpath) {
                    let mut perms = metadata.permissions();
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(&outpath, perms);
                }
            }
        }
    }

    Ok(())
}

pub async fn download_routing_rules() -> Result<(), String> {
    let client = Client::new();

    let geoip_url =
        "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat";
    let geosite_url =
        "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat";

    let geoip_bytes = client
        .get(geoip_url)
        .header("User-Agent", "2con-client")
        .send()
        .await
        .map_err(|e| format!("Failed to download geoip.dat: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read geoip.dat bytes: {}", e))?;

    std::fs::write("geoip.dat", &geoip_bytes)
        .map_err(|e| format!("Failed to save geoip.dat: {}", e))?;

    let geosite_bytes = client
        .get(geosite_url)
        .header("User-Agent", "2con-client")
        .send()
        .await
        .map_err(|e| format!("Failed to download geosite.dat: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read geosite.dat bytes: {}", e))?;

    std::fs::write("geosite.dat", &geosite_bytes)
        .map_err(|e| format!("Failed to save geosite.dat: {}", e))?;

    Ok(())
}

pub struct ProxyRunner {
    child: Option<Child>,
}

impl ProxyRunner {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub async fn start(
        &mut self,
        config: Value,
        log_sender: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<(), String> {
        self.stop().await;

        let config_path = "xray_config.json";
        std::fs::write(config_path, serde_json::to_string_pretty(&config).unwrap())
            .map_err(|e| format!("Failed to write config: {}", e))?;

        let xray_bin = if cfg!(windows) { "xray.exe" } else { "./xray" };

        if !Path::new(xray_bin).exists() {
            download_xray_core().await?;
        }

        let mut child = Command::new(xray_bin)
            .arg("-c")
            .arg(config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to start xray: {}", e))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let tx1 = log_sender.clone();
        if let Some(out) = stdout {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx1.send(format!("[INFO] {}\n", line));
                }
            });
        }

        let tx2 = log_sender;
        if let Some(err) = stderr {
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx2.send(format!("[ERROR] {}\n", line));
                }
            });
        }

        self.child = Some(child);

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
    }
}

pub fn generate_xray_config(
    parsed_cfg: &crate::config::ProxyConfig,
    address: &str,
    app_settings: &AppSettings,
) -> Value {
    let settings = if parsed_cfg.protocol == "wireguard" {
        let mut settings_obj = json!({
            "secretKey": parsed_cfg.uuid,
            "address": parsed_cfg.local_address,
            "peers": [
                {
                    "endpoint": format!("{}:{}", address, parsed_cfg.port),
                    "publicKey": parsed_cfg.public_key
                }
            ]
        });
        if let Some(mtu) = parsed_cfg.mtu {
            settings_obj
                .as_object_mut()
                .unwrap()
                .insert("mtu".to_string(), json!(mtu));
        }
        if let Some(ref reserved) = parsed_cfg.reserved {
            settings_obj
                .as_object_mut()
                .unwrap()
                .insert("reserved".to_string(), json!(reserved));
        }
        settings_obj
    } else if parsed_cfg.protocol == "trojan" {
        json!({
            "servers": [
                {
                    "address": address,
                    "port": parsed_cfg.port,
                    "password": parsed_cfg.uuid
                }
            ]
        })
    } else if parsed_cfg.protocol == "shadowsocks" {
        // method:password is stored in uuid
        let mut method = "aes-256-gcm";
        let mut pass = &parsed_cfg.uuid as &str;
        if let Some(colon) = parsed_cfg.uuid.find(':') {
            method = &parsed_cfg.uuid[..colon];
            pass = &parsed_cfg.uuid[colon + 1..];
        }
        json!({
            "servers": [
                {
                    "address": address,
                    "port": parsed_cfg.port,
                    "method": method,
                    "password": pass
                }
            ]
        })
    } else if parsed_cfg.protocol == "vmess" {
        json!({
            "vnext": [
                {
                    "address": address,
                    "port": parsed_cfg.port,
                    "users": [
                        {
                            "id": parsed_cfg.uuid,
                            "alterId": 0,
                            "security": "auto"
                        }
                    ]
                }
            ]
        })
    } else {
        let flow_val = if parsed_cfg.protocol == "vless" {
            &parsed_cfg.flow
        } else {
            ""
        };
        json!({
            "vnext": [
                {
                    "address": address,
                    "port": parsed_cfg.port,
                    "users": [
                        {
                            "id": parsed_cfg.uuid,
                            "encryption": "none",
                            "flow": flow_val
                        }
                    ]
                }
            ]
        })
    };

    let network = if !parsed_cfg.path.is_empty() {
        "ws"
    } else if parsed_cfg.transport.is_empty() {
        "tcp"
    } else {
        &parsed_cfg.transport
    };

    let mut stream_settings = json!({
        "network": network,
    });

    if parsed_cfg.tls == "reality" {
        stream_settings
            .as_object_mut()
            .unwrap()
            .insert("security".to_string(), json!("reality"));
        stream_settings.as_object_mut().unwrap().insert(
            "realitySettings".to_string(),
            json!({
                "serverName": parsed_cfg.sni,
                "publicKey": parsed_cfg.pbk,
                "shortId": parsed_cfg.sid,
                "fingerprint": parsed_cfg.fp,
                "show": false
            }),
        );
    } else if parsed_cfg.tls == "tls" || parsed_cfg.tls.is_empty() {
        stream_settings
            .as_object_mut()
            .unwrap()
            .insert("security".to_string(), json!("tls"));
        stream_settings.as_object_mut().unwrap().insert(
            "tlsSettings".to_string(),
            json!({
                "serverName": parsed_cfg.sni,
                "allowInsecure": false
            }),
        );
    } else if parsed_cfg.tls != "none" {
        stream_settings
            .as_object_mut()
            .unwrap()
            .insert("security".to_string(), json!(parsed_cfg.tls));
    }

    if network == "ws" {
        stream_settings.as_object_mut().unwrap().insert(
            "wsSettings".to_string(),
            json!({
                "path": parsed_cfg.path,
                "headers": {
                    "Host": parsed_cfg.hostname
                }
            }),
        );
    }

    let mut outbound = if parsed_cfg.protocol == "wireguard" {
        json!({
            "protocol": "wireguard",
            "settings": settings
        })
    } else {
        json!({
            "protocol": parsed_cfg.protocol,
            "settings": settings,
            "streamSettings": stream_settings
        })
    };

    if app_settings.mux_enabled && !app_settings.enable_fragment {
        outbound.as_object_mut().unwrap().insert(
            "mux".to_string(),
            json!({
                "enabled": true,
                "concurrency": 8
            }),
        );
    }

    let listen_ip = if app_settings.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };

    let mut sniffing = json!({});
    if app_settings.enable_sniffing {
        sniffing = json!({
            "enabled": true,
            "destOverride": app_settings.sniffing_types
        });
    }

    let mut rules = vec![];

    // Convert bypass list to routing rules
    if !app_settings.bypass_list.is_empty() {
        let domains: Vec<String> = app_settings
            .bypass_list
            .split(';')
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                let s = s.trim().replace("*", "");
                if s.ends_with('.') {
                    s.trim_end_matches('.').to_string()
                } else {
                    s
                }
            })
            .collect();

        if !domains.is_empty() {
            rules.push(json!({
                "type": "field",
                "outboundTag": "direct",
                "domain": domains
            }));
            rules.push(json!({
                "type": "field",
                "outboundTag": "direct",
                "ip": domains // Catch IP patterns too
            }));
        }
    }

    // Add geoip rules
    rules.push(json!({
        "type": "field",
        "outboundTag": "direct",
        "ip": ["geoip:private", "geoip:cn"]
    }));
    rules.push(json!({
        "type": "field",
        "outboundTag": "direct",
        "domain": ["geosite:cn"]
    }));
    rules.push(json!({
        "type": "field",
        "outboundTag": "block",
        "domain": ["geosite:category-ads-all"]
    }));

    // Build DNS block
    let mut dns_obj = json!({});

    if !app_settings.custom_dns_json.trim().is_empty() {
        if let Ok(parsed_dns) =
            serde_json::from_str::<serde_json::Value>(&app_settings.custom_dns_json)
        {
            dns_obj = parsed_dns;
        }
    } else {
        let mut servers = vec![];
        if !app_settings.domestic_dns.is_empty() {
            servers.push(json!(app_settings.domestic_dns));
        }
        if !app_settings.remote_dns.is_empty() {
            servers.push(json!(app_settings.remote_dns));
        }
        if !app_settings.bootstrap_dns.is_empty() {
            servers.push(json!(app_settings.bootstrap_dns));
        }
        if servers.is_empty() {
            servers = vec![json!("8.8.8.8"), json!("1.1.1.1"), json!("localhost")];
        }

        let mut hosts = json!({});
        if !app_settings.dns_hosts.trim().is_empty() {
            for line in app_settings.dns_hosts.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let domain = parts[0];
                    let ips: Vec<&str> = parts[1..].to_vec();
                    if ips.len() == 1 {
                        hosts
                            .as_object_mut()
                            .unwrap()
                            .insert(domain.to_string(), json!(ips[0]));
                    } else {
                        hosts
                            .as_object_mut()
                            .unwrap()
                            .insert(domain.to_string(), json!(ips));
                    }
                }
            }
        }

        dns_obj = json!({
            "servers": servers,
            "hosts": hosts
        });
    }

    // Add Block SVCB
    if app_settings.block_svcb {
        rules.push(json!({
            "type": "field",
            "outboundTag": "block",
            "network": "udp",
            "port": 443
        })); // Simple heuristic for QUIC/SVCB blocking if not natively supported by client
    }

    json!({
        "log": {
            "loglevel": app_settings.log_level
        },
        "dns": dns_obj,
        "routing": {
            "domainStrategy": "AsIs",
            "rules": rules
        },
        "inbounds": [
            {
                "port": app_settings.socks_port,
                "listen": listen_ip,
                "protocol": "socks",
                "settings": {
                    "udp": app_settings.enable_udp
                },
                "sniffing": sniffing.clone()
            },
            {
                "port": app_settings.socks_port + 1,
                "listen": listen_ip,
                "protocol": "http",
                "sniffing": sniffing
            }
        ],
        "outbounds": [
            outbound,
            {
                "protocol": "freedom",
                "tag": "direct"
            },
            {
                "protocol": "blackhole",
                "tag": "block"
            }
        ]
    })
}
