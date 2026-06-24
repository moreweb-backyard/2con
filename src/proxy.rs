use crate::config::AppSettings;
use crate::error::AppError;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

use reqwest::Client;
use std::io::Cursor;

pub async fn download_xray_core() -> Result<(), AppError> {
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
        .await?
        .json()
        .await?;

    let tag_name = release_info["tag_name"]
        .as_str()
        .ok_or_else(|| AppError::Network("No tag_name found in release info".to_string()))?;

    let zip_name = format!("Xray-{}-{}.zip", os, arch);
    let download_url = format!(
        "https://github.com/XTLS/Xray-core/releases/download/{}/{}",
        tag_name, zip_name
    );

    let response = client
        .get(&download_url)
        .header("User-Agent", "2con-client")
        .send()
        .await?;

    let bytes = response.bytes().await?;
    let cursor = Cursor::new(bytes);

    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| AppError::Io(format!("Failed to open zip: {}", e)))?;

    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| AppError::Io(format!("Failed to read zip file {}: {}", i, e)))?;
        let outpath = match file.enclosed_name() {
            Some(path) => path.to_owned(),
            None => continue,
        };

        let filename = outpath.file_name().unwrap_or_default().to_string_lossy();
        if filename.starts_with("xray") || filename == "geoip.dat" || filename == "geosite.dat" {
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| AppError::Io(format!("Failed to create {}: {}", filename, e)))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| AppError::Io(format!("Failed to extract {}: {}", filename, e)))?;

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

pub async fn download_routing_rules() -> Result<(), AppError> {
    let client = Client::new();

    let geoip_url =
        "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat";
    let geosite_url =
        "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat";

    let geoip_bytes = client
        .get(geoip_url)
        .header("User-Agent", "2con-client")
        .send()
        .await?
        .bytes()
        .await?;

    std::fs::write("geoip.dat", &geoip_bytes)?;

    let geosite_bytes = client
        .get(geosite_url)
        .header("User-Agent", "2con-client")
        .send()
        .await?
        .bytes()
        .await?;

    std::fs::write("geosite.dat", &geosite_bytes)?;

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
    ) -> Result<(), AppError> {
        self.stop().await;

        let config_path = "xray_config.json";
        let config_pretty = serde_json::to_string_pretty(&config)
            .map_err(|e| AppError::ConfigGeneration(format!("Failed to format config: {}", e)))?;
        std::fs::write(config_path, config_pretty)?;

        let xray_bin = if cfg!(windows) { "xray.exe" } else { "./xray" };

        if !Path::new(xray_bin).exists() {
            download_xray_core().await?;
        }

        let mut cmd = Command::new(xray_bin);
        cmd.arg("-c")
            .arg(config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()
            .map_err(|e| AppError::XrayProcess(format!("Failed to spawn xray process: {}", e)))?;

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

impl Drop for ProxyRunner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let _ = child.kill().await;
                });
            }
        }
    }
}

pub fn generate_xray_config(
    parsed_cfg: &crate::config::ProxyConfig,
    address: &str,
    app_settings: &AppSettings,
) -> Result<Value, AppError> {
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
                .ok_or_else(|| AppError::ConfigGeneration("Wireguard settings is not an object".to_string()))?
                .insert("mtu".to_string(), json!(mtu));
        }
        if let Some(ref reserved) = parsed_cfg.reserved {
            settings_obj
                .as_object_mut()
                .ok_or_else(|| AppError::ConfigGeneration("Wireguard settings is not an object".to_string()))?
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
    } else if parsed_cfg.protocol == "vless" {
        json!({
            "vnext": [
                {
                    "address": address,
                    "port": parsed_cfg.port,
                    "users": [
                        {
                            "id": parsed_cfg.uuid,
                            "encryption": "none",
                            "flow": parsed_cfg.flow
                        }
                    ]
                }
            ]
        })
    } else {
        return Err(AppError::ConfigGeneration(format!(
            "Unsupported protocol: {}",
            parsed_cfg.protocol
        )));
    };

    let network = &parsed_cfg.transport;

    let mut stream_settings = json!({
        "network": network,
    });

    let stream_obj = stream_settings
        .as_object_mut()
        .ok_or_else(|| AppError::ConfigGeneration("Failed to modify stream settings".to_string()))?;

    let security = parsed_cfg.tls.to_lowercase();
    if security == "reality" {
        stream_obj.insert("security".to_string(), json!("reality"));
        stream_obj.insert(
            "realitySettings".to_string(),
            json!({
                "serverName": parsed_cfg.sni,
                "publicKey": parsed_cfg.pbk,
                "shortId": parsed_cfg.sid,
                "fingerprint": parsed_cfg.fp,
                "spiderX": parsed_cfg.spx,
                "show": false
            }),
        );
    } else if security == "tls" {
        stream_obj.insert("security".to_string(), json!("tls"));
        stream_obj.insert(
            "tlsSettings".to_string(),
            json!({
                "serverName": parsed_cfg.sni,
                "allowInsecure": false
            }),
        );
    } else if !security.is_empty() && security != "none" {
        stream_obj.insert("security".to_string(), json!(parsed_cfg.tls));
    } else {
        stream_obj.insert("security".to_string(), json!("none"));
    }

    match network.as_str() {
        "ws" => {
            stream_obj.insert(
                "wsSettings".to_string(),
                json!({
                    "path": parsed_cfg.path,
                    "headers": {
                        "Host": parsed_cfg.hostname
                    }
                }),
            );
        }
        "grpc" => {
            stream_obj.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": parsed_cfg.path
                }),
            );
        }
        "http" | "h2" => {
            stream_obj.insert(
                "httpSettings".to_string(),
                json!({
                    "path": parsed_cfg.path,
                    "host": [parsed_cfg.hostname]
                }),
            );
        }
        "tcp" => {
            if parsed_cfg.header_type == "http" {
                stream_obj.insert(
                    "tcpSettings".to_string(),
                    json!({
                        "header": {
                            "type": "http",
                            "request": {
                                "version": "1.1",
                                "method": "GET",
                                "path": [if parsed_cfg.path.is_empty() { "/".to_string() } else { parsed_cfg.path.clone() }],
                                "headers": {
                                    "Host": [parsed_cfg.hostname.clone()],
                                    "User-Agent": [
                                        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.131 Safari/537.36"
                                    ],
                                    "Accept-Encoding": ["gzip, deflate"],
                                    "Connection": ["keep-alive"],
                                    "Pragma": ["no-cache"]
                                }
                            },
                            "response": {
                                "version": "1.1",
                                "status": "200",
                                "reason": "OK",
                                "headers": {
                                    "Content-Type": ["application/octet-stream"],
                                    "Connection": ["keep-alive"],
                                    "Transfer-Encoding": ["chunked"],
                                    "Pragma": ["no-cache"]
                                }
                            }
                        }
                    }),
                );
            }
        }
        "kcp" | "mkcp" => {
            stream_obj.insert(
                "kcpSettings".to_string(),
                json!({
                    "mtu": 1350,
                    "tti": 50,
                    "uplinkCapacity": 12,
                    "downlinkCapacity": 100,
                    "congestion": false,
                    "readBufferSize": 2,
                    "writeBufferSize": 2,
                    "header": {
                        "type": if parsed_cfg.header_type.is_empty() { "none".to_string() } else { parsed_cfg.header_type.clone() }
                    },
                    "seed": parsed_cfg.seed
                }),
            );
        }
        "quic" => {
            stream_obj.insert(
                "quicSettings".to_string(),
                json!({
                    "security": if parsed_cfg.quic_security.is_empty() { "none".to_string() } else { parsed_cfg.quic_security.clone() },
                    "key": parsed_cfg.key,
                    "header": {
                        "type": if parsed_cfg.header_type.is_empty() { "none".to_string() } else { parsed_cfg.header_type.clone() }
                    }
                }),
            );
        }
        _ => {}
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
        outbound.as_object_mut()
            .ok_or_else(|| AppError::ConfigGeneration("Outbound settings is not an object".to_string()))?
            .insert(
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
                "ip": domains
            }));
        }
    }

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
                            .ok_or_else(|| AppError::ConfigGeneration("DNS hosts object is not an object".to_string()))?
                            .insert(domain.to_string(), json!(ips[0]));
                    } else {
                        hosts
                            .as_object_mut()
                            .ok_or_else(|| AppError::ConfigGeneration("DNS hosts object is not an object".to_string()))?
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

    if app_settings.block_svcb {
        rules.push(json!({
            "type": "field",
            "outboundTag": "block",
            "network": "udp",
            "port": 443
        }));
    }

    Ok(json!({
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
    }))
}
