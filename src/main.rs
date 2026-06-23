mod config;
mod scanner;
mod proxy;
mod tester;
mod sysproxy;

slint::include_modules!();

use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::rc::Rc;
use slint::{ModelRc, VecModel, Model};
#[cfg(target_family = "windows")]
use i_slint_backend_winit::WinitWindowAccessor;

#[tokio::main]
async fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    let ui_handle = ui.as_weak();
    
    let proxy_runner = Arc::new(TokioMutex::new(proxy::ProxyRunner::new()));
    let autopilot_active = Arc::new(AtomicBool::new(false));

    // 0. Load App Settings
    let app_settings = Arc::new(StdMutex::new(config::load_settings()));
    {
        let s_guard = app_settings.lock().unwrap();
        ui.set_socks_port(s_guard.socks_port.to_string().into());
        ui.set_mux_enabled(s_guard.mux_enabled);
        ui.set_log_level(s_guard.log_level.clone().into());
        ui.set_docking_position(s_guard.docking.clone().into());
    }

    // 1. Config List Model
    let profiles = Arc::new(StdMutex::new(config::load_profiles()));
    let proxy_model: Rc<VecModel<ProxyItem>> = Rc::new(VecModel::default());
    
    let profiles_guard = profiles.lock().unwrap();
    for p in profiles_guard.iter() {
        let mut address = String::new();
        let mut port = String::new();
        let mut transport = String::new();
        let mut tls = String::new();
        
        if let Some(parsed) = config::ProxyConfig::parse(&p.raw_link) {
            address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
            port = parsed.port.to_string();
            transport = parsed.transport;
            tls = parsed.tls;
        }
        
        proxy_model.push(ProxyItem {
            id: p.id.clone().into(),
            name: p.name.clone().into(),
            protocol: p.protocol.to_uppercase().into(),
            address: address.into(),
            port: port.into(),
            transport: transport.into(),
            tls: tls.into(),
            sub_group: p.sub_group.clone().into(),
            is_active: false,
            latency: "Ping...".into(),
            latency_color: slint::Color::from_rgb_u8(136, 136, 136),
        });
    }
    let mut groups = std::collections::HashSet::new();
    for p in profiles_guard.iter() {
        groups.insert(p.sub_group.clone());
    }
    drop(profiles_guard);
    
    let mut groups_vec: Vec<String> = groups.into_iter().collect();
    groups_vec.sort();
    groups_vec.retain(|g| g != "All");
    groups_vec.insert(0, "All".to_string());
    let slint_groups: std::rc::Rc<slint::VecModel<slint::SharedString>> = std::rc::Rc::new(slint::VecModel::from(
        groups_vec.into_iter().map(|s| s.into()).collect::<Vec<slint::SharedString>>()
    ));
    ui.set_subscription_groups(slint::ModelRc::new(slint_groups));
    
    ui.set_proxy_list(ModelRc::new(proxy_model.clone()));

    let proxy_model_filter = proxy_model.clone();
    let profiles_filter = profiles.clone();
    let ui_filter = ui.as_weak();
    ui.on_filter_changed(move || {
        if let Some(u) = ui_filter.upgrade() {
            let filter_group = u.get_filter_group().to_string();
            let profiles_guard = profiles_filter.lock().unwrap();
            
            let mut items = Vec::new();
            
            for p in profiles_guard.iter() {
                if filter_group == "All" || p.sub_group == filter_group {
                    let mut address = String::new();
                    let mut port = String::new();
                    let mut transport = String::new();
                    let mut tls = String::new();
                    
                    if let Some(parsed) = config::ProxyConfig::parse(&p.raw_link) {
                        address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
                        port = parsed.port.to_string();
                        transport = parsed.transport;
                        tls = parsed.tls;
                    }
                    
                    items.push(ProxyItem {
                        id: p.id.clone().into(),
                        name: p.name.clone().into(),
                        protocol: p.protocol.to_uppercase().into(),
                        address: address.into(),
                        port: port.into(),
                        transport: transport.into(),
                        tls: tls.into(),
                        sub_group: p.sub_group.clone().into(),
                        is_active: false,
                        latency: "Ping...".into(),
                        latency_color: slint::Color::from_rgb_u8(136, 136, 136),
                    });
                }
            }
            
            proxy_model_filter.set_vec(items);
        }
    });

    // 1.1 Subscription List Model
    let subs = Arc::new(StdMutex::new(config::load_subscriptions()));
    let sub_model: Rc<VecModel<SubItem>> = Rc::new(VecModel::default());
    
    {
        let subs_guard = subs.lock().unwrap();
        for s in subs_guard.iter() {
            sub_model.push(SubItem {
                id: s.id.clone().into(),
                url: s.url.clone().into(),
                last_updated: s.last_updated.clone().into(),
            });
        }
    }
    ui.set_subscription_list(ModelRc::new(sub_model.clone()));

    // Apply Initial Docking
    {
        let docking = app_settings.lock().unwrap().docking.clone();
        if docking != "None" {
            ui.window().with_winit_window(|w| {
                if let Some(monitor) = w.current_monitor() {
                    let size = monitor.size();
                    let w_size = w.outer_size();
                    let pos = match docking.as_str() {
                        "Top Left" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                        "Top Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(size.width.saturating_sub(w_size.width), 0),
                        "Bottom Left" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, size.height.saturating_sub(w_size.height)),
                        "Bottom Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(size.width.saturating_sub(w_size.width), size.height.saturating_sub(w_size.height)),
                        _ => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                    };
                    w.set_outer_position(pos);
                }
            });
        }
    }

    // 1.5 Auto-Ping loaded profiles in background
    let profiles_ping = profiles.clone();
    let ui_ping = ui.as_weak();
    tokio::spawn(async move {
        // Wait a tiny bit for UI to render
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        let profiles_copy = {
            let p_guard = profiles_ping.lock().unwrap();
            p_guard.iter().cloned().collect::<Vec<_>>()
        };
        
        for (idx, profile) in profiles_copy.iter().enumerate() {
            if let Some(parsed) = config::ProxyConfig::parse(&profile.raw_link) {
                if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                    let ip = parsed.addresses[0].clone();
                    let port = parsed.port;
                    let res = scanner::test_ip(&ip, port, 3000).await;
                    
                    let lat_str = if res.is_valid && res.latency.as_millis() < 3000 {
                        format!("{}ms", res.latency.as_millis())
                    } else {
                        "Timeout".to_string()
                    };
                    
                    let ui_weak_clone = ui_ping.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let list = u.get_proxy_list();
                            if let Some(mut item) = list.row_data(idx) {
                                item.latency = lat_str.into();
                                item.latency_color = if res.is_valid && res.latency.as_millis() < 150 {
                                    slint::Color::from_rgb_u8(16, 185, 129)
                                } else if res.is_valid && res.latency.as_millis() < 300 {
                                    slint::Color::from_rgb_u8(245, 158, 11)
                                } else if res.is_valid && res.latency.as_millis() < 3000 {
                                    slint::Color::from_rgb_u8(239, 68, 68)
                                } else {
                                    slint::Color::from_rgb_u8(136, 136, 136)
                                };
                                list.set_row_data(idx, item);
                            }
                        }
                    }).unwrap();
                } else if !parsed.addresses.is_empty() && parsed.addresses[0] == "scan.cfl" {
                    let ui_weak_clone = ui_ping.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let list = u.get_proxy_list();
                            if let Some(mut item) = list.row_data(idx) {
                                item.latency = "Dynamic".into();
                                item.latency_color = slint::Color::from_rgb_u8(0, 229, 255);
                                list.set_row_data(idx, item);
                            }
                        }
                    }).unwrap();
                }
            }
        }
    });

    // 2. Window Control Callbacks
    ui.on_close_window(|| {
        slint::quit_event_loop().unwrap();
    });
    
    let ui_drag = ui.as_weak();
    ui.on_drag_window(move || {
        if let Some(u) = ui_drag.upgrade() {
            u.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });

    let app_settings_update = app_settings.clone();
    let ui_settings = ui.as_weak();
    ui.on_settings_changed(move || {
        if let Some(u) = ui_settings.upgrade() {
            let mut s_guard = app_settings_update.lock().unwrap();
            s_guard.socks_port = u.get_socks_port().to_string().parse().unwrap_or(10808);
            s_guard.mux_enabled = u.get_mux_enabled();
            s_guard.log_level = u.get_log_level().to_string();
            
            let docking = u.get_docking_position().to_string();
            let docking_changed = s_guard.docking != docking;
            s_guard.docking = docking.clone();
            
            config::save_settings(&s_guard);
            
            if docking_changed && docking != "None" {
                u.window().with_winit_window(|w| {
                    if let Some(monitor) = w.current_monitor() {
                        let size = monitor.size();
                        let w_size = w.outer_size();
                        let pos = match docking.as_str() {
                            "Top Left" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                            "Top Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(size.width.saturating_sub(w_size.width), 0),
                            "Bottom Left" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, size.height.saturating_sub(w_size.height)),
                            "Bottom Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(size.width.saturating_sub(w_size.width), size.height.saturating_sub(w_size.height)),
                            _ => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                        };
                        w.set_outer_position(pos);
                    }
                });
            }
        }
    });

    // 3. Import Configs
    let profiles_import = profiles.clone();
    let proxy_model_import = proxy_model.clone();
    let ui_import = ui.as_weak();
    ui.on_import_proxy(move |link| {
        let link_str = link.to_string();
        if let Some(parsed) = config::ProxyConfig::parse(&link_str) {
            let id = uuid::Uuid::new_v4().to_string();
            let name = if let Some(idx) = link_str.find('#') {
                link_str[idx+1..].to_string()
            } else {
                parsed.hostname.clone()
            };
            
            let profile = config::Profile {
                id: id.clone(),
                name: name.clone(),
                protocol: parsed.protocol.clone(),
                raw_link: link_str.clone(),
                sub_group: "Personal".to_string(),
            };
            
            let mut p_guard = profiles_import.lock().unwrap();
            p_guard.push(profile);
            config::save_profiles(&p_guard);
            
            let address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
            
            proxy_model_import.push(ProxyItem {
                id: id.into(),
                name: name.into(),
                protocol: parsed.protocol.to_uppercase().into(),
                address: address.into(),
                port: parsed.port.to_string().into(),
                transport: parsed.transport.clone().into(),
                tls: parsed.tls.clone().into(),
                sub_group: "Personal".into(),
                is_active: false,
                latency: "-".into(),
                latency_color: slint::Color::from_rgb_u8(136, 136, 136),
            });
        } else {
            if let Some(u) = ui_import.upgrade() {
                u.set_current_ip("Invalid Config Link!".into());
                u.set_active_tab(0);
            }
        }
    });

    // 4. Select and Delete Config
    let proxy_model_select = proxy_model.clone();
    let active_config_id = Arc::new(StdMutex::new(String::new()));
    let active_id_select = active_config_id.clone();
    let ui_select = ui.as_weak();
    
    ui.on_select_proxy(move |id| {
        let id_str = id.to_string();
        let mut active_id = active_id_select.lock().unwrap();
        *active_id = id_str.clone();
        
        let mut name_to_set = String::new();
        for i in 0..proxy_model_select.row_count() {
            if let Some(mut item) = proxy_model_select.row_data(i) {
                let is_match = item.id == id;
                item.is_active = is_match;
                if is_match {
                    name_to_set = item.name.to_string();
                }
                proxy_model_select.set_row_data(i, item);
            }
        }
        
        if let Some(u) = ui_select.upgrade() {
            u.set_active_config_name(name_to_set.into());
            // We do not auto-connect to allow the user to check dashboard first
        }
    });

    let profiles_delete = profiles.clone();
    let proxy_model_delete = proxy_model.clone();
    ui.on_delete_proxy(move |id| {
        let id_str = id.to_string();
        let mut p_guard = profiles_delete.lock().unwrap();
        p_guard.retain(|p| p.id != id_str);
        config::save_profiles(&p_guard);
        
        // Remove from UI model
        for i in 0..proxy_model_delete.row_count() {
            if let Some(item) = proxy_model_delete.row_data(i) {
                if item.id == id_str {
                    proxy_model_delete.remove(i);
                    break;
                }
            }
        }
    });

    // 4.5. Subscriptions Management
    let subs_add = subs.clone();
    let sub_model_add = sub_model.clone();
    ui.on_add_subscription(move |url| {
        let url_str = url.to_string();
        if url_str.is_empty() { return; }
        
        let id = uuid::Uuid::new_v4().to_string();
        let sub = config::Subscription {
            id: id.clone(),
            url: url_str.clone(),
            last_updated: "Never".to_string(),
        };
        
        let mut s_guard = subs_add.lock().unwrap();
        s_guard.push(sub);
        config::save_subscriptions(&s_guard);
        
        sub_model_add.push(SubItem {
            id: id.into(),
            url: url_str.into(),
            last_updated: "Never".into(),
        });
    });

    let subs_del = subs.clone();
    let sub_model_del = sub_model.clone();
    ui.on_delete_subscription(move |id| {
        let id_str = id.to_string();
        let mut s_guard = subs_del.lock().unwrap();
        s_guard.retain(|s| s.id != id_str);
        config::save_subscriptions(&s_guard);
        
        for i in 0..sub_model_del.row_count() {
            if let Some(item) = sub_model_del.row_data(i) {
                if item.id == id_str {
                    sub_model_del.remove(i);
                    break;
                }
            }
        }
    });

    let subs_update = subs.clone();
    let profiles_update = profiles.clone();
    let _proxy_model_update = proxy_model.clone();
    let _sub_model_update = sub_model.clone();
    let ui_update = ui.as_weak();
    
    ui.on_update_subscriptions(move |use_proxy| {
        let ui_weak = ui_update.clone();
        if let Some(u) = ui_weak.upgrade() {
            u.set_is_updating_subs(true);
        }
        
        let subs_clone = subs_update.clone();
        let profiles_clone = profiles_update.clone();
        
        tokio::spawn(async move {
            let urls: Vec<String> = {
                let guard = subs_clone.lock().unwrap();
                guard.iter().map(|s| s.url.clone()).collect()
            };
            
            let mut new_profiles = Vec::new();
            use base64::Engine;
            
            let client = if use_proxy {
                let proxy = reqwest::Proxy::all("socks5h://127.0.0.1:10808").unwrap();
                reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(std::time::Duration::from_secs(10))
                    .build().unwrap_or_else(|_| reqwest::Client::new())
            } else {
                reqwest::Client::new()
            };
            
            for url in urls {
                if let Ok(resp) = client.get(&url).send().await {
                    if let Ok(text) = resp.text().await {
                        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(text.trim()) {
                            if let Ok(decoded_str) = String::from_utf8(decoded) {
                                for line in decoded_str.lines() {
                                    let line = line.trim();
                                    if line.is_empty() { continue; }
                                    
                                    if let Some(parsed) = config::ProxyConfig::parse(line) {
                                        let id = uuid::Uuid::new_v4().to_string();
                                        let name = if let Some(idx) = line.find('#') {
                                            line[idx+1..].to_string()
                                        } else {
                                            parsed.hostname.clone()
                                        };
                                        
                                        new_profiles.push(config::Profile {
                                            id,
                                            name: format!("[Sub] {}", name),
                                            protocol: parsed.protocol,
                                            raw_link: line.to_string(),
                                            sub_group: url.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            {
                let mut p_guard = profiles_clone.lock().unwrap();
                let mut added = false;
                for p in new_profiles {
                    if !p_guard.iter().any(|existing| existing.raw_link == p.raw_link) {
                        p_guard.push(p.clone());
                        added = true;
                    }
                }
                config::save_profiles(&p_guard);
                
                if added {
                    let profiles_copy = p_guard.clone();
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let new_model = std::rc::Rc::new(slint::VecModel::default());
                            for p in &profiles_copy {
                                let mut address = String::new();
                                let mut port = String::new();
                                let mut transport = String::new();
                                let mut tls = String::new();
                                
                                if let Some(parsed) = config::ProxyConfig::parse(&p.raw_link) {
                                    address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
                                    port = parsed.port.to_string();
                                    transport = parsed.transport;
                                    tls = parsed.tls;
                                }
                                
                                new_model.push(ProxyItem {
                                    id: p.id.clone().into(),
                                    name: p.name.clone().into(),
                                    protocol: p.protocol.to_uppercase().into(),
                                    address: address.into(),
                                    port: port.into(),
                                    transport: transport.into(),
                                    tls: tls.into(),
                                    sub_group: p.sub_group.clone().into(),
                                    is_active: false,
                                    latency: "-".into(),
                                    latency_color: slint::Color::from_rgb_u8(136, 136, 136),
                                });
                            }
                            u.set_proxy_list(slint::ModelRc::new(new_model));
                            
                            let mut groups = std::collections::HashSet::new();
                            for p in &profiles_copy {
                                groups.insert(p.sub_group.clone());
                            }
                            let mut groups_vec: Vec<String> = groups.into_iter().collect();
                            groups_vec.sort();
                            groups_vec.retain(|g| g != "All");
                            groups_vec.insert(0, "All".to_string());
                            let slint_groups: std::rc::Rc<slint::VecModel<slint::SharedString>> = std::rc::Rc::new(slint::VecModel::from(
                                groups_vec.into_iter().map(|s| s.into()).collect::<Vec<slint::SharedString>>()
                            ));
                            u.set_subscription_groups(slint::ModelRc::new(slint_groups));
                        }
                    }).unwrap();
                }
            }
            
            let now = chrono::Local::now().format("%H:%M").to_string();
            {
                let mut s_guard = subs_clone.lock().unwrap();
                for s in s_guard.iter_mut() {
                    s.last_updated = now.clone();
                }
                config::save_subscriptions(&s_guard);
                
                let subs_copy = s_guard.clone();
                let ui_weak_clone = ui_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(u) = ui_weak_clone.upgrade() {
                        let new_model = std::rc::Rc::new(slint::VecModel::default());
                        for s in subs_copy {
                            new_model.push(SubItem {
                                id: s.id.into(),
                                url: s.url.into(),
                                last_updated: s.last_updated.into(),
                            });
                        }
                        u.set_subscription_list(slint::ModelRc::new(new_model));
                    }
                }).unwrap();
            }
            
            slint::invoke_from_event_loop(move || {
                if let Some(u) = ui_weak.upgrade() {
                    u.set_is_updating_subs(false);
                }
            }).unwrap();
        });
    });

    // 5. Autopilot
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
                    if tester::real_delay(10808).await.is_none() {
                        let ui_weak_clone = ui_weak.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak_clone.upgrade() {
                                if u.get_connected() {
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

    // 6. Connect / Disconnect logic
    let profiles_connect = profiles.clone();
    let active_id_connect = active_config_id.clone();
    let app_settings_connect = app_settings.clone();
    
    ui.on_toggle_connect(move || {
        let ui = ui_handle.unwrap();
        let currently_connected = ui.get_connected();
        
        if currently_connected {
            ui.set_connected(false);
            ui.set_current_ip("N/A".into());
            ui.set_latency("-".into());
            
            let runner_clone = proxy_runner.clone();
            tokio::spawn(async move {
                let mut runner = runner_clone.lock().await;
                runner.stop().await;
                let _ = crate::sysproxy::disable_system_proxy();
            });
        } else {
            let active_id = active_id_connect.lock().unwrap().clone();
            if active_id.is_empty() { return; }
            
            let p_guard = profiles_connect.lock().unwrap();
            let profile_opt = p_guard.iter().find(|p| p.id == active_id).cloned();
            drop(p_guard);
            
            if let Some(profile) = profile_opt {
                if let Some(parsed_cfg) = config::ProxyConfig::parse(&profile.raw_link) {
                    let ui_clone = ui.as_weak();
                    let runner_clone = proxy_runner.clone();
                    
                    ui.set_current_ip("Connecting...".into());
                    
                    let app_settings_spawn = app_settings_connect.clone();
                    
                    tokio::spawn(async move {
                        let mut final_ip = parsed_cfg.addresses[0].clone();
                        let mut best_latency = std::time::Duration::from_secs(999);
                        
                        let is_scanning = parsed_cfg.addresses.len() == 1 && parsed_cfg.addresses[0] == "scan.cfl";
                        
                        if is_scanning {
                            slint::invoke_from_event_loop({
                                let ui_weak = ui_clone.clone();
                                move || {
                                    if let Some(u) = ui_weak.upgrade() {
                                        u.set_is_scanning(true);
                                    }
                                }
                            }).unwrap();
                            
                            let ips = scanner::get_common_cf_ips();
                            for ip in ips {
                                let res = scanner::test_ip(&ip, parsed_cfg.port, 2000).await;
                                if res.is_valid && res.latency < best_latency {
                                    final_ip = ip;
                                    best_latency = res.latency;
                                }
                            }
                            
                            slint::invoke_from_event_loop({
                                let ui_weak = ui_clone.clone();
                                move || {
                                    if let Some(u) = ui_weak.upgrade() {
                                        u.set_is_scanning(false);
                                    }
                                }
                            }).unwrap();
                        } else if parsed_cfg.addresses.len() > 1 {
                            for ip in &parsed_cfg.addresses {
                                let res = scanner::test_ip(ip, parsed_cfg.port, 2000).await;
                                if res.latency < best_latency {
                                    final_ip = ip.clone();
                                    best_latency = res.latency;
                                }
                            }
                        }
                        
                        let settings_copy = app_settings_spawn.lock().unwrap().clone();
                        let xray_cfg = proxy::generate_xray_config(
                            &parsed_cfg.protocol,
                            &final_ip,
                            parsed_cfg.port,
                            &parsed_cfg.uuid,
                            &parsed_cfg.sni,
                            &parsed_cfg.hostname,
                            &parsed_cfg.path,
                            &settings_copy
                        );
                        
                        let mut runner = runner_clone.lock().await;
                        if let Err(e) = runner.start(xray_cfg).await {
                            eprintln!("Error starting proxy: {}", e);
                            slint::invoke_from_event_loop(move || {
                                if let Some(u) = ui_clone.upgrade() {
                                    u.set_connected(false);
                                    u.set_current_ip("Error Starting Xray".into());
                                }
                            }).unwrap();
                            return;
                        }
                        
                        let _ = crate::sysproxy::enable_system_proxy(settings_copy.socks_port + 1);
                        
                        let latency_str = if best_latency.as_millis() < 999000 {
                            format!("{}ms", best_latency.as_millis())
                        } else {
                            "-".to_string()
                        };

                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_clone.upgrade() {
                                u.set_connected(true);
                                u.set_current_ip(final_ip.into());
                                u.set_latency(latency_str.into());
                            }
                        }).unwrap();
                    });
                }
            }
        }
    });

    // 7. Advanced Config/Sub Suite Features
    
    let profiles_ping_all = profiles.clone();
    let ui_ping_all = ui.as_weak();
    ui.on_ping_all(move || {
        let profiles_copy = {
            let p_guard = profiles_ping_all.lock().unwrap();
            p_guard.iter().cloned().collect::<Vec<_>>()
        };
        let ui_weak = ui_ping_all.clone();
        
        tokio::spawn(async move {
            for (idx, profile) in profiles_copy.iter().enumerate() {
                if let Some(parsed) = config::ProxyConfig::parse(&profile.raw_link) {
                    if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                        let ip = parsed.addresses[0].clone();
                        let port = parsed.port;
                        let res = scanner::test_ip(&ip, port, 3000).await;
                        
                        let lat_str = if res.is_valid && res.latency.as_millis() < 3000 {
                            format!("{}ms", res.latency.as_millis())
                        } else {
                            "Timeout".to_string()
                        };
                        
                        let ui_weak_clone = ui_weak.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak_clone.upgrade() {
                                let list = u.get_proxy_list();
                                if let Some(mut item) = list.row_data(idx) {
                                    item.latency = lat_str.into();
                                    item.latency_color = if res.is_valid && res.latency.as_millis() < 150 {
                                        slint::Color::from_rgb_u8(16, 185, 129)
                                    } else if res.is_valid && res.latency.as_millis() < 300 {
                                        slint::Color::from_rgb_u8(245, 158, 11)
                                    } else if res.is_valid && res.latency.as_millis() < 3000 {
                                        slint::Color::from_rgb_u8(239, 68, 68)
                                    } else {
                                        slint::Color::from_rgb_u8(136, 136, 136)
                                    };
                                    list.set_row_data(idx, item);
                                }
                            }
                        }).unwrap();
                    }
                }
            }
        });
    });

    let profiles_del_all = profiles.clone();
    let proxy_model_del_all = proxy_model.clone();
    ui.on_delete_all_proxies(move || {
        {
            let mut p_guard = profiles_del_all.lock().unwrap();
            p_guard.clear();
            config::save_profiles(&p_guard);
        }
        
        while proxy_model_del_all.row_count() > 0 {
            proxy_model_del_all.remove(0);
        }
    });

    let profiles_ping_single = profiles.clone();
    let ui_ping_single = ui.as_weak();
    ui.on_ping_proxy(move |id| {
        let id_str = id.to_string();
        let profiles_copy = {
            let p_guard = profiles_ping_single.lock().unwrap();
            p_guard.iter().cloned().collect::<Vec<_>>()
        };
        
        if let Some((idx, profile)) = profiles_copy.iter().enumerate().find(|(_, p)| p.id == id_str) {
            if let Some(parsed) = config::ProxyConfig::parse(&profile.raw_link) {
                if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                    let ip = parsed.addresses[0].clone();
                    let port = parsed.port;
                    let ui_weak = ui_ping_single.clone();
                    
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak.upgrade() {
                            let list = u.get_proxy_list();
                            if let Some(mut item) = list.row_data(idx) {
                                item.latency = "Ping...".into();
                                item.latency_color = slint::Color::from_rgb_u8(136, 136, 136);
                                list.set_row_data(idx, item);
                            }
                        }
                    }).unwrap();
                    
                    let ui_weak2 = ui_ping_single.clone();
                    tokio::spawn(async move {
                        let res = scanner::test_ip(&ip, port, 3000).await;
                        let lat_str = if res.is_valid && res.latency.as_millis() < 3000 {
                            format!("{}ms", res.latency.as_millis())
                        } else {
                            "Timeout".to_string()
                        };
                        
                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak2.upgrade() {
                                let list = u.get_proxy_list();
                                if let Some(mut item) = list.row_data(idx) {
                                    item.latency = lat_str.into();
                                    item.latency_color = if res.is_valid && res.latency.as_millis() < 150 {
                                        slint::Color::from_rgb_u8(16, 185, 129)
                                    } else if res.is_valid && res.latency.as_millis() < 300 {
                                        slint::Color::from_rgb_u8(245, 158, 11)
                                    } else if res.is_valid && res.latency.as_millis() < 3000 {
                                        slint::Color::from_rgb_u8(239, 68, 68)
                                    } else {
                                        slint::Color::from_rgb_u8(136, 136, 136)
                                    };
                                    list.set_row_data(idx, item);
                                }
                            }
                        }).unwrap();
                    });
                }
            }
        }
    });

    let subs_update_single = subs.clone();
    let profiles_update_single = profiles.clone();
    let ui_update_single = ui.as_weak();
    
    ui.on_update_single_subscription(move |id| {
        let id_str = id.to_string();
        let subs_clone = subs_update_single.clone();
        let profiles_clone = profiles_update_single.clone();
        let ui_weak = ui_update_single.clone();
        
        let url = {
            let s_guard = subs_clone.lock().unwrap();
            s_guard.iter().find(|s| s.id == id_str).map(|s| s.url.clone())
        };
        
        if let Some(url) = url {
            tokio::spawn(async move {
                let mut new_profiles = Vec::new();
                use base64::Engine;
                
                if let Ok(resp) = reqwest::get(&url).await {
                    if let Ok(text) = resp.text().await {
                        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(text.trim()) {
                            if let Ok(decoded_str) = String::from_utf8(decoded) {
                                for line in decoded_str.lines() {
                                    let line = line.trim();
                                    if line.is_empty() { continue; }
                                    
                                    if let Some(parsed) = config::ProxyConfig::parse(line) {
                                        let pid = uuid::Uuid::new_v4().to_string();
                                        let name = if let Some(idx) = line.find('#') {
                                            line[idx+1..].to_string()
                                        } else {
                                            parsed.hostname.clone()
                                        };
                                        
                                        new_profiles.push(config::Profile {
                                            id: pid,
                                            name: format!("[Sub] {}", name),
                                            protocol: parsed.protocol,
                                            raw_link: line.to_string(),
                                            sub_group: url.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                
                {
                    let mut p_guard = profiles_clone.lock().unwrap();
                    let mut added = false;
                    for p in new_profiles {
                        if !p_guard.iter().any(|existing| existing.raw_link == p.raw_link) {
                            p_guard.push(p.clone());
                            added = true;
                        }
                    }
                    config::save_profiles(&p_guard);
                    
                    if added {
                        let profiles_copy = p_guard.clone();
                        let ui_weak_clone = ui_weak.clone();
                        slint::invoke_from_event_loop(move || {
                            if let Some(u) = ui_weak_clone.upgrade() {
                                let new_model = std::rc::Rc::new(slint::VecModel::default());
                                for p in &profiles_copy {
                                    let mut address = String::new();
                                    let mut port = String::new();
                                    let mut transport = String::new();
                                    let mut tls = String::new();
                                    
                                    if let Some(parsed) = config::ProxyConfig::parse(&p.raw_link) {
                                        address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
                                        port = parsed.port.to_string();
                                        transport = parsed.transport;
                                        tls = parsed.tls;
                                    }
                                    
                                    new_model.push(ProxyItem {
                                        id: p.id.clone().into(),
                                        name: p.name.clone().into(),
                                        protocol: p.protocol.to_uppercase().into(),
                                        address: address.into(),
                                        port: port.into(),
                                        transport: transport.into(),
                                        tls: tls.into(),
                                        sub_group: p.sub_group.clone().into(),
                                        is_active: false,
                                        latency: "-".into(),
                                        latency_color: slint::Color::from_rgb_u8(136, 136, 136),
                                    });
                                }
                                u.set_proxy_list(slint::ModelRc::new(new_model));
                                
                                let mut groups = std::collections::HashSet::new();
                                for p in &profiles_copy {
                                    groups.insert(p.sub_group.clone());
                                }
                                let mut groups_vec: Vec<String> = groups.into_iter().collect();
                                groups_vec.sort();
                                groups_vec.retain(|g| g != "All");
                                groups_vec.insert(0, "All".to_string());
                                let slint_groups: std::rc::Rc<slint::VecModel<slint::SharedString>> = std::rc::Rc::new(slint::VecModel::from(
                                    groups_vec.into_iter().map(|s| s.into()).collect::<Vec<slint::SharedString>>()
                                ));
                                u.set_subscription_groups(slint::ModelRc::new(slint_groups));
                            }
                        }).unwrap();
                    }
                }
                
                let now = chrono::Local::now().format("%H:%M").to_string();
                {
                    let mut s_guard = subs_clone.lock().unwrap();
                    for s in s_guard.iter_mut() {
                        if s.id == id_str {
                            s.last_updated = now.clone();
                        }
                    }
                    config::save_subscriptions(&s_guard);
                    
                    let subs_copy = s_guard.clone();
                    let ui_weak_clone = ui_weak.clone();
                    slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let new_model = std::rc::Rc::new(slint::VecModel::default());
                            for s in subs_copy {
                                new_model.push(SubItem {
                                    id: s.id.into(),
                                    url: s.url.into(),
                                    last_updated: s.last_updated.into(),
                                });
                            }
                            u.set_subscription_list(slint::ModelRc::new(new_model));
                        }
                    }).unwrap();
                }
            });
        }
    });

    // 8. Mock Network Metrics
    let ui_metrics = ui.as_weak();
    tokio::spawn(async move {
        let mut upload_total: u64 = 0;
        let mut download_total: u64 = 0;
        
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            
            let (up_speed_kb, dl_speed_kb) = {
                use rand::Rng;
                let mut rng = rand::thread_rng();
                (rng.gen_range(10..2500), rng.gen_range(50..15000))
            };
            
            upload_total += up_speed_kb as u64;
            download_total += dl_speed_kb as u64;
            
            let up_str = if up_speed_kb > 1024 {
                format!("{:.1} MB/s", up_speed_kb as f64 / 1024.0)
            } else {
                format!("{} KB/s", up_speed_kb)
            };
            
            let dl_str = if dl_speed_kb > 1024 {
                format!("{:.1} MB/s", dl_speed_kb as f64 / 1024.0)
            } else {
                format!("{} KB/s", dl_speed_kb)
            };
            
            let total_mb = (upload_total + download_total) / 1024;
            let total_str = if total_mb > 1024 {
                format!("{:.2} GB", total_mb as f64 / 1024.0)
            } else {
                format!("{} MB", total_mb)
            };
            
            slint::invoke_from_event_loop({
                let ui_weak = ui_metrics.clone();
                move || {
                    if let Some(u) = ui_weak.upgrade() {
                        if u.get_connected() {
                            u.set_upload_speed(up_str.into());
                            u.set_download_speed(dl_str.into());
                            u.set_total_data(total_str.into());
                            u.set_ip_country("Germany".into());
                        } else {
                            u.set_upload_speed("0 B/s".into());
                            u.set_download_speed("0 B/s".into());
                            u.set_ip_country("Unknown".into());
                        }
                    }
                }
            }).unwrap();
        }
    });

    // 9. Icon Suite Handlers
    let profiles_export = profiles.clone();
    ui.on_export_proxy(move |id| {
        let id_str = id.to_string();
        let p_guard = profiles_export.lock().unwrap();
        if let Some(p) = p_guard.iter().find(|x| x.id == id_str) {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(p.raw_link.clone());
            }
        }
    });

    let profiles_dup = profiles.clone();
    let proxy_model_dup = proxy_model.clone();
    ui.on_duplicate_proxy(move |id| {
        let id_str = id.to_string();
        let mut p_guard = profiles_dup.lock().unwrap();
        if let Some(p) = p_guard.iter().find(|x| x.id == id_str).cloned() {
            let mut new_p = p.clone();
            new_p.id = uuid::Uuid::new_v4().to_string();
            new_p.name = format!("{} (Copy)", p.name);
            p_guard.push(new_p.clone());
            config::save_profiles(&p_guard);
            
            let mut address = String::new();
            let mut port = String::new();
            let mut transport = String::new();
            let mut tls = String::new();
            
            if let Some(parsed) = config::ProxyConfig::parse(&new_p.raw_link) {
                address = parsed.addresses.first().unwrap_or(&"".to_string()).clone();
                port = parsed.port.to_string();
                transport = parsed.transport;
                tls = parsed.tls;
            }
            
            proxy_model_dup.push(ProxyItem {
                id: new_p.id.into(),
                name: new_p.name.into(),
                protocol: new_p.protocol.to_uppercase().into(),
                address: address.into(),
                port: port.into(),
                transport: transport.into(),
                tls: tls.into(),
                sub_group: new_p.sub_group.clone().into(),
                is_active: false,
                latency: "-".into(),
                latency_color: slint::Color::from_rgb_u8(136, 136, 136),
            });
        }
    });

    let profiles_qr = profiles.clone();
    let ui_qr = ui.as_weak();
    ui.on_show_qr_for_proxy(move |id| {
        let id_str = id.to_string();
        let p_guard = profiles_qr.lock().unwrap();
        if let Some(p) = p_guard.iter().find(|x| x.id == id_str) {
            use qrcode_generator::QrCodeEcc;
            if let Ok(svg) = qrcode_generator::to_svg_to_string(&p.raw_link, QrCodeEcc::Low, 200, None::<&str>) {
                if let Ok(img) = slint::Image::load_from_svg_data(svg.as_bytes()) {
                    if let Some(u) = ui_qr.upgrade() {
                        u.set_qr_image_data(img);
                        u.set_show_qr_modal(true);
                    }
                }
            }
        }
    });

    let ui_close_qr = ui.as_weak();
    ui.on_close_qr_modal(move || {
        if let Some(u) = ui_close_qr.upgrade() {
            u.set_show_qr_modal(false);
        }
    });

    let profiles_edit = profiles.clone();
    let ui_edit = ui.as_weak();
    ui.on_open_edit_proxy(move |id| {
        let id_str = id.to_string();
        let p_guard = profiles_edit.lock().unwrap();
        if let Some(p) = p_guard.iter().find(|x| x.id == id_str) {
            if let Some(u) = ui_edit.upgrade() {
                u.set_edit_proxy_id(p.id.clone().into());
                u.set_edit_proxy_name(p.name.clone().into());
                u.set_edit_proxy_link(p.raw_link.clone().into());
                u.set_show_edit_modal(true);
            }
        }
    });

    let ui_close_edit = ui.as_weak();
    ui.on_close_edit_modal(move || {
        if let Some(u) = ui_close_edit.upgrade() {
            u.set_show_edit_modal(false);
        }
    });

    let profiles_save = profiles.clone();
    let proxy_model_save = proxy_model.clone();
    let ui_save = ui.as_weak();
    ui.on_save_edit_proxy(move || {
        if let Some(u) = ui_save.upgrade() {
            let id = u.get_edit_proxy_id().to_string();
            let new_name = u.get_edit_proxy_name().to_string();
            let new_link = u.get_edit_proxy_link().to_string();
            
            let mut p_guard = profiles_save.lock().unwrap();
            if let Some(p) = p_guard.iter_mut().find(|x| x.id == id) {
                p.name = new_name.clone();
                p.raw_link = new_link.clone();
                if let Some(parsed) = config::ProxyConfig::parse(&new_link) {
                    p.protocol = parsed.protocol.clone();
                }
            }
            config::save_profiles(&p_guard);
            
            for i in 0..proxy_model_save.row_count() {
                if let Some(mut item) = proxy_model_save.row_data(i) {
                    if item.id == id {
                        item.name = new_name.clone().into();
                        if let Some(parsed) = config::ProxyConfig::parse(&new_link) {
                            item.protocol = parsed.protocol.into();
                        }
                        proxy_model_save.set_row_data(i, item);
                        break;
                    }
                }
            }
            
            u.set_show_edit_modal(false);
        }
    });

    ui.run()
}
