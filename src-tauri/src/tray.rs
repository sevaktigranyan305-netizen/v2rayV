use log::{info, warn};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime};

use crate::models::ConnectionStatus;
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

/// Tray Connect/Disconnect button handler. For Disconnect we can
/// drive the manager directly because `XrayManager::stop()` is
/// platform-uniform and idempotent. For Connect we delegate to the
/// frontend by emitting `tray-connect-requested` and showing the
/// window: the frontend already knows which server the user has
/// selected (which may differ from the persisted `last_server_id`,
/// especially after a subscription refresh that hands out fresh
/// internal IDs), and it owns the macOS sudo-password modal flow.
/// Going through the frontend keeps the Connect button consistent
/// with the in-window button instead of silently falling through to
/// "show window" when the persisted state is stale.
fn handle_toggle_connection<R: Runtime>(app: &AppHandle<R>) {
    let manager = app.state::<XrayManager>();
    let info = manager.status();
    info!("Tray toggle clicked; current status: {:?}", info.status);

    match info.status {
        ConnectionStatus::Connected | ConnectionStatus::Connecting => {
            // Fire an immediate disconnecting event so the frontend can
            // bounce to the "Disconnecting…" state without waiting for
            // its poll loop. The xray watcher / wait thread will emit
            // the final "disconnected" event once xray actually exits.
            let _ = app.emit("connection-status-changed", "disconnecting");
            if let Err(e) = manager.stop() {
                warn!("Tray disconnect failed: {e}");
                let _ = app.emit("connection-status-changed", "connected");
            }
        }
        ConnectionStatus::Disconnected | ConnectionStatus::Error => {
            // Surface the window first so the user can see what is
            // happening (and so the sudo-password modal has a parent
            // window on macOS), then ask the frontend to start the
            // connect with whichever server is currently selected.
            show_main_window(app);
            let _ = app.emit("tray-connect-requested", ());
        }
        ConnectionStatus::Disconnecting => {
            // Do nothing while transitioning
        }
    }
}
