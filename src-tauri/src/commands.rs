use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Runtime, State};

#[cfg(target_os = "macos")]
use crate::config;
#[cfg(target_os = "macos")]
use crate::macos_helper;
use crate::models::{
    AppSettings, ConnectionInfo, ConnectionStatus, DetectedVpn, LogEntry, ServerConfig, SpeedStats,
    Subscription,
};
use crate::network;
use crate::storage;
use crate::subscription;
use crate::xray::XrayManager;

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Outcome of a `connect` call. We need a richer return type than
/// `()` because on macOS we may have to bounce back to the UI to ask
/// the user for their sudo password before we can actually launch
/// xray under root. The `kind` discriminator is what the frontend
/// switches on.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum ConnectOutcome {
    /// xray-core was spawned (possibly via sudo on macOS) and is now
    /// transitioning through Connecting → Connected. The frontend's
    /// poll loop will pick up the actual status.
    Ok,
    /// macOS only: there is no sudo password in Keychain (first connect
    /// since install, or the saved entry was just deleted because it
    /// no longer authenticated). The frontend should show its
    /// SudoPasswordModal and call `macos_store_sudo_password` before
    /// invoking `connect` again.
    NeedsSudoPassword,
}

#[tauri::command]
pub fn connect<R: Runtime>(
    app: AppHandle<R>,
    manager: State<'_, XrayManager>,
    server_config: ServerConfig,
) -> Result<ConnectOutcome, String> {
    server_config.validate()?;

    // macOS supports L3 only. Refuse non-L3 servers up front so the
    // user gets a clear error instead of a cryptic xray spawn failure.
    // If the server *is* L3, make sure we have a sudo password stashed
    // in Keychain — if not, surface NeedsSudoPassword to the UI.
    #[cfg(target_os = "macos")]
    {
        if config::virtualnet_enabled(&server_config).is_none() {
            return Err("macOS build of v2rayV supports L3 (vnet=1) servers only. \
                 The selected server is missing vnet=1/vnetIp= parameters."
                .to_string());
        }
        if !macos_helper::has_password() {
            return Ok(ConnectOutcome::NeedsSudoPassword);
        }
    }

    let settings = storage::load_settings(&app).unwrap_or_default();
    manager
        .start(&app, &server_config, &settings.bypass_domains)
        .map_err(|e| e.to_string())?;

    // Save last server id for auto-connect and tray reconnect
    let mut settings = settings;
    settings.last_server_id = Some(server_config.id.clone());
    let _ = storage::save_settings(&app, &settings);

    Ok(ConnectOutcome::Ok)
}

/// True if a macOS sudo password is currently cached in Keychain.
/// Other platforms return false unconditionally so the frontend
/// can call this without cfg gymnastics.
#[tauri::command]
pub fn macos_has_sudo_password() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(macos_helper::has_password())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(false)
    }
}

/// Validate a candidate macOS sudo password and, if valid, save it to
/// Keychain so subsequent connects can re-use it. Returns Ok(()) on
/// success and Err with a human-readable message on failure (wrong
/// password, sudo missing, Keychain write rejected).
///
/// On non-macOS platforms this is a no-op error so the IPC surface
/// stays uniform but the frontend never accidentally believes it
/// succeeded.
#[tauri::command]
pub fn macos_store_sudo_password(password: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if password.is_empty() {
            return Err("Password must not be empty".to_string());
        }
        let ok = macos_helper::validate_password(&password).map_err(|e| e.to_string())?;
        if !ok {
            return Err("Incorrect macOS password".to_string());
        }
        macos_helper::write_password(&password).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = password;
        Err("macos_store_sudo_password is only available on macOS".to_string())
    }
}

/// Remove the cached macOS sudo password from Keychain. The UI calls
/// this when sudo rejects the saved password (e.g. the user changed
/// their macOS account password) so the next connect re-prompts.
#[tauri::command]
pub fn macos_clear_sudo_password() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos_helper::delete_password().map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[tauri::command]
pub fn validate_config(server_config: ServerConfig) -> Result<(), String> {
    server_config.validate()
}

#[tauri::command]
pub fn disconnect<R: Runtime>(
    _app: AppHandle<R>,
    manager: State<'_, XrayManager>,
) -> Result<(), String> {
    manager.stop().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_status(manager: State<'_, XrayManager>) -> Result<ConnectionStatus, String> {
    Ok(manager.status().status)
}

#[tauri::command]
pub fn get_connection_info(manager: State<'_, XrayManager>) -> Result<ConnectionInfo, String> {
    Ok(manager.status())
}

#[tauri::command]
pub fn test_connection(manager: State<'_, XrayManager>) -> Result<bool, String> {
    manager.test_connection().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_socks_port(manager: State<'_, XrayManager>) -> Result<u16, String> {
    Ok(manager.socks_port())
}

#[tauri::command]
pub fn get_servers<R: Runtime>(app: AppHandle<R>) -> Result<Vec<ServerConfig>, String> {
    storage::load_servers(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_server<R: Runtime>(
    app: AppHandle<R>,
    server_config: ServerConfig,
) -> Result<ServerConfig, String> {
    let mut servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    let mut new_server = server_config;
    // Always assign a fresh id
    new_server.id = uuid::Uuid::new_v4().to_string();
    servers.push(new_server.clone());
    storage::save_servers(&app, &servers).map_err(|e| e.to_string())?;
    Ok(new_server)
}

#[tauri::command]
pub fn update_server<R: Runtime>(
    app: AppHandle<R>,
    server_config: ServerConfig,
) -> Result<(), String> {
    let mut servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    let pos = servers
        .iter()
        .position(|s| s.id == server_config.id)
        .ok_or_else(|| format!("Server with id {} not found", server_config.id))?;
    servers[pos] = server_config;
    storage::save_servers(&app, &servers).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_server<R: Runtime>(app: AppHandle<R>, id: String) -> Result<(), String> {
    let mut servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    let len_before = servers.len();
    servers.retain(|s| s.id != id);
    if servers.len() == len_before {
        return Err(format!("Server with id {id} not found"));
    }
    storage::save_servers(&app, &servers).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_servers<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&servers).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_servers<R: Runtime>(
    app: AppHandle<R>,
    json: String,
) -> Result<Vec<ServerConfig>, String> {
    let imported: Vec<ServerConfig> =
        serde_json::from_str(&json).map_err(|e| format!("Invalid JSON: {e}"))?;
    let mut servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    // Assign fresh ids to imported servers to avoid collisions
    let new_servers: Vec<ServerConfig> = imported
        .into_iter()
        .map(|mut s| {
            s.id = uuid::Uuid::new_v4().to_string();
            s
        })
        .collect();
    servers.extend(new_servers.clone());
    storage::save_servers(&app, &servers).map_err(|e| e.to_string())?;
    Ok(new_servers)
}

/// Result of a subscription add/refresh: the (possibly updated) saved
/// subscription plus the freshly-imported set of servers tagged with
/// that subscription's id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionRefresh {
    pub subscription: Subscription,
    pub servers: Vec<ServerConfig>,
}

/// List every saved subscription. The UI uses this to render the
/// "Subscriptions" panel with Refresh / Delete buttons per row.
#[tauri::command]
pub fn list_subscriptions<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Subscription>, String> {
    storage::load_subscriptions(&app).map_err(|e| e.to_string())
}

/// Save a subscription (name + URL) and import its servers in one go.
/// The URL is fetched, decoded (base64 or plaintext), and every parseable
/// `vless://` line is persisted with `subscription_id` set so a later
/// `refresh_subscription` can replace exactly this set.
///
/// Sync storage I/O is dispatched via `spawn_blocking` so it cannot stall
/// the async runtime worker that is also responsible for delivering the
/// IPC response back to the WebView. Without this, `pnpm tauri build` on
/// Windows occasionally left the frontend `await invoke(...)` promise
/// pending forever even though the data on disk was written correctly.
#[tauri::command]
pub async fn add_subscription<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    url: String,
) -> Result<SubscriptionRefresh, String> {
    log::info!("add_subscription: start (name={name:?})");
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Subscription name must not be empty".to_string());
    }

    log::info!("add_subscription: fetching {url}");
    let parsed = subscription::fetch_subscription(&url)
        .await
        .map_err(|e| e.to_string())?;
    log::info!("add_subscription: fetched {} server(s)", parsed.len());
    if parsed.is_empty() {
        return Err("No vless:// servers found in subscription".to_string());
    }

    let sub_id = uuid::Uuid::new_v4().to_string();
    let new_servers: Vec<ServerConfig> = parsed
        .into_iter()
        .map(|mut s| {
            s.id = uuid::Uuid::new_v4().to_string();
            s.subscription_id = Some(sub_id.clone());
            s
        })
        .collect();

    let subscription = Subscription {
        id: sub_id,
        name,
        url: url.trim().to_string(),
        last_updated_at: Some(now_unix_seconds()),
        last_server_count: Some(new_servers.len() as u32),
    };

    let app_for_blocking = app.clone();
    let new_servers_for_blocking = new_servers.clone();
    let subscription_for_blocking = subscription.clone();
    log::info!("add_subscription: persisting to disk");
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut servers = storage::load_servers(&app_for_blocking).map_err(|e| e.to_string())?;
        servers.extend(new_servers_for_blocking);
        storage::save_servers(&app_for_blocking, &servers).map_err(|e| e.to_string())?;

        let mut subs = storage::load_subscriptions(&app_for_blocking).map_err(|e| e.to_string())?;
        subs.push(subscription_for_blocking);
        storage::save_subscriptions(&app_for_blocking, &subs).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| format!("storage task panicked: {e}"))??;

    log::info!("add_subscription: returning success");
    Ok(SubscriptionRefresh {
        subscription,
        servers: new_servers,
    })
}

/// Re-fetch a saved subscription and **replace** every server tagged with
/// its id with the freshly-imported set. The subscription's
/// `last_updated_at` and `last_server_count` are bumped on success.
///
/// If the URL still parses but returns zero servers, the call fails and
/// nothing is touched on disk — better to keep the old set than wipe
/// everything because of a transient panel-side error.
#[tauri::command]
pub async fn refresh_subscription<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<SubscriptionRefresh, String> {
    log::info!("refresh_subscription: start id={id}");
    let app_for_lookup = app.clone();
    let id_for_lookup = id.clone();
    let url = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let subs = storage::load_subscriptions(&app_for_lookup).map_err(|e| e.to_string())?;
        subs.into_iter()
            .find(|s| s.id == id_for_lookup)
            .map(|s| s.url)
            .ok_or_else(|| format!("Subscription with id {id_for_lookup} not found"))
    })
    .await
    .map_err(|e| format!("storage task panicked: {e}"))??;

    log::info!("refresh_subscription: fetching {url}");
    let parsed = subscription::fetch_subscription(&url)
        .await
        .map_err(|e| e.to_string())?;
    log::info!("refresh_subscription: fetched {} server(s)", parsed.len());
    if parsed.is_empty() {
        return Err("No vless:// servers found in subscription".to_string());
    }

    let app_for_blocking = app.clone();
    let id_for_blocking = id.clone();
    // All storage I/O — including the ID-preservation lookup — runs on
    // a blocking thread. Doing the lookup on the async runtime worker
    // can leave the IPC response pending forever on Windows (see the
    // matching note in `add_subscription`).
    let (updated, new_servers) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Subscription, Vec<ServerConfig>), String> {
            let mut servers =
                storage::load_servers(&app_for_blocking).map_err(|e| e.to_string())?;

            // Preserve internal IDs across refresh: a server in the
            // freshly-fetched batch that matches an existing server
            // on (address, port, vless-uuid) is the same server from
            // the user's point of view (name may have been edited in
            // the panel), so we reuse the old internal ID. Without
            // this every refresh would hand out brand-new UUIDs to
            // every server and invalidate any `last_server_id`
            // reference that auto-connect and the tray's Connect
            // button rely on.
            let existing_for_sub: std::collections::HashMap<(String, u16, String), String> =
                servers
                    .iter()
                    .filter(|s| s.subscription_id.as_deref() == Some(id_for_blocking.as_str()))
                    .map(|s| ((s.address.clone(), s.port, s.uuid.clone()), s.id.clone()))
                    .collect();

            let new_servers: Vec<ServerConfig> = parsed
                .into_iter()
                .map(|mut s| {
                    let key = (s.address.clone(), s.port, s.uuid.clone());
                    s.id = existing_for_sub
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    s.subscription_id = Some(id_for_blocking.clone());
                    s
                })
                .collect();

            // Replace every server tagged with this subscription id
            // with the freshly-imported set. Manually-added servers
            // (subscription_id == None) and servers from other
            // subscriptions are left alone.
            servers.retain(|s| s.subscription_id.as_deref() != Some(id_for_blocking.as_str()));
            let new_count = new_servers.len() as u32;
            servers.extend(new_servers.clone());
            storage::save_servers(&app_for_blocking, &servers).map_err(|e| e.to_string())?;

            let mut subs =
                storage::load_subscriptions(&app_for_blocking).map_err(|e| e.to_string())?;
            let pos = subs
                .iter()
                .position(|s| s.id == id_for_blocking)
                .ok_or_else(|| format!("Subscription with id {id_for_blocking} not found"))?;
            subs[pos].last_updated_at = Some(now_unix_seconds());
            subs[pos].last_server_count = Some(new_count);
            let updated = subs[pos].clone();
            storage::save_subscriptions(&app_for_blocking, &subs).map_err(|e| e.to_string())?;
            Ok((updated, new_servers))
        },
    )
    .await
    .map_err(|e| format!("storage task panicked: {e}"))??;

    log::info!("refresh_subscription: returning success");
    Ok(SubscriptionRefresh {
        subscription: updated,
        servers: new_servers,
    })
}

/// Delete a saved subscription. Per-server cascade is opt-in via
/// `delete_servers` — by default the imported servers stay in the list
/// (just untagged from the subscription) so the user doesn't lose them
/// by accident.
#[tauri::command]
pub fn delete_subscription<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    delete_servers: bool,
) -> Result<(), String> {
    let mut subs = storage::load_subscriptions(&app).map_err(|e| e.to_string())?;
    let len_before = subs.len();
    subs.retain(|s| s.id != id);
    if subs.len() == len_before {
        return Err(format!("Subscription with id {id} not found"));
    }
    storage::save_subscriptions(&app, &subs).map_err(|e| e.to_string())?;

    let mut servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    if delete_servers {
        servers.retain(|s| s.subscription_id.as_deref() != Some(id.as_str()));
    } else {
        for s in servers.iter_mut() {
            if s.subscription_id.as_deref() == Some(id.as_str()) {
                s.subscription_id = None;
            }
        }
    }
    storage::save_servers(&app, &servers).map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_speed_stats<R: Runtime>(
    app: AppHandle<R>,
    manager: State<'_, XrayManager>,
) -> Result<SpeedStats, String> {
    manager.query_stats(&app).await.map_err(|e| e.to_string())
}

// Logs
#[tauri::command]
pub fn get_logs(manager: State<'_, XrayManager>) -> Result<Vec<LogEntry>, String> {
    Ok(manager.get_logs())
}

#[tauri::command]
pub fn clear_logs(manager: State<'_, XrayManager>) -> Result<(), String> {
    manager.clear_logs();
    Ok(())
}

// Settings
#[tauri::command]
pub fn get_settings<R: Runtime>(app: AppHandle<R>) -> Result<AppSettings, String> {
    storage::load_settings(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_settings<R: Runtime>(app: AppHandle<R>, settings: AppSettings) -> Result<(), String> {
    storage::save_settings(&app, &settings).map_err(|e| e.to_string())
}

/// Persist a new bypass-domain list. If a VPN session is active, rebuild the
/// xray/TUN/gsettings stack so the new list takes effect immediately — without
/// this, the running session keeps the list it was started with and edits in
/// the UI silently do nothing until the user reconnects.
///
/// Returns `true` if a reconnect was performed, `false` if only the setting was
/// saved (because there was no active session).
#[tauri::command]
pub fn apply_bypass_domains<R: Runtime>(
    app: AppHandle<R>,
    manager: State<'_, XrayManager>,
    domains: Vec<String>,
) -> Result<bool, String> {
    let mut settings = storage::load_settings(&app).unwrap_or_default();
    // Defense-in-depth against the frontend firing this on no-op changes: if
    // the saved list already matches, don't touch the running session. A
    // spurious stop→start cycle tears down xray and TUN for nothing and the
    // user sees the VPN drop mid-session.
    let unchanged = settings.bypass_domains == domains;
    settings.bypass_domains = domains.clone();
    storage::save_settings(&app, &settings).map_err(|e| e.to_string())?;

    let status = manager.status().status;
    let active = matches!(
        status,
        ConnectionStatus::Connected | ConnectionStatus::Connecting
    );
    if !active || unchanged {
        return Ok(false);
    }

    log::info!(
        "Reloading xray with new bypass domains ({} entries)",
        domains.len()
    );

    let server_id = settings.last_server_id.clone().ok_or_else(|| {
        "No last_server_id; cannot reload bypass without a known server".to_string()
    })?;
    let servers = storage::load_servers(&app).map_err(|e| e.to_string())?;
    let server = servers
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("Server with id {server_id} not found"))?;

    manager.stop().map_err(|e| e.to_string())?;
    // Give TUN teardown a moment so hev-socks5-tunnel releases rvpn0 before
    // the new xray+hev pair tries to recreate it.
    std::thread::sleep(std::time::Duration::from_millis(500));
    manager
        .start(&app, &server, &domains)
        .map_err(|e| e.to_string())?;

    Ok(true)
}

// VPN detection
#[tauri::command]
pub fn detect_vpn_interfaces() -> Result<Vec<DetectedVpn>, String> {
    Ok(network::detect_vpn_routes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RealitySettings;

    fn valid_config() -> ServerConfig {
        ServerConfig {
            id: "test-id-1234".to_string(),
            name: "Test".to_string(),
            address: "1.2.3.4".to_string(),
            port: 443,
            uuid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            flow: "xtls-rprx-vision".to_string(),
            reality: RealitySettings {
                public_key: "abc123".to_string(),
                short_id: "def456".to_string(),
                server_name: "example.com".to_string(),
                fingerprint: "chrome".to_string(),
            },
            virtualnet: None,
            subscription_id: None,
        }
    }

    #[test]
    fn validate_config_accepts_valid_config() {
        let config = valid_config();
        assert!(validate_config(config).is_ok());
    }

    #[test]
    fn validate_config_rejects_empty_address() {
        let mut config = valid_config();
        config.address = String::new();
        let err = validate_config(config).unwrap_err();
        assert!(
            err.contains("address"),
            "expected 'address' in error: {err}"
        );
    }

    #[test]
    fn validate_config_rejects_port_zero() {
        let mut config = valid_config();
        config.port = 0;
        let err = validate_config(config).unwrap_err();
        assert!(err.contains("port"), "expected 'port' in error: {err}");
    }

    #[test]
    fn validate_config_rejects_invalid_uuid() {
        let mut config = valid_config();
        config.uuid = "not-a-valid-uuid".to_string();
        let err = validate_config(config).unwrap_err();
        assert!(err.contains("UUID"), "expected 'UUID' in error: {err}");
    }
}
