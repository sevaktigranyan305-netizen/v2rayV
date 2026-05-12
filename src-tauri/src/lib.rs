pub mod commands;
pub mod config;
pub mod models;
pub mod network;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod priv_xray;
pub mod proxy;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod secret_store;
pub mod storage;
pub mod subscription;
pub mod tray;
#[cfg(target_os = "linux")]
pub mod tun;
pub mod uri;
pub mod xray;

use tauri::Manager;
use xray::XrayManager;

/// Copy `wintun.dll` from the bundle resources into the directory of the
/// currently-running v2rayV.exe (== directory of xray.exe sidecar). The
/// xray-core wintun backend calls `LoadLibrary("wintun.dll")` which only
/// finds the DLL if it sits next to the calling executable.
///
/// Idempotent: if the destination already exists we leave it alone.
#[cfg(target_os = "windows")]
fn ensure_wintun_next_to_exe<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().ok_or("current_exe has no parent directory")?;
    let dst = exe_dir.join("wintun.dll");
    if dst.exists() {
        log::info!("wintun.dll already present at {}", dst.display());
        return Ok(());
    }

    let resource_path = app
        .path()
        .resolve("binaries/wintun.dll", tauri::path::BaseDirectory::Resource)?;
    if !resource_path.exists() {
        return Err(format!("wintun.dll resource missing at {}", resource_path.display()).into());
    }

    std::fs::copy(&resource_path, &dst)?;
    log::info!(
        "Staged wintun.dll: {} -> {}",
        resource_path.display(),
        dst.display()
    );
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            app.handle().plugin(tauri_plugin_shell::init())?;

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            // Windows: wintun.dll has to live next to xray.exe (the loader
            // searches the calling exe's directory first). Tauri ships it
            // as a bundle resource under <install_dir>/resources/binaries/,
            // so copy it once into <install_dir>/ if it isn't there yet.
            // Writing to Program Files works because our manifest forces
            // UAC elevation.
            #[cfg(target_os = "windows")]
            {
                if let Err(e) = ensure_wintun_next_to_exe(app.handle()) {
                    log::warn!("Failed to stage wintun.dll: {e}");
                }
            }

            // Clean up stale TUN from previous crash (Linux only)
            #[cfg(target_os = "linux")]
            {
                let config_dir = app
                    .handle()
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
                tun::cleanup_stale_tun(&config_dir);
            }

            // Reset system proxy if it's still pointing at our ports from a prior
            // session that didn't shut down cleanly. Must happen BEFORE auto-connect
            // (which would set it again) — otherwise apps would try to reach a dead
            // SOCKS proxy during the window between app start and VPN connect.
            proxy::reset_stale_system_proxy();

            app.manage(XrayManager::new());

            let handle = app.handle().clone();

            tray::setup_tray(&handle)?;

            // Hide to tray instead of closing. On macOS we additionally
            // demote the app to ActivationPolicy::Accessory so it
            // vanishes from the Dock and the menu bar (matching the
            // Windows behavior where the taskbar entry disappears when
            // the window closes). The tray's "Show Window" handler
            // and left-click handler flip the policy back to Regular
            // before showing the window again.
            let window = app.get_webview_window("main").unwrap();
            let window_clone = window.clone();
            #[cfg(target_os = "macos")]
            let close_handle = app.handle().clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window_clone.hide();
                    #[cfg(target_os = "macos")]
                    {
                        if let Err(e) =
                            close_handle.set_activation_policy(tauri::ActivationPolicy::Accessory)
                        {
                            log::warn!("Failed to set Accessory activation policy: {e}");
                        }
                    }
                }
            });

            let settings = storage::load_settings(&handle).unwrap_or_default();

            // Auto-connect on startup
            if settings.auto_connect {
                if let Some(ref server_id) = settings.last_server_id {
                    if let Ok(servers) = storage::load_servers(&handle) {
                        if let Some(server) = servers.iter().find(|s| s.id == *server_id) {
                            let manager = app.state::<XrayManager>();
                            if let Err(e) = manager.start(&handle, server, &settings.bypass_domains)
                            {
                                log::warn!("Auto-connect failed: {e}");
                            }
                        }
                    }
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::get_status,
            commands::get_connection_info,
            commands::test_connection,
            commands::get_socks_port,
            commands::validate_config,
            commands::get_servers,
            commands::add_server,
            commands::update_server,
            commands::delete_server,
            commands::export_servers,
            commands::import_servers,
            commands::list_subscriptions,
            commands::add_subscription,
            commands::refresh_subscription,
            commands::delete_subscription,
            commands::get_speed_stats,
            commands::get_logs,
            commands::clear_logs,
            commands::get_settings,
            commands::update_settings,
            commands::apply_bypass_domains,
            uri::parse_vless_uri_cmd,
            uri::export_vless_uri,
            commands::detect_vpn_interfaces,
            commands::has_sudo_password,
            commands::store_sudo_password,
            commands::clear_sudo_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
