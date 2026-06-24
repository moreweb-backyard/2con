use crate::MainWindow;
use crate::state::AppState;
use slint::{Weak, ComponentHandle};


#[cfg(target_os = "windows")]
use i_slint_backend_winit::WinitWindowAccessor;

pub fn handle_settings_changed(ui_weak: &Weak<MainWindow>, state: &AppState) {
    if let Some(u) = ui_weak.upgrade() {
        let mut s_guard = state.core.app_settings.lock().unwrap();
        s_guard.socks_port = u.get_socks_port().to_string().parse().unwrap_or(10808);
        s_guard.mux_enabled = u.get_mux_enabled();
        s_guard.log_level = u.get_log_level().to_string();

        s_guard.enable_udp = u.get_enable_udp();
        s_guard.enable_sniffing = u.get_enable_sniffing();
        s_guard.allow_lan = u.get_allow_lan();
        s_guard.enable_fragment = u.get_enable_fragment();

        s_guard.bypass_list = u.get_bypass_list().to_string();

        s_guard.domestic_dns = u.get_domestic_dns().to_string();
        s_guard.remote_dns = u.get_remote_dns().to_string();
        s_guard.bootstrap_dns = u.get_bootstrap_dns().to_string();
        s_guard.enable_fakeip = u.get_enable_fakeip();
        s_guard.block_svcb = u.get_block_svcb();
        s_guard.add_common_dns = u.get_add_common_dns();
        s_guard.dns_hosts = u.get_dns_hosts().to_string();
        s_guard.custom_dns_json = u.get_custom_dns_json().to_string();

        let start_on_boot_changed = s_guard.start_on_boot != u.get_start_on_boot();
        s_guard.start_on_boot = u.get_start_on_boot();
        if start_on_boot_changed {
            if s_guard.start_on_boot {
                let _ = crate::sysproxy::enable_auto_start();
            } else {
                let _ = crate::sysproxy::disable_auto_start();
            }
        }
        s_guard.auto_update_geo = u.get_auto_update_geo().to_string().parse().unwrap_or(0);
        
        s_guard.auto_connect = u.get_auto_connect();

        let docking = u.get_docking_position().to_string();
        let docking_changed = s_guard.docking != docking;
        s_guard.docking = docking.clone();

        crate::config::save_settings(&s_guard);

        #[cfg(target_os = "windows")]
        if docking_changed && docking != "None" {
            u.window().with_winit_window(|w| {
                if let Some(monitor) = w.current_monitor() {
                    let size = monitor.size();
                    let w_size = w.outer_size();
                    let pos = match docking.as_str() {
                        "Top Left" => {
                            i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0)
                        }
                        "Top Right" => {
                            i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                                size.width.saturating_sub(w_size.width),
                                0,
                            )
                        }
                        "Bottom Left" => {
                            i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                                0,
                                size.height.saturating_sub(w_size.height),
                            )
                        }
                        "Bottom Right" => {
                            i_slint_backend_winit::winit::dpi::PhysicalPosition::new(
                                size.width.saturating_sub(w_size.width),
                                size.height.saturating_sub(w_size.height),
                            )
                        }
                        _ => i_slint_backend_winit::winit::dpi::PhysicalPosition::new(0, 0),
                    };
                    w.set_outer_position(pos);
                }
            });
        }
    }
}
