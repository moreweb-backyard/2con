use crate::{ProxyItem, SubItem};
use crate::config::{self, AppSettings, Profile, Subscription};
use crate::proxy::ProxyRunner;
use slint::{Model, VecModel, SharedString, Color};
use std::rc::Rc;
use std::sync::{Arc, Mutex as StdMutex};
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex as TokioMutex;

#[derive(Clone)]
pub struct AppCore {
    pub proxy_runner: Arc<TokioMutex<ProxyRunner>>,
    pub autopilot_active: Arc<AtomicBool>,
    pub app_settings: Arc<StdMutex<AppSettings>>,
    pub profiles: Arc<StdMutex<Vec<Profile>>>,
    pub subs: Arc<StdMutex<Vec<Subscription>>>,
    pub active_config_id: Arc<StdMutex<String>>,
}

#[derive(Clone)]
pub struct AppState {
    pub core: AppCore,
    pub proxy_model: Rc<VecModel<ProxyItem>>,
    pub sub_model: Rc<VecModel<SubItem>>,
    pub slint_groups: Rc<VecModel<SharedString>>,
}

impl AppState {
    pub fn new() -> Self {
        let app_settings = config::load_settings();
        let active_config_id = app_settings.active_config_id.clone();
        
        let core = AppCore {
            proxy_runner: Arc::new(TokioMutex::new(ProxyRunner::new())),
            autopilot_active: Arc::new(AtomicBool::new(false)),
            app_settings: Arc::new(StdMutex::new(app_settings)),
            profiles: Arc::new(StdMutex::new(config::load_profiles())),
            subs: Arc::new(StdMutex::new(config::load_subscriptions())),
            active_config_id: Arc::new(StdMutex::new(active_config_id)),
        };

        // Create UI models
        let proxy_model = Rc::new(VecModel::default());
        let sub_model = Rc::new(VecModel::default());
        let slint_groups = Rc::new(VecModel::default());

        let state = Self {
            core,
            proxy_model,
            sub_model,
            slint_groups,
        };

        state.refresh_ui_models();

        state
    }

    pub fn refresh_ui_models(&self) {
        while self.proxy_model.row_count() > 0 {
            self.proxy_model.remove(0);
        }
        let active_id = self.core.active_config_id.lock().unwrap().clone();
        
        let profiles_guard = self.core.profiles.lock().unwrap();
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

            self.proxy_model.push(ProxyItem {
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
                latency_color: Color::from_rgb_u8(136, 136, 136),
            });
        }

        while self.sub_model.row_count() > 0 {
            self.sub_model.remove(0);
        }
        let subs_guard = self.core.subs.lock().unwrap();
        for s in subs_guard.iter() {
            self.sub_model.push(SubItem {
                id: s.id.clone().into(),
                url: s.url.clone().into(),
                last_updated: s.last_updated.clone().into(),
            });
        }

        let mut groups = std::collections::HashSet::new();
        for p in profiles_guard.iter() {
            groups.insert(p.sub_group.clone());
        }
        let mut groups_vec: Vec<String> = groups.into_iter().collect();
        groups_vec.sort();
        groups_vec.retain(|g| g != "All");
        groups_vec.insert(0, "All".to_string());

        while self.slint_groups.row_count() > 0 {
            self.slint_groups.remove(0);
        }
        for g in groups_vec {
            self.slint_groups.push(g.into());
        }
    }
}

pub fn refresh_ui_from_core(u: &crate::MainWindow, core: &AppCore) {
    let active_id = core.active_config_id.lock().unwrap().clone();
    
    let new_proxy_model = std::rc::Rc::new(slint::VecModel::default());
    let profiles_guard = core.profiles.lock().unwrap();
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

        new_proxy_model.push(ProxyItem {
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
            latency_color: Color::from_rgb_u8(136, 136, 136),
        });
    }
    u.set_proxy_list(slint::ModelRc::new(new_proxy_model));

    let new_sub_model = std::rc::Rc::new(slint::VecModel::default());
    let subs_guard = core.subs.lock().unwrap();
    for s in subs_guard.iter() {
        new_sub_model.push(SubItem {
            id: s.id.clone().into(),
            url: s.url.clone().into(),
            last_updated: s.last_updated.clone().into(),
        });
    }
    u.set_subscription_list(slint::ModelRc::new(new_sub_model));

    let mut groups = std::collections::HashSet::new();
    for p in profiles_guard.iter() {
        groups.insert(p.sub_group.clone());
    }
    let mut groups_vec: Vec<String> = groups.into_iter().collect();
    groups_vec.sort();
    groups_vec.retain(|g| g != "All");
    groups_vec.insert(0, "All".to_string());

    let new_groups = std::rc::Rc::new(slint::VecModel::default());
    for g in groups_vec {
        new_groups.push(g.into());
    }
    u.set_subscription_groups(slint::ModelRc::new(new_groups));
}
