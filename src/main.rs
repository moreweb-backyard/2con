mod config;
mod scanner;
mod proxy;
mod tester;

slint::include_modules!();

use std::sync::Arc;
use tokio::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::main]
async fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let ui_handle = ui.as_weak();
    
    let proxy_runner = Arc::new(Mutex::new(proxy::ProxyRunner::new()));
    let autopilot_active = Arc::new(AtomicBool::new(false));

    let autopilot_active_clone = autopilot_active.clone();
    let ui_handle_ap = ui_handle.clone();
    
    ui.on_set_autopilot(move |enabled| {
        autopilot_active_clone.store(enabled, Ordering::Relaxed);
        if enabled {
            let ap_active = autopilot_active_clone.clone();
            let ui_weak = ui_handle_ap.clone();
            tokio::spawn(async move {
                while ap_active.load(Ordering::Relaxed) {
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    // Test connection
                    if !tester::test_proxy_connection().await {
                        // If it fails and we are connected, trigger a reconnect
                        let ui_weak_clone = ui_weak.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak_clone.upgrade() {
                                if u.get_connected() {
                                    // Hack to trigger reconnect: call toggle twice
                                    u.invoke_toggle_connect();
                                    u.invoke_toggle_connect();
                                }
                            }
                        }).unwrap();
                    }
                }
            });
        }
    });

    ui.on_toggle_connect(move || {
        let ui = ui_handle.unwrap();
        let currently_connected = ui.get_connected();
        
        // Example hardcoded config link for testing (would be dynamic in real app)
        let sample_link = "vless://d342d11e-d424-4583-b36e-524ab1f0afa4@scan.cfl:443?encryption=none&security=tls&sni=speed.cloudflare.com&host=speed.cloudflare.com&type=ws&path=%2F#Test";
        
        if currently_connected {
            ui.set_status_text("Disconnected".into());
            ui.set_connected(false);
            ui.set_current_ip("N/A".into());
            ui.set_latency("-".into());
            
            let runner_clone = proxy_runner.clone();
            tokio::spawn(async move {
                let mut runner = runner_clone.lock().await;
                runner.stop().await;
            });
        } else {
            ui.set_status_text("Connecting...".into());
            let ui_clone = ui.as_weak();
            let runner_clone = proxy_runner.clone();
            
            tokio::spawn(async move {
                // If it's scan.cfl, we trigger the scanner
                let parsed_cfg = config::ProxyConfig::parse(sample_link).unwrap();
                
                let mut final_ip = parsed_cfg.addresses[0].clone();
                let mut best_latency = std::time::Duration::from_secs(999);
                
                if parsed_cfg.addresses.contains(&"scan.cfl".to_string()) || parsed_cfg.hostname == "scan.cfl" {
                    // Try to find a clean IP
                    let ips = scanner::get_common_cf_ips();
                    for ip in ips {
                        let res = scanner::test_ip(&ip, parsed_cfg.port, 2000).await;
                        if res.is_valid && res.latency < best_latency {
                            final_ip = ip;
                            best_latency = res.latency;
                        }
                    }
                } else if parsed_cfg.addresses.len() > 1 {
                    // Load balance between multiple IPs by testing them
                    for ip in &parsed_cfg.addresses {
                        let res = scanner::test_ip(ip, parsed_cfg.port, 2000).await;
                        // For a real proxy, testing just TCP is enough to see if it connects
                        if res.latency < best_latency {
                            final_ip = ip.clone();
                            best_latency = res.latency;
                        }
                    }
                }
                
                let xray_cfg = proxy::generate_xray_config(
                    &final_ip,
                    parsed_cfg.port,
                    &parsed_cfg.uuid,
                    &parsed_cfg.sni,
                    &parsed_cfg.hostname,
                    &parsed_cfg.path
                );
                
                let mut runner = runner_clone.lock().await;
                if let Err(e) = runner.start(xray_cfg).await {
                    eprintln!("Error starting proxy: {}", e);
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_clone.upgrade() {
                            u.set_status_text("Error".into());
                            u.set_connected(false);
                        }
                    }).unwrap();
                    return;
                }
                
                let latency_str = if best_latency.as_millis() < 999000 {
                    format!("{}ms", best_latency.as_millis())
                } else {
                    "-".to_string()
                };

                slint::invoke_from_event_loop(move || {
                    if let Some(u) = ui_clone.upgrade() {
                        u.set_status_text("Connected".into());
                        u.set_connected(true);
                        u.set_current_ip(final_ip.into());
                        u.set_latency(latency_str.into());
                    }
                }).unwrap();
            });
        }
    });

    ui.run()
}
