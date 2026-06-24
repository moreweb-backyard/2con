mod config;
mod proxy;
mod scanner;
mod sysproxy;
mod tester;
mod error;
mod state;
mod handlers;

slint::include_modules!();

#[cfg(target_os = "windows")]
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Model, ModelRc};
use state::AppState;
use tracing::info;

fn cleanup_orphaned_processes() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(&["/F", "/IM", "xray.exe"])
            .output();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("killall")
            .arg("xray")
            .output();
    }
}

#[tokio::main]
async fn main() -> Result<(), slint::PlatformError> {
    // 0. Cleanup orphaned processes from previous crashes
    cleanup_orphaned_processes();

    // 1. Initialize State
    let app_state = AppState::new();

    // 2. Initialize Logging
    let log_level = {
        let guard = app_state.core.app_settings.lock().unwrap();
        guard.log_level.clone()
    };
    let filter_level = match log_level.to_lowercase().as_str() {
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "error" => tracing::Level::ERROR,
        "none" => tracing::Level::ERROR,
        _ => tracing::Level::WARN,
    };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(filter_level)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    info!("2con client starting...");

    // 3. Initialize Slint UI
    let ui = MainWindow::new()?;
    
    // Set UI platform helper
    ui.set_is_windows(cfg!(target_os = "windows"));

    // Populate Settings UI Properties
    {
        let s_guard = app_state.core.app_settings.lock().unwrap();
        ui.set_socks_port(s_guard.socks_port.to_string().into());
        ui.set_mux_enabled(s_guard.mux_enabled);
        ui.set_log_level(s_guard.log_level.clone().into());
        ui.set_docking_position(s_guard.docking.clone().into());

        ui.set_enable_udp(s_guard.enable_udp);
        ui.set_enable_sniffing(s_guard.enable_sniffing);
        ui.set_allow_lan(s_guard.allow_lan);
        ui.set_enable_fragment(s_guard.enable_fragment);

        ui.set_bypass_list(s_guard.bypass_list.clone().into());

        ui.set_domestic_dns(s_guard.domestic_dns.clone().into());
        ui.set_remote_dns(s_guard.remote_dns.clone().into());
        ui.set_bootstrap_dns(s_guard.bootstrap_dns.clone().into());
        ui.set_enable_fakeip(s_guard.enable_fakeip);
        ui.set_block_svcb(s_guard.block_svcb);
        ui.set_add_common_dns(s_guard.add_common_dns);
        ui.set_dns_hosts(s_guard.dns_hosts.clone().into());
        ui.set_custom_dns_json(s_guard.custom_dns_json.clone().into());

        ui.set_start_on_boot(s_guard.start_on_boot);
        ui.set_auto_update_geo(s_guard.auto_update_geo.to_string().into());
        
        ui.set_auto_connect(s_guard.auto_connect);
    }

    // Set Active Config Name
    {
        let active_id = app_state.core.active_config_id.lock().unwrap().clone();
        let mut name_to_set = "Select Configuration".to_string();
        for i in 0..app_state.proxy_model.row_count() {
            if let Some(item) = app_state.proxy_model.row_data(i) {
                if item.id == active_id {
                    name_to_set = item.name.to_string();
                    break;
                }
            }
        }
        ui.set_active_config_name(name_to_set.into());
    }

    // Bind UI models
    ui.set_proxy_list(ModelRc::new(app_state.proxy_model.clone()));
    ui.set_subscription_list(ModelRc::new(app_state.sub_model.clone()));
    ui.set_subscription_groups(ModelRc::new(app_state.slint_groups.clone()));

    // Validation Callbacks
    ui.on_is_valid_proxy_link(|link| {
        let link_str = link.to_string();
        !link_str.is_empty()
            && (link_str.starts_with("vless://")
                || link_str.starts_with("vmess://")
                || link_str.starts_with("trojan://")
                || link_str.starts_with("ss://"))
    });

    ui.on_is_valid_sub_link(|link| {
        let link_str = link.to_string();
        !link_str.is_empty()
            && (link_str.starts_with("http://") || link_str.starts_with("https://"))
    });

    // 4. Register Event Callback Handlers
    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_toggle_connect(move || {
        handlers::connection::handle_toggle_connect(ui_weak.clone(), state_clone.core.clone());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_set_autopilot(move |enabled| {
        handlers::connection::handle_set_autopilot(&ui_weak, &state_clone.core, enabled);
    });

    let ui_weak = ui.as_weak();
    ui.on_clear_logs(move || {
        handlers::connection::handle_clear_logs(&ui_weak);
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_import_proxy(move |link| {
        handlers::proxy_crud::handle_import_proxy(&ui_weak, &state_clone, link.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_select_proxy(move |id| {
        handlers::proxy_crud::handle_select_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_delete_proxy(move |id| {
        handlers::proxy_crud::handle_delete_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_delete_all_proxies(move || {
        handlers::proxy_crud::handle_delete_all_proxies(&ui_weak, &state_clone);
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_ping_proxy(move |id| {
        handlers::proxy_crud::handle_ping_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_ping_all(move || {
        handlers::proxy_crud::handle_ping_all(&ui_weak, &state_clone);
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_export_proxy(move |id| {
        handlers::proxy_crud::handle_export_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_duplicate_proxy(move |id| {
        handlers::proxy_crud::handle_duplicate_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_show_qr_for_proxy(move |id| {
        handlers::proxy_crud::handle_show_qr_for_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_open_edit_proxy(move |id| {
        handlers::proxy_crud::handle_open_edit_proxy(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_save_edit_proxy(move || {
        handlers::proxy_crud::handle_save_edit_proxy(&ui_weak, &state_clone);
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_add_subscription(move |url| {
        handlers::subscription::handle_add_subscription(&ui_weak, &state_clone, url.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_delete_subscription(move |id| {
        handlers::subscription::handle_delete_subscription(&ui_weak, &state_clone, id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_update_subscriptions(move |use_proxy| {
        handlers::subscription::handle_update_subscriptions(ui_weak.clone(), state_clone.clone(), use_proxy);
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_update_single_subscription(move |id| {
        handlers::subscription::handle_update_single_subscription(ui_weak.clone(), state_clone.clone(), id.to_string());
    });

    let ui_weak = ui.as_weak();
    let state_clone = app_state.clone();
    ui.on_settings_changed(move || {
        handlers::settings::handle_settings_changed(&ui_weak, &state_clone);
    });

    // Window Callbacks
    let core_close = app_state.core.clone();
    ui.on_close_window(move || {
        let core = core_close.clone();
        tokio::spawn(async move {
            let _ = handlers::connection::perform_disconnect(&core).await;
            std::process::exit(0);
        });
        let _ = slint::quit_event_loop();
    });

    let ui_drag = ui.as_weak();
    ui.on_drag_window(move || {
        if let Some(u) = ui_drag.upgrade() {
            #[cfg(target_os = "windows")]
            u.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });

    let ui_close_qr = ui.as_weak();
    ui.on_close_qr_modal(move || {
        if let Some(u) = ui_close_qr.upgrade() {
            u.set_show_qr_modal(false);
        }
    });

    let ui_close_edit = ui.as_weak();
    ui.on_close_edit_modal(move || {
        if let Some(u) = ui_close_edit.upgrade() {
            u.set_show_edit_modal(false);
        }
    });

    let proxy_model_filter = app_state.proxy_model.clone();
    let profiles_filter = app_state.core.profiles.clone();
    let ui_filter = ui.as_weak();
    let active_id_filter = app_state.core.active_config_id.clone();
    ui.on_filter_changed(move || {
        if let Some(u) = ui_filter.upgrade() {
            let filter_group = u.get_filter_group().to_string();
            let profiles_guard = profiles_filter.lock().unwrap();
            let active_id = active_id_filter.lock().unwrap().clone();

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
                        is_active: p.id == active_id,
                        latency: "Ping...".into(),
                        latency_color: slint::Color::from_rgb_u8(136, 136, 136),
                    });
                }
            }

            proxy_model_filter.set_vec(items);
        }
    });

    // Download rules helper
    ui.on_download_routing_rules(move || {
        tokio::spawn(async move {
            let _ = proxy::download_routing_rules().await;
        });
    });

    // 5. Apply Initial Docking Position (Windows Only)
    #[cfg(target_os = "windows")]
    {
        let docking = app_state.core.app_settings.lock().unwrap().docking.clone();
        if docking != "None" {
            ui.window().with_winit_window(|w| {
                if let Some(monitor) = w.current_monitor() {
                    let size = monitor.size();
                    let w_size = w.outer_size();
                    let pos = match docking.as_str() {
                        "Top Left" => {
                            i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0)
                        }
                        "Top Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                            size.width.saturating_sub(w_size.width),
                            0,
                        ),
                        "Bottom Left" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                            0,
                            size.height.saturating_sub(w_size.height),
                        ),
                        "Bottom Right" => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                            size.width.saturating_sub(w_size.width),
                            size.height.saturating_sub(w_size.height),
                        ),
                        _ => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                    };
                    w.set_outer_position(pos);
                }
            });
        }
    }

    // 6. Graceful Shutdown Signal Registration (Ctrl+C)
    let core_ctrlc = app_state.core.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("Received shutdown signal. Stopping services...");
            let _ = handlers::connection::perform_disconnect(&core_ctrlc).await;
            std::process::exit(0);
        }
    });

    // 7. Auto-Ping loaded profiles on startup
    let ui_ping = ui.as_weak();
    let state_ping = app_state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        let profiles_copy = {
            let p_guard = state_ping.core.profiles.lock().unwrap();
            p_guard.clone()
        };

        for profile in profiles_copy {
            if let Some(parsed) = config::ProxyConfig::parse(&profile.raw_link) {
                if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                    let ip = parsed.addresses[0].clone();
                    let port = parsed.port;
                    let profile_id = profile.id.clone();
                    let ui_weak_clone = ui_ping.clone();

                    let res = scanner::test_ip(&ip, port, 3000).await;
                    let lat_str = if res.is_valid && res.latency.as_millis() < 3000 {
                        format!("{}ms", res.latency.as_millis())
                    } else {
                        "Timeout".to_string()
                    };

                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let list = u.get_proxy_list();
                            for i in 0..list.row_count() {
                                if let Some(mut item) = list.row_data(i) {
                                    if item.id == profile_id {
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
                                        list.set_row_data(i, item);
                                        break;
                                    }
                                }
                            }
                        }
                    });
                } else if !parsed.addresses.is_empty() && parsed.addresses[0] == "scan.cfl" {
                    let profile_id = profile.id.clone();
                    let ui_weak_clone = ui_ping.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak_clone.upgrade() {
                            let list = u.get_proxy_list();
                            for i in 0..list.row_count() {
                                if let Some(mut item) = list.row_data(i) {
                                    if item.id == profile_id {
                                        item.latency = "Dynamic".into();
                                        item.latency_color = slint::Color::from_rgb_u8(0, 229, 255);
                                        list.set_row_data(i, item);
                                        break;
                                    }
                                }
                            }
                        }
                    });
                }
            }
        }
    });

    // 8. Trigger Auto-Connect on Startup if enabled
    let auto_connect = app_state.core.app_settings.lock().unwrap().auto_connect;
    let active_id = app_state.core.active_config_id.lock().unwrap().clone();
    if auto_connect && !active_id.is_empty() {
        let ui_weak = ui.as_weak();
        let core_clone = app_state.core.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
            handlers::connection::handle_toggle_connect(ui_weak, core_clone);
        });
    }

    ui.run()
}
