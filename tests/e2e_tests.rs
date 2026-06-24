use base64::Engine;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[path = "../src/config.rs"]
pub mod config;

#[path = "../src/proxy.rs"]
pub mod proxy;

#[path = "../src/sysproxy.rs"]
pub mod sysproxy;

static CWD_MUTEX: Mutex<()> = Mutex::new(());

struct CwdGuard {
    original_cwd: PathBuf,
    temp_dir: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original_cwd);
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

fn isolate_execution() -> CwdGuard {
    let lock = CWD_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let original_cwd = std::env::current_dir().unwrap();
    let temp_dir = std::env::temp_dir().join(format!("2con_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    std::env::set_current_dir(&temp_dir).unwrap();
    CwdGuard {
        original_cwd,
        temp_dir,
        _lock: lock,
    }
}

struct SysProxyGuard;

impl Drop for SysProxyGuard {
    fn drop(&mut self) {
        let _ = sysproxy::disable_system_proxy();
    }
}

async fn start_mock_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0; 4096];
                if let Ok(n) = socket.read(&mut buf).await {
                    let req_str = String::from_utf8_lossy(&buf[..n]);
                    let path = if let Some(first_line) = req_str.lines().next() {
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if parts.len() > 1 { parts[1] } else { "/" }
                    } else {
                        "/"
                    };

                    let payload = match path {
                        p if p.starts_with("/empty") => "".to_string(),
                        p if p.starts_with("/malformed") => "invalid_base64_!!!".to_string(),
                        p if p.starts_with("/mixed") => {
                            let raw = vec![
                                "invalid://xyz",
                                "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQxQDEyLjM0LjU2Ljc4OjQ0Mw==#SS_TEST",
                                "unknown://abc",
                            ]
                            .join("\n");
                            base64::engine::general_purpose::STANDARD.encode(raw)
                        }
                        p if p.starts_with("/large") => {
                            let mut lines = Vec::new();
                            for i in 0..5000 {
                                lines.push(format!("vless://uuid{}@12.34.56.78:443?security=tls&sni=vless{}.example.com&type=tcp#VLESS_{}", i, i, i));
                            }
                            let raw = lines.join("\n");
                            base64::engine::general_purpose::STANDARD.encode(raw)
                        }
                        _ => {
                            let raw = vec![
                                "vmess://eyJhZGQiOiIxMi4zNC41Ni43OCIsInBvcnQiOjQ0MywiaWQiOiJ1dWlkMSIsImhvc3QiOiJ2bWVzcy5leGFtcGxlLmNvbSIsInBhdGgiOiIvIiwidGxzIjoidGxzIiwic25pIjoidm1lc3MuZXhwYW1wbGUuY29tIiwibmV0IjoidGNwIn0==",
                                "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQxQDEyLjM0LjU2Ljc4OjQ0Mw==#SS_TEST",
                                "vless://uuid2@12.34.56.78:443?security=tls&sni=vless.example.com&type=tcp#VLESS_TEST",
                                "trojan://password123@12.34.56.78:443?security=tls&sni=trojan.example.com&type=tcp#TROJAN_TEST",
                                "wg://private_key@12.34.56.78:51820?public_key=public_key&address=10.0.0.2/24&mtu=1420#WG_TEST",
                            ].join("\n");
                            base64::engine::general_purpose::STANDARD.encode(raw)
                        }
                    };

                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        payload.len(),
                        payload
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            });
        }
    });

    server_url
}

#[tokio::test]
async fn test_tier1_feature_coverage() {
    let server_url = start_mock_server().await;
    let _guard = isolate_execution();

    // Call config load/save functions to mock the subscription list
    let sub = config::Subscription {
        id: "test_sub".to_string(),
        url: format!("{}/", server_url),
        last_updated: "".to_string(),
    };
    config::save_subscriptions(&[sub]);

    let loaded_subs = config::load_subscriptions();
    assert_eq!(loaded_subs.len(), 1);
    assert_eq!(loaded_subs[0].url, format!("{}/", server_url));

    // Fetch the subscription payload
    let client = reqwest::Client::new();
    let response = client.get(&loaded_subs[0].url).send().await.unwrap();
    let body = response.text().await.unwrap();

    // Decode Base64
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .unwrap();
    let decoded_str = String::from_utf8(decoded).unwrap();

    // Parse configs and save them
    let mut profiles = Vec::new();
    for line in decoded_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(parsed) = config::ProxyConfig::parse(line) {
            let pid = uuid::Uuid::new_v4().to_string();
            let name = if let Some(idx) = line.find('#') {
                line[idx + 1..].to_string()
            } else {
                parsed.hostname.clone()
            };
            profiles.push(config::Profile {
                id: pid,
                name: format!("[Sub] {}", name),
                protocol: parsed.protocol,
                raw_link: line.to_string(),
                sub_group: "Personal".to_string(),
            });
        }
    }
    config::save_profiles(&profiles);

    // Verify that the profile list gets populated correctly in configs.json
    let saved_profiles = config::load_profiles();
    assert!(!saved_profiles.is_empty());

    // Assert details of each protocol
    // VMess
    let vmess_prof = saved_profiles
        .iter()
        .find(|p| p.protocol == "vmess")
        .expect("VMess profile should exist");
    let vmess_config =
        config::ProxyConfig::parse(&vmess_prof.raw_link).expect("Should parse VMess raw link");
    assert_eq!(vmess_config.protocol, "vmess");
    assert_eq!(vmess_config.addresses[0], "12.34.56.78");
    assert_eq!(vmess_config.port, 443);
    assert_eq!(vmess_config.uuid, "uuid1");
    assert_eq!(vmess_config.hostname, "vmess.example.com");
    assert_eq!(vmess_config.path, "/");
    assert_eq!(vmess_config.tls, "tls");
    assert_eq!(vmess_config.sni, "vmess.expample.com");
    assert_eq!(vmess_config.transport, "tcp");

    // Shadowsocks
    let ss_prof = saved_profiles
        .iter()
        .find(|p| p.protocol == "shadowsocks")
        .expect("Shadowsocks profile should exist");
    let ss_config =
        config::ProxyConfig::parse(&ss_prof.raw_link).expect("Should parse Shadowsocks raw link");
    assert_eq!(ss_config.protocol, "shadowsocks");
    assert_eq!(ss_config.addresses[0], "12.34.56.78");
    assert_eq!(ss_config.port, 443);
    assert_eq!(ss_config.uuid, "aes-256-gcm:password1");
    assert_eq!(ss_config.hostname, "");
    assert_eq!(ss_config.path, "");
    assert_eq!(ss_config.tls, "");
    assert_eq!(ss_config.sni, "");
    assert_eq!(ss_config.transport, "tcp");

    // VLESS
    let vless_prof = saved_profiles
        .iter()
        .find(|p| p.protocol == "vless")
        .expect("VLESS profile should exist");
    let vless_config =
        config::ProxyConfig::parse(&vless_prof.raw_link).expect("Should parse VLESS raw link");
    assert_eq!(vless_config.protocol, "vless");
    assert_eq!(vless_config.addresses[0], "12.34.56.78");
    assert_eq!(vless_config.port, 443);
    assert_eq!(vless_config.uuid, "uuid2");
    assert_eq!(vless_config.sni, "vless.example.com");
    assert_eq!(vless_config.tls, "tls");
    assert_eq!(vless_config.transport, "tcp");

    // Trojan
    let trojan_prof = saved_profiles
        .iter()
        .find(|p| p.protocol == "trojan")
        .expect("Trojan profile should exist");
    let trojan_config =
        config::ProxyConfig::parse(&trojan_prof.raw_link).expect("Should parse Trojan raw link");
    assert_eq!(trojan_config.protocol, "trojan");
    assert_eq!(trojan_config.addresses[0], "12.34.56.78");
    assert_eq!(trojan_config.port, 443);
    assert_eq!(trojan_config.uuid, "password123");
    assert_eq!(trojan_config.sni, "trojan.example.com");
    assert_eq!(trojan_config.tls, "tls");
    assert_eq!(trojan_config.transport, "tcp");

    // For Wireguard, since the parser returns None currently, assert that it is parsed successfully
    // (this will fail now, but it's expected since implementation is in progress).
    let wg_link = "wg://private_key@12.34.56.78:51820?public_key=public_key&address=10.0.0.2/24&mtu=1420#WG_TEST";
    let parsed_wg = config::ProxyConfig::parse(wg_link);
    assert!(
        parsed_wg.is_some(),
        "Assert that Wireguard parsed successfully (expected to fail since implementation is in progress)"
    );
}

#[tokio::test]
async fn test_tier2_boundary_corner_cases() {
    let server_url = start_mock_server().await;
    let _guard = isolate_execution();
    let client = reqwest::Client::new();

    // 1. Test empty response (mock server returns empty string, verify 0 profiles parsed/saved)
    let response = client
        .get(&format!("{}/empty", server_url))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    let mut profiles = Vec::new();
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(body.trim()) {
        if let Ok(decoded_str) = String::from_utf8(decoded) {
            for line in decoded_str.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(parsed) = config::ProxyConfig::parse(line) {
                    profiles.push(config::Profile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: "test".to_string(),
                        protocol: parsed.protocol,
                        raw_link: line.to_string(),
                        sub_group: "Personal".to_string(),
                    });
                }
            }
        }
    }
    config::save_profiles(&profiles);
    let saved = config::load_profiles();
    assert_eq!(saved.len(), 0);

    // 2. Test malformed base64 (mock server returns invalid characters, verify error handled gracefully with no panic/crash)
    let response = client
        .get(&format!("{}/malformed", server_url))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    let decoded_res = base64::engine::general_purpose::STANDARD.decode(body.trim());
    assert!(decoded_res.is_err());
    let mut profiles = Vec::new();
    if let Ok(decoded) = decoded_res {
        if let Ok(decoded_str) = String::from_utf8(decoded) {
            for line in decoded_str.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(parsed) = config::ProxyConfig::parse(line) {
                    profiles.push(config::Profile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: "test".to_string(),
                        protocol: parsed.protocol,
                        raw_link: line.to_string(),
                        sub_group: "Personal".to_string(),
                    });
                }
            }
        }
    }
    config::save_profiles(&profiles);
    let saved = config::load_profiles();
    assert_eq!(saved.len(), 0);

    // 3. Test mixed valid and invalid protocol strings (verify invalid lines are skipped, valid lines are parsed and saved, no panic/crash)
    let response = client
        .get(&format!("{}/mixed", server_url))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    let mut profiles = Vec::new();
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(body.trim()) {
        if let Ok(decoded_str) = String::from_utf8(decoded) {
            for line in decoded_str.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(parsed) = config::ProxyConfig::parse(line) {
                    profiles.push(config::Profile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: "test".to_string(),
                        protocol: parsed.protocol,
                        raw_link: line.to_string(),
                        sub_group: "Personal".to_string(),
                    });
                }
            }
        }
    }
    config::save_profiles(&profiles);
    let saved = config::load_profiles();
    assert_eq!(saved.len(), 1); // Only shadowsocks is valid
    assert_eq!(saved[0].protocol, "shadowsocks");

    // 4. Test large payload (mock server returns 5000 valid/invalid config lines, verify no stack overflow or out of memory)
    let response = client
        .get(&format!("{}/large", server_url))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    let mut profiles = Vec::new();
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(body.trim()) {
        if let Ok(decoded_str) = String::from_utf8(decoded) {
            for line in decoded_str.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(parsed) = config::ProxyConfig::parse(line) {
                    profiles.push(config::Profile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: "test".to_string(),
                        protocol: parsed.protocol,
                        raw_link: line.to_string(),
                        sub_group: "Personal".to_string(),
                    });
                }
            }
        }
    }
    config::save_profiles(&profiles);
    let saved = config::load_profiles();
    assert_eq!(saved.len(), 5000);
}

#[tokio::test]
async fn test_tier3_cross_feature_combinations() {
    let _guard = isolate_execution();

    // Toggle system proxy settings with drop guard protection
    let _proxy_guard = SysProxyGuard;
    let _ = sysproxy::enable_system_proxy(10808);
    let _ = sysproxy::disable_system_proxy();

    // Toggle auto-start settings
    let _ = sysproxy::enable_auto_start();
    let _ = sysproxy::disable_auto_start();

    // Test settings synchronization: load settings via config::load_settings(), modify properties, save, reload and verify
    let mut settings = config::load_settings();
    settings.socks_port = 12345;
    settings.bypass_list = "localhost;127.0.0.1;google.com".to_string();
    config::save_settings(&settings);

    let reloaded = config::load_settings();
    assert_eq!(reloaded.socks_port, 12345);
    assert_eq!(reloaded.bypass_list, "localhost;127.0.0.1;google.com");
}

#[tokio::test]
async fn test_tier4_real_world_application() {
    let server_url = start_mock_server().await;
    let _guard = isolate_execution();
    let client = reqwest::Client::new();

    // Write default settings
    let default_settings = config::AppSettings::default();
    config::save_settings(&default_settings);

    // Update subscription
    let sub = config::Subscription {
        id: "test_sub".to_string(),
        url: format!("{}/", server_url),
        last_updated: "".to_string(),
    };
    config::save_subscriptions(&[sub.clone()]);

    // Parse config
    let response = client.get(&sub.url).send().await.unwrap();
    let body = response.text().await.unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .unwrap();
    let decoded_str = String::from_utf8(decoded).unwrap();

    let mut profiles = Vec::new();
    for line in decoded_str.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(parsed) = config::ProxyConfig::parse(line) {
            let pid = uuid::Uuid::new_v4().to_string();
            let name = if let Some(idx) = line.find('#') {
                line[idx + 1..].to_string()
            } else {
                parsed.hostname.clone()
            };
            profiles.push(config::Profile {
                id: pid,
                name: format!("[Sub] {}", name),
                protocol: parsed.protocol,
                raw_link: line.to_string(),
                sub_group: "Personal".to_string(),
            });
        }
    }
    config::save_profiles(&profiles);

    // Select VLESS and Trojan profiles, and call proxy::generate_xray_config for each
    let vless_prof = profiles
        .iter()
        .find(|p| p.protocol == "vless")
        .expect("VLESS profile exists");
    let trojan_prof = profiles
        .iter()
        .find(|p| p.protocol == "trojan")
        .expect("Trojan profile exists");

    let vless_config = config::ProxyConfig::parse(&vless_prof.raw_link).unwrap();
    let xray_vless =
        proxy::generate_xray_config(&vless_config, &vless_config.addresses[0], &default_settings);

    let trojan_config = config::ProxyConfig::parse(&trojan_prof.raw_link).unwrap();
    let xray_trojan = proxy::generate_xray_config(
        &trojan_config,
        &trojan_config.addresses[0],
        &default_settings,
    );

    // Assert generated Xray JSON configuration has correct address, port, protocol, and streamSettings (TLS/SNI/etc.)

    // VLESS assertions
    assert_eq!(
        xray_vless["outbounds"][0]["protocol"].as_str(),
        Some("vless")
    );
    assert_eq!(
        xray_vless["outbounds"][0]["settings"]["vnext"][0]["address"].as_str(),
        Some("12.34.56.78")
    );
    assert_eq!(
        xray_vless["outbounds"][0]["settings"]["vnext"][0]["port"].as_u64(),
        Some(443)
    );
    assert_eq!(
        xray_vless["outbounds"][0]["streamSettings"]["network"].as_str(),
        Some("tcp")
    );
    assert_eq!(
        xray_vless["outbounds"][0]["streamSettings"]["security"].as_str(),
        Some("tls")
    );
    assert_eq!(
        xray_vless["outbounds"][0]["streamSettings"]["tlsSettings"]["serverName"].as_str(),
        Some("vless.example.com")
    );

    // Trojan assertions
    assert_eq!(
        xray_trojan["outbounds"][0]["protocol"].as_str(),
        Some("trojan")
    );
    assert_eq!(
        xray_trojan["outbounds"][0]["settings"]["servers"][0]["address"].as_str(),
        Some("12.34.56.78")
    );
    assert_eq!(
        xray_trojan["outbounds"][0]["settings"]["servers"][0]["port"].as_u64(),
        Some(443)
    );
    assert_eq!(
        xray_trojan["outbounds"][0]["streamSettings"]["network"].as_str(),
        Some("tcp")
    );
    assert_eq!(
        xray_trojan["outbounds"][0]["streamSettings"]["security"].as_str(),
        Some("tls")
    );
    assert_eq!(
        xray_trojan["outbounds"][0]["streamSettings"]["tlsSettings"]["serverName"].as_str(),
        Some("trojan.example.com")
    );
}
