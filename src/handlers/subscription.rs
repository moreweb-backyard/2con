use crate::MainWindow;
use slint::Weak;
use std::time::Duration;
use tracing::{info, warn};

fn parse_subscription_content(content: &str) -> Vec<String> {
    let content = content.trim();
    
    if content.starts_with('[') {
        if let Ok(links) = serde_json::from_str::<Vec<String>>(content) {
            return links;
        }
    }
    
    if let Ok(decoded_bytes) = crate::config::robust_base64_decode(content) {
        if let Ok(decoded_str) = String::from_utf8(decoded_bytes) {
            return decoded_str.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
    }
    
    content.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

async fn fetch_subscription(client: &reqwest::Client, url: &str) -> Result<Vec<String>, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("HTTP request failed: {}", e))?;
    let text = resp.text().await.map_err(|e| format!("Failed to read response body: {}", e))?;
    let links = parse_subscription_content(&text);
    if links.is_empty() {
        return Err("No proxy links found in subscription response.".to_string());
    }
    Ok(links)
}

pub fn handle_add_subscription(ui_weak: &Weak<MainWindow>, state: &crate::state::AppState, url: String) {
    let url_str = url.trim().to_string();
    if url_str.is_empty() {
        return;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let sub = crate::config::Subscription {
        id: id.clone(),
        url: url_str.clone(),
        last_updated: "Never".to_string(),
    };

    {
        let mut s_guard = state.core.subs.lock().unwrap();
        s_guard.push(sub);
        crate::config::save_subscriptions(&s_guard);
    }

    state.refresh_ui_models();
    info!("Added subscription: {}", url_str);
}

pub fn handle_delete_subscription(ui_weak: &Weak<MainWindow>, state: &crate::state::AppState, id: String) {
    {
        let mut s_guard = state.core.subs.lock().unwrap();
        s_guard.retain(|s| s.id != id);
        crate::config::save_subscriptions(&s_guard);
    }

    state.refresh_ui_models();
    info!("Deleted subscription profile.");
}

pub fn handle_update_subscriptions(ui_weak: Weak<MainWindow>, state: crate::state::AppState, use_proxy: bool) {
    let ui_weak_clone = ui_weak.clone();
    let core_clone = state.core.clone();

    tokio::spawn(async move {
        let _ = slint::invoke_from_event_loop({
            let ui_weak = ui_weak_clone.clone();
            move || {
                if let Some(u) = ui_weak.upgrade() {
                    u.set_is_updating_subs(true);
                    u.set_sub_update_progress("Initializing...".into());
                }
            }
        });

        let subs_to_update = {
            let guard = core_clone.subs.lock().unwrap();
            guard.clone()
        };

        let total = subs_to_update.len();
        
        let client = if use_proxy {
            let port = core_clone.app_settings.lock().unwrap().socks_port;
            let proxy_url = format!("socks5h://127.0.0.1:{}", port);
            match reqwest::Proxy::all(&proxy_url) {
                Ok(proxy) => reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(Duration::from_secs(10))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new()),
                Err(_) => reqwest::Client::new()
            }
        } else {
            reqwest::Client::new()
        };

        let mut all_new_profiles = Vec::new();
        let now_time = chrono::Local::now().format("%H:%M").to_string();

        for (idx, mut sub) in subs_to_update.into_iter().enumerate() {
            let progress_text = format!("Updating ({}/{})", idx + 1, total);
            let ui_weak = ui_weak_clone.clone();
            let progress_text_clone = progress_text.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(u) = ui_weak.upgrade() {
                    u.set_sub_update_progress(progress_text_clone.into());
                }
            });

            match fetch_subscription(&client, &sub.url).await {
                Ok(links) => {
                    info!("Successfully fetched subscription: {}", sub.url);
                    sub.last_updated = format!("Success ({})", now_time);
                    
                    for link in links {
                        if let Some(parsed) = crate::config::ProxyConfig::parse(&link) {
                            let name = if let Some(h_idx) = link.find('#') {
                                crate::config::url_decode(&link[h_idx + 1..])
                            } else {
                                parsed.hostname.clone()
                            };

                            all_new_profiles.push(crate::config::Profile {
                                id: uuid::Uuid::new_v4().to_string(),
                                name: format!("[Sub] {}", name),
                                protocol: parsed.protocol,
                                raw_link: link.clone(),
                                sub_group: sub.url.clone(),
                            });
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch subscription {}: {}", sub.url, e);
                    sub.last_updated = format!("Failed ({})", now_time);
                    
                    let ui_weak = ui_weak_clone.clone();
                    let err_msg = format!("Subscription update failed: {}", e);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(u) = ui_weak.upgrade() {
                            let current = u.get_app_logs();
                            u.set_app_logs(format!("{}{}\n", current, err_msg).into());
                        }
                    });
                }
            }

            let mut subs_guard = core_clone.subs.lock().unwrap();
            if let Some(s) = subs_guard.iter_mut().find(|s| s.id == sub.id) {
                s.last_updated = sub.last_updated;
            }
            crate::config::save_subscriptions(&subs_guard);
        }

        {
            let mut p_guard = core_clone.profiles.lock().unwrap();
            let mut added = false;
            for p in all_new_profiles {
                if !p_guard.iter().any(|existing| existing.raw_link == p.raw_link) {
                    p_guard.push(p);
                    added = true;
                }
            }
            if added {
                crate::config::save_profiles(&p_guard);
            }
        }

        let core_ui_clone = core_clone.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(u) = ui_weak_clone.upgrade() {
                crate::state::refresh_ui_from_core(&u, &core_ui_clone);
                u.set_is_updating_subs(false);
                u.set_sub_update_progress("Done".into());
            }
        });
    });
}

pub fn handle_update_single_subscription(ui_weak: Weak<MainWindow>, state: crate::state::AppState, id: String) {
    let ui_weak_clone = ui_weak.clone();
    let core_clone = state.core.clone();

    tokio::spawn(async move {
        let (url, use_proxy, socks_port) = {
            let u_opt = {
                let s_guard = core_clone.subs.lock().unwrap();
                s_guard.iter().find(|s| s.id == id).map(|s| s.url.clone())
            };
            let url = match u_opt {
                Some(url) => url,
                None => return,
            };
            let (tx, rx) = tokio::sync::oneshot::channel();
            let ui_weak = ui_weak_clone.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let res = ui_weak.upgrade().map(|u| u.get_connected()).unwrap_or(false);
                let _ = tx.send(res);
            });
            let is_connected = rx.await.unwrap_or(false);
            
            let port = core_clone.app_settings.lock().unwrap().socks_port;
            (url, is_connected, port)
        };

        let client = if use_proxy {
            let proxy_url = format!("socks5h://127.0.0.1:{}", socks_port);
            match reqwest::Proxy::all(&proxy_url) {
                Ok(proxy) => reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(Duration::from_secs(10))
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new()),
                Err(_) => reqwest::Client::new()
            }
        } else {
            reqwest::Client::new()
        };

        let now_time = chrono::Local::now().format("%H:%M").to_string();
        let mut sub_status = String::new();
        let mut fetched_profiles = Vec::new();

        match fetch_subscription(&client, &url).await {
            Ok(links) => {
                info!("Successfully fetched single subscription: {}", url);
                sub_status = format!("Success ({})", now_time);
                
                for link in links {
                    if let Some(parsed) = crate::config::ProxyConfig::parse(&link) {
                        let name = if let Some(h_idx) = link.find('#') {
                            crate::config::url_decode(&link[h_idx + 1..])
                        } else {
                            parsed.hostname.clone()
                        };

                        fetched_profiles.push(crate::config::Profile {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: format!("[Sub] {}", name),
                            protocol: parsed.protocol,
                            raw_link: link.clone(),
                            sub_group: url.clone(),
                        });
                    }
                }
            }
            Err(e) => {
                warn!("Failed to fetch single subscription {}: {}", url, e);
                sub_status = format!("Failed ({})", now_time);
                
                let ui_weak = ui_weak_clone.clone();
                let err_msg = format!("Subscription update failed: {}", e);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(u) = ui_weak.upgrade() {
                        let current = u.get_app_logs();
                        u.set_app_logs(format!("{}{}\n", current, err_msg).into());
                    }
                });
            }
        }

        {
            let mut subs_guard = core_clone.subs.lock().unwrap();
            if let Some(s) = subs_guard.iter_mut().find(|s| s.id == id) {
                s.last_updated = sub_status;
            }
            crate::config::save_subscriptions(&subs_guard);
        }

        if !fetched_profiles.is_empty() {
            let mut p_guard = core_clone.profiles.lock().unwrap();
            let mut added = false;
            for p in fetched_profiles {
                if !p_guard.iter().any(|existing| existing.raw_link == p.raw_link) {
                    p_guard.push(p);
                    added = true;
                }
            }
            if added {
                crate::config::save_profiles(&p_guard);
            }
        }

        let core_ui_clone = core_clone.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(u) = ui_weak_clone.upgrade() {
                crate::state::refresh_ui_from_core(&u, &core_ui_clone);
            }
        });
    });
}
