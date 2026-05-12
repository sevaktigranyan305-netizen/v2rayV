use log::warn;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime};

use crate::models::ConnectionStatus;
use crate::storage;
use crate::xray::XrayManager;

pub fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let toggle_item = MenuItem::with_id(app, "toggle_connection", "Connect", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_item, &toggle_item, &quit_item])?;

    let toggle_clone = toggle_item.clone();
    app.listen("connection-status-changed", move |event| {
        let payload = event.payload();
        if payload.contains("connected") && !payload.contains("disconnected") {
            let _ = toggle_clone.set_text("Disconnect");
        } else {
            let _ = toggle_clone.set_text("Connect");
        }
    });

    // Tauri v2's TrayIconBuilder does NOT auto-derive an icon from the
    // bundle config; without an explicit .icon() the tray entry renders
    // as a blank slot on Windows (notification area), and as an empty
    // placeholder on macOS (menu bar) and StatusNotifierItem-based Linux
    // DEs. Pull the icon Tauri loaded from `tauri.conf.json:bundle.icon`
    // at startup so the tray graphic stays in sync with the window icon.
    #[cfg(not(target_os = "macos"))]
    let icon = app
        .default_window_icon()
        .ok_or("default window icon not configured in tauri.conf.json")?
        .clone();

    // macOS menubar icons should follow Apple's "template image"
    // convention: a monochrome glyph with transparency that the system
    // auto-tints to match the menubar appearance (white in dark mode,
    // black in light mode, plus the highlight tint when the menu is
    // open). Reusing the colorful Dock/Windows icon here makes the
    // tray entry stand out awkwardly versus the rest of the menubar
    // and ignores the user's light/dark preference. The PNG is
    // pre-rendered from the same paw SVG and embedded at compile time.
    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-mac-22@2x.png"))?;

    #[cfg(target_os = "macos")]
    let builder = TrayIconBuilder::new().icon(icon).icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = TrayIconBuilder::new().icon(icon);

    builder
        .menu(&menu)
        .tooltip("v2rayV")
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => {
                show_main_window(app);
            }
            "toggle_connection" => {
                handle_toggle_connection(app);
            }
            "quit" => {
                let manager = app.state::<XrayManager>();
                let _ = manager.stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Show (or restore) the main window from the tray. On macOS the
/// window-close handler in `lib::run` demotes the app to
/// `ActivationPolicy::Accessory` to hide it from the Dock; flip back
/// to `Regular` here so the Dock icon reappears and the window can
/// receive keyboard focus the moment it becomes visible.
fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
            warn!("Failed to set Regular activation policy: {e}");
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn handle_toggle_connection<R: Runtime>(app: &AppHandle<R>) {
    let manager = app.state::<XrayManager>();
    let info = manager.status();

    match info.status {
        ConnectionStatus::Connected | ConnectionStatus::Connecting => {
            // Fire an immediate disconnecting event so the frontend can
            // bounce to the "Disconnecting…" state without waiting for
            // its poll loop. The xray watcher / wait thread will emit
            // the final "disconnected" event once xray actually exits.
            let _ = app.emit("connection-status-changed", "disconnecting");
            let _ = manager.stop();
        }
        ConnectionStatus::Disconnected | ConnectionStatus::Error => {
            // Try to connect with last server
            let settings = storage::load_settings(app).unwrap_or_default();
            if let Some(ref server_id) = settings.last_server_id {
                if let Ok(servers) = storage::load_servers(app) {
                    if let Some(server) = servers.iter().find(|s| s.id == *server_id) {
                        // Same pattern as above: emit a connecting
                        // event so the UI flips to "Connecting…" right
                        // away. start() updates internal state to
                        // Connecting synchronously but has no AppHandle
                        // of its own, so we do the emit here.
                        let _ = app.emit("connection-status-changed", "connecting");
                        if let Err(e) = manager.start(app, server, &settings.bypass_domains) {
                            warn!("Tray connect failed: {e}");
                            // start() already reset internal state to
                            // Error, but the frontend may still be
                            // showing "Connecting…" from our emit
                            // above — nudge it back.
                            let _ = app.emit("connection-status-changed", "disconnected");
                        }
                        return;
                    }
                }
            }
            // No last server — show the window instead
            show_main_window(app);
        }
        ConnectionStatus::Disconnecting => {
            // Do nothing while transitioning
        }
    }
}
