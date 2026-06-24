use crate::MainWindow;
use crate::state::AppCore;
use crate::error::AppError;
use crate::scanner;
use crate::tester;
use slint::Weak;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tracing::{info, warn, error};

pub fn handle_clear_logs(ui_weak: &Weak<MainWindow>) {
    if let Some(u) = ui_weak.upgrade() {
        u.set_app_logs("".into());
    }
}

pub fn handle_set_autopilot(ui_weak: &Weak<MainWindow>, core: &AppCore, enabled: bool) {
    core.autopilot_active.store(enabled, Ordering::Relaxed);
    if let Some(u) = ui_weak.upgrade() {
        u.set_autopilot_enabled(enabled);
    }
    if enabled {
        let ap_active = core.autopilot_active.clone();
        let ui_clone = ui_weak.clone();
        let core_clone = core.clone();
        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(2);
            let max_retries = 5;
            
            while ap_active.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_secs(10)).await;
                
                let (tx, rx) = tokio::sync::oneshot::channel();
                let ui_weak = ui_clone.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let res = ui_weak.upgrade().map(|u| (u.get_connected(), u.get_socks_port().to_string().parse::<u16>().unwrap_or(10808)));
                    let _ = tx.send(res);
                });
                let (connected, socks_port) = match rx.await {
                    Ok(Some(res)) => res,
                    _ => continue,
                };
                
                if !connected {
                    continue;
                }
                
                if tester::real_delay(socks_port).await.is_none() {
                    warn!("Connection lost, starting autopilot reconnection...");
                    
                    let mut retries = 0;
                    let mut success = false;
                    
                    while retries < max_retries && ap_active.load(Ordering::Relaxed) {
                        retries += 1;
                        info!("Reconnection attempt {}/{}", retries, max_retries);
                        
                        let ui_weak = ui_clone.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak.upgrade() {
                                u.set_error_message(format!("Connection lost. Reconnecting (Attempt {}/{})...", retries, max_retries).into());
                                u.set_error_level("warning".into());
                            }
                        });
                        
                        let _ = perform_disconnect(&core_clone).await;
                        
                        tokio::time::sleep(backoff).await;
                        
                        match perform_connect(&ui_clone, &core_clone).await {
                            Ok(_) => {
                                success = true;
                                info!("Autopilot reconnection successful!");
                                let ui_weak = ui_clone.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(u) = ui_weak.upgrade() {
                                        u.set_error_message("Reconnected successfully!".into());
                                        u.set_error_level("info".into());
                                    }
                                });
                                break;
                            }
                            Err(e) => {
                                error!("Autopilot reconnection attempt failed: {}", e);
                                backoff = (backoff * 2).min(Duration::from_secs(30));
                            }
                        }
                    }
                    
                    if !success {
                        warn!("Autopilot reconnection failed after max retries.");
                        let _ = perform_disconnect(&core_clone).await;
                        let ui_weak = ui_clone.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak.upgrade() {
                                u.set_connected(false);
                                u.set_connection_state("disconnected".into());
                                u.set_error_message("Reconnection failed. Autopilot stopped.".into());
                                u.set_error_level("error".into());
                            }
                        });
                    }
                }
            }
        });
    }
}

pub async fn perform_disconnect(core: &AppCore) -> Result<(), AppError> {
    let mut runner = core.proxy_runner.lock().await;
    runner.stop().await;
    
    let _ = crate::sysproxy::disable_system_proxy();
    
    Ok(())
}

pub async fn perform_connect(ui_weak: &Weak<MainWindow>, core: &AppCore) -> Result<(), AppError> {
    let active_id = core.active_config_id.lock().unwrap().clone();
    if active_id.is_empty() {
        return Err(AppError::ConfigParse("No active proxy configuration selected.".to_string()));
    }

    let profile_opt = {
        let p_guard = core.profiles.lock().unwrap();
        p_guard.iter().find(|p| p.id == active_id).cloned()
    };

    let profile = profile_opt.ok_or_else(|| AppError::ConfigParse("Selected configuration not found.".to_string()))?;
    let parsed_cfg = crate::config::ProxyConfig::parse(&profile.raw_link)
        .ok_or_else(|| AppError::ConfigParse("Failed to parse active proxy configuration.".to_string()))?;

    let mut final_ip = parsed_cfg.addresses.first().cloned().unwrap_or_default();
    let mut best_latency = Duration::from_secs(999);

    let is_scanning = parsed_cfg.addresses.len() == 1 && parsed_cfg.addresses[0] == "scan.cfl";

    if is_scanning {
        let ui_clone = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(u) = ui_clone.upgrade() {
                u.set_is_scanning(true);
            }
        });

        let ips = scanner::get_common_cf_ips();
        for ip in ips {
            let res = scanner::test_ip(&ip, parsed_cfg.port, 2000).await;
            if res.is_valid && res.latency < best_latency {
                final_ip = ip;
                best_latency = res.latency;
            }
        }

        let ui_clone = ui_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(u) = ui_clone.upgrade() {
                u.set_is_scanning(false);
            }
        });
    } else if parsed_cfg.addresses.len() > 1 {
        for ip in &parsed_cfg.addresses {
            let res = scanner::test_ip(ip, parsed_cfg.port, 2000).await;
            if res.latency < best_latency {
                final_ip = ip.clone();
                best_latency = res.latency;
            }
        }
    }

    let settings = core.app_settings.lock().unwrap().clone();
    let xray_cfg = crate::proxy::generate_xray_config(&parsed_cfg, &final_ip, &settings)?;

    let mut runner = core.proxy_runner.lock().await;

    let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let ui_logs = ui_weak.clone();
    
    tokio::spawn(async move {
        while let Some(log_msg) = log_rx.recv().await {
            if log_msg.contains("[ERROR]") {
                error!("{}", log_msg.trim());
            } else {
                info!("{}", log_msg.trim());
            }
            
            let ui_weak = ui_logs.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(u) = ui_weak.upgrade() {
                    let current = u.get_app_logs();
                    let mut new_logs = format!("{}{}", current, log_msg);
                    if new_logs.len() > 10000 {
                        let split_point = new_logs.len() - 5000;
                        if let Some(idx) = new_logs[split_point..].find('\n') {
                            new_logs = new_logs[split_point + idx + 1..].to_string();
                        } else {
                            new_logs = new_logs[split_point..].to_string();
                        }
                    }
                    u.set_app_logs(new_logs.into());
                }
            });
        }
    });

    runner.start(xray_cfg, log_tx).await?;

    let _ = crate::sysproxy::enable_system_proxy(settings.socks_port + 1);

    let latency_str = if best_latency.as_millis() < 999000 {
        format!("{}ms", best_latency.as_millis())
    } else {
        "-".to_string()
    };

    let ui_clone = ui_weak.clone();
    let final_ip_clone = final_ip.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(u) = ui_clone.upgrade() {
            u.set_connected(true);
            u.set_connection_state("connected".into());
            u.set_current_ip(final_ip_clone.into());
            u.set_latency(latency_str.into());
        }
    });

    Ok(())
}

pub fn handle_toggle_connect(ui_weak: Weak<MainWindow>, core: AppCore) {
    let ui_weak_clone = ui_weak.clone();
    let core_clone = core.clone();
    
    tokio::spawn(async move {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ui_weak = ui_weak_clone.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let res = ui_weak.upgrade().map(|u| u.get_connected()).unwrap_or(false);
            let _ = tx.send(res);
        });
        let currently_connected = rx.await.unwrap_or(false);

        if currently_connected {
            slint::invoke_from_event_loop({
                let ui_weak = ui_weak_clone.clone();
                move || {
                    if let Some(u) = ui_weak.upgrade() {
                        u.set_connected(false);
                        u.set_connection_state("disconnected".into());
                        u.set_current_ip("N/A".into());
                        u.set_latency("-".into());
                    }
                }
            }).unwrap();

            if let Err(e) = perform_disconnect(&core_clone).await {
                error!("Disconnect error: {}", e);
            }
        } else {
            slint::invoke_from_event_loop({
                let ui_weak = ui_weak_clone.clone();
                move || {
                    if let Some(u) = ui_weak.upgrade() {
                        u.set_connection_state("connecting".into());
                        u.set_current_ip("Connecting...".into());
                    }
                }
            }).unwrap();

            match perform_connect(&ui_weak_clone, &core_clone).await {
                Ok(_) => {
                    info!("Successfully connected!");
                    start_duration_timer(&ui_weak_clone);
                }
                Err(e) => {
                    error!("Connection error: {}", e);
                    let err_msg = e.to_string();
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            u.set_connected(false);
                            u.set_connection_state("disconnected".into());
                            u.set_current_ip("Error".into());
                            u.set_error_message(err_msg.into());
                            u.set_error_level("error".into());
                        }
                    }).unwrap();
                }
            }
        }
    });
}

fn start_duration_timer(ui_weak: &Weak<MainWindow>) {
    let ui_clone = ui_weak.clone();
    tokio::spawn(async move {
        let start_time = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            
            let (tx, rx) = tokio::sync::oneshot::channel();
            let ui_weak = ui_clone.clone();
            let elapsed = start_time.elapsed().as_secs();
            let hours = elapsed / 3600;
            let minutes = (elapsed % 3600) / 60;
            let seconds = elapsed % 60;
            let duration_str = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);

            let _ = slint::invoke_from_event_loop(move || {
                if let Some(u) = ui_weak.upgrade() {
                    if u.get_connected() {
                        u.set_connection_duration(duration_str.into());
                        let _ = tx.send(true);
                        return;
                    }
                }
                let _ = tx.send(false);
            });

            if !rx.await.unwrap_or(false) {
                break;
            }
        }
    });
}
