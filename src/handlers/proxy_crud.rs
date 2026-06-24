use crate::MainWindow;
use crate::state::AppState;
use slint::{Weak, Color, Model};
use tracing::{info, error};

fn show_error(ui_weak: &Weak<MainWindow>, msg: String) {
    error!("{}", msg);
    let msg_clone = msg.clone();
    let ui_weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(u) = ui_weak.upgrade() {
            u.set_error_message(msg_clone.into());
            u.set_error_level("error".into());
        }
    });
}

fn is_valid_proxy_protocol(link: &str) -> bool {
    link.starts_with("vless://") || link.starts_with("vmess://") || link.starts_with("trojan://") || link.starts_with("ss://")
}

pub fn handle_import_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, mut link_str: String) {
    if link_str.is_empty() {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                match clipboard.get_text() {
                    Ok(text) => {
                        link_str = text.trim().to_string();
                    }
                    Err(e) => {
                        show_error(ui_weak, format!("Failed to read text from clipboard: {}", e));
                        return;
                    }
                }
            }
            Err(e) => {
                show_error(ui_weak, format!("Failed to access clipboard: {}", e));
                return;
            }
        }
    }
    
    if link_str.is_empty() {
        show_error(ui_weak, "Clipboard is empty.".to_string());
        return;
    }

    if !is_valid_proxy_protocol(&link_str) {
        show_error(ui_weak, "Unsupported or invalid proxy link protocol. Supported: vless, vmess, trojan, shadowsocks (ss)".to_string());
        return;
    }

    if let Some(parsed) = crate::config::ProxyConfig::parse(&link_str) {
        let id = uuid::Uuid::new_v4().to_string();
        
        let name = if let Some(idx) = link_str.find('#') {
            crate::config::url_decode(&link_str[idx + 1..])
        } else {
            parsed.hostname.clone()
        };

        let profile = crate::config::Profile {
            id: id.clone(),
            name: name.clone(),
            protocol: parsed.protocol.clone(),
            raw_link: link_str.clone(),
            sub_group: "Personal".to_string(),
        };

        {
            let mut p_guard = state.core.profiles.lock().unwrap();
            p_guard.push(profile);
            crate::config::save_profiles(&p_guard);
        }

        state.refresh_ui_models();
        
        info!("Imported proxy configuration: {}", name);
    } else {
        show_error(ui_weak, "Failed to parse proxy link. Please verify format.".to_string());
    }
}

pub fn handle_select_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    {
        let mut active_id = state.core.active_config_id.lock().unwrap();
        *active_id = id.clone();
    }
    
    {
        let mut settings = state.core.app_settings.lock().unwrap();
        settings.active_config_id = id.clone();
        crate::config::save_settings(&settings);
    }

    let mut name_to_set = String::new();
    for i in 0..state.proxy_model.row_count() {
        if let Some(mut item) = state.proxy_model.row_data(i) {
            let is_match = item.id == id;
            item.is_active = is_match;
            if is_match {
                name_to_set = item.name.to_string();
            }
            state.proxy_model.set_row_data(i, item);
        }
    }

    if let Some(u) = ui_weak.upgrade() {
        u.set_active_config_name(name_to_set.into());
    }
}

pub fn handle_delete_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    {
        let mut p_guard = state.core.profiles.lock().unwrap();
        p_guard.retain(|p| p.id != id);
        crate::config::save_profiles(&p_guard);
    }

    for i in 0..state.proxy_model.row_count() {
        if let Some(item) = state.proxy_model.row_data(i) {
            if item.id == id {
                state.proxy_model.remove(i);
                break;
            }
        }
    }
}

pub fn handle_delete_all_proxies(ui_weak: &Weak<MainWindow>, state: &AppState) {
    {
        let mut p_guard = state.core.profiles.lock().unwrap();
        p_guard.clear();
        crate::config::save_profiles(&p_guard);
    }

    while state.proxy_model.row_count() > 0 {
        state.proxy_model.remove(0);
    }
}

pub fn handle_ping_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    let profiles_copy = {
        let p_guard = state.core.profiles.lock().unwrap();
        p_guard.iter().cloned().collect::<Vec<_>>()
    };

    if let Some(profile) = profiles_copy.iter().find(|p| p.id == id) {
        if let Some(parsed) = crate::config::ProxyConfig::parse(&profile.raw_link) {
            if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                let ip = parsed.addresses[0].clone();
                let port = parsed.port;
                let ui_weak_clone = ui_weak.clone();
                let profile_id = profile.id.clone();
                
                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak_clone.clone();
                    let profile_id = profile_id.clone();
                    move || {
                        if let Some(u) = ui_weak.upgrade() {
                            let list = u.get_proxy_list();
                            for i in 0..list.row_count() {
                                if let Some(mut item) = list.row_data(i) {
                                    if item.id == profile_id {
                                        item.latency = "Ping...".into();
                                        item.latency_color = Color::from_rgb_u8(136, 136, 136);
                                        list.set_row_data(i, item);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });

                tokio::spawn(async move {
                    let res = crate::scanner::test_ip(&ip, port, 3000).await;
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
                                            Color::from_rgb_u8(16, 185, 129)
                                        } else if res.is_valid && res.latency.as_millis() < 300 {
                                            Color::from_rgb_u8(245, 158, 11)
                                        } else if res.is_valid && res.latency.as_millis() < 3000 {
                                            Color::from_rgb_u8(239, 68, 68)
                                        } else {
                                            Color::from_rgb_u8(136, 136, 136)
                                        };
                                        list.set_row_data(i, item);
                                        break;
                                    }
                                }
                            }
                        }
                    });
                });
            }
        }
    }
}

pub fn handle_ping_all(ui_weak: &Weak<MainWindow>, state: &AppState) {
    let profiles_copy = {
        let p_guard = state.core.profiles.lock().unwrap();
        p_guard.iter().cloned().collect::<Vec<_>>()
    };

    let ui_clone = ui_weak.clone();
    tokio::spawn(async move {
        for profile in profiles_copy {
            if let Some(parsed) = crate::config::ProxyConfig::parse(&profile.raw_link) {
                if !parsed.addresses.is_empty() && parsed.addresses[0] != "scan.cfl" {
                    let ip = parsed.addresses[0].clone();
                    let port = parsed.port;
                    let profile_id = profile.id.clone();
                    let ui_weak_clone = ui_clone.clone();

                    let res = crate::scanner::test_ip(&ip, port, 3000).await;
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
                                            Color::from_rgb_u8(16, 185, 129)
                                        } else if res.is_valid && res.latency.as_millis() < 300 {
                                            Color::from_rgb_u8(245, 158, 11)
                                        } else if res.is_valid && res.latency.as_millis() < 3000 {
                                            Color::from_rgb_u8(239, 68, 68)
                                        } else {
                                            Color::from_rgb_u8(136, 136, 136)
                                        };
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
}

pub fn handle_export_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    let p_guard = state.core.profiles.lock().unwrap();
    if let Some(p) = p_guard.iter().find(|x| x.id == id) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                let _ = clipboard.set_text(p.raw_link.clone());
                info!("Copied configuration link to clipboard.");
                let _ = slint::invoke_from_event_loop({
                    let ui_weak = ui_weak.clone();
                    move || {
                        if let Some(u) = ui_weak.upgrade() {
                            u.set_error_message("Copied link to clipboard!".into());
                            u.set_error_level("info".into());
                        }
                    }
                });
            }
            Err(e) => {
                show_error(ui_weak, format!("Failed to access clipboard: {}", e));
            }
        }
    }
}

pub fn handle_duplicate_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    let mut p_guard = state.core.profiles.lock().unwrap();
    if let Some(p) = p_guard.iter().find(|x| x.id == id).cloned() {
        let mut new_p = p.clone();
        new_p.id = uuid::Uuid::new_v4().to_string();
        new_p.name = format!("{} (Copy)", p.name);
        p_guard.push(new_p);
        crate::config::save_profiles(&p_guard);
        drop(p_guard);
        
        state.refresh_ui_models();
        info!("Duplicated config profile.");
    }
}

pub fn handle_show_qr_for_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    let p_guard = state.core.profiles.lock().unwrap();
    if let Some(p) = p_guard.iter().find(|x| x.id == id) {
        use qrcode_generator::QrCodeEcc;
        match qrcode_generator::to_svg_to_string(&p.raw_link, QrCodeEcc::Low, 200, None::<&str>) {
            Ok(svg) => {
                if let Ok(img) = slint::Image::load_from_svg_data(svg.as_bytes()) {
                    if let Some(u) = ui_weak.upgrade() {
                        u.set_qr_image_data(img);
                        u.set_show_qr_modal(true);
                    }
                }
            }
            Err(e) => {
                show_error(ui_weak, format!("Failed to generate QR code: {}", e));
            }
        }
    }
}

pub fn handle_open_edit_proxy(ui_weak: &Weak<MainWindow>, state: &AppState, id: String) {
    if id.is_empty() {
        if let Some(u) = ui_weak.upgrade() {
            u.set_edit_proxy_id("".into());
            u.set_edit_proxy_name("New Custom Proxy".into());
            u.set_edit_proxy_link("vless://...".into());
            u.set_show_edit_modal(true);
        }
    } else {
        let p_guard = state.core.profiles.lock().unwrap();
        if let Some(p) = p_guard.iter().find(|x| x.id == id) {
            if let Some(u) = ui_weak.upgrade() {
                u.set_edit_proxy_id(p.id.clone().into());
                u.set_edit_proxy_name(p.name.clone().into());
                u.set_edit_proxy_link(p.raw_link.clone().into());
                u.set_show_edit_modal(true);
            }
        }
    }
}

pub fn handle_save_edit_proxy(ui_weak: &Weak<MainWindow>, state: &AppState) {
    if let Some(u) = ui_weak.upgrade() {
        let id = u.get_edit_proxy_id().to_string();
        let new_name = u.get_edit_proxy_name().to_string();
        let new_link = u.get_edit_proxy_link().to_string();

        let mut protocol = "unknown".to_string();
        if let Some(parsed) = crate::config::ProxyConfig::parse(&new_link) {
            protocol = parsed.protocol;
        } else {
            show_error(ui_weak, "Invalid or unsupported proxy configuration link format.".to_string());
            return;
        }

        let mut p_guard = state.core.profiles.lock().unwrap();

        if id.is_empty() {
            let new_id = uuid::Uuid::new_v4().to_string();
            let new_profile = crate::config::Profile {
                id: new_id,
                name: new_name,
                protocol,
                raw_link: new_link,
                sub_group: "Custom".to_string(),
            };
            p_guard.push(new_profile);
        } else {
            if let Some(p) = p_guard.iter_mut().find(|x| x.id == id) {
                p.name = new_name;
                p.raw_link = new_link;
                p.protocol = protocol;
            }
        }

        crate::config::save_profiles(&p_guard);
        drop(p_guard);
        
        state.refresh_ui_models();
        u.set_show_edit_modal(false);
        info!("Saved proxy configuration.");
    }
}
