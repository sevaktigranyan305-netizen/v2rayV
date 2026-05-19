import { invoke } from '@tauri-apps/api/core';
import type {
	AppSettings,
	ConnectionInfo,
	ConnectOutcome,
	DetectedVpn,
	LogEntry,
	ServerConfig,
	SpeedStats,
	Subscription,
	SubscriptionRefresh
} from '$lib/types';

/**
 * Start xray-core and connect to the given server.
 *
 * On macOS / Linux the backend may return `{ kind: 'NeedsSudoPassword' }`
 * instead of starting xray: that means there is no sudo password
 * cached in the OS credential store yet, the UI must show its
 * `SudoPasswordModal`, call `storeSudoPassword`, and retry this
 * `connect()`.
 */
export async function connect(config: ServerConfig): Promise<ConnectOutcome> {
	return await invoke<ConnectOutcome>('connect', { serverConfig: config });
}

/**
 * macOS / Linux: returns true if the user's sudo password is already
 * cached in the OS credential store (Keychain on macOS, Secret
 * Service on Linux). On Windows always returns false.
 */
export async function hasSudoPassword(): Promise<boolean> {
	return await invoke<boolean>('has_sudo_password');
}

/**
 * macOS / Linux: validate `password` against `sudo -v` and, if it
 * works, save it to the OS credential store for future connects.
 * Rejects with a descriptive error string if sudo refuses the
 * password or the credential store write fails. No-op error on
 * Windows.
 */
export async function storeSudoPassword(password: string): Promise<void> {
	await invoke<void>('store_sudo_password', { password });
}

/**
 * macOS / Linux: drop the cached sudo password from the OS credential
 * store. Called when the backend signals the saved password no longer
 * authenticates.
 */
export async function clearSudoPassword(): Promise<void> {
	await invoke<void>('clear_sudo_password');
}

export async function disconnect(): Promise<void> {
	await invoke<void>('disconnect');
}

export async function getConnectionInfo(): Promise<ConnectionInfo> {
	return await invoke<ConnectionInfo>('get_connection_info');
}

// Server CRUD
export async function getServers(): Promise<ServerConfig[]> {
	return await invoke<ServerConfig[]>('get_servers');
}

export async function addServer(config: ServerConfig): Promise<ServerConfig> {
	return await invoke<ServerConfig>('add_server', { serverConfig: config });
}

export async function updateServer(config: ServerConfig): Promise<void> {
	await invoke<void>('update_server', { serverConfig: config });
}

export async function deleteServer(id: string): Promise<void> {
	await invoke<void>('delete_server', { id });
}

// Import / Export
export async function exportServers(): Promise<string> {
	return await invoke<string>('export_servers');
}

export async function importServers(json: string): Promise<ServerConfig[]> {
	return await invoke<ServerConfig[]>('import_servers', { json });
}

// Saved subscriptions: persisted name+URL with last-refresh timestamp.
// `add_subscription` does both save+import in one call; `refresh_subscription`
// re-fetches the URL and **replaces** every server tagged with that
// subscription's id. `delete_subscription` removes the subscription and
// optionally cascades to its imported servers.
export async function listSubscriptions(): Promise<Subscription[]> {
	return await invoke<Subscription[]>('list_subscriptions');
}

export async function addSubscription(
	name: string,
	url: string
): Promise<SubscriptionRefresh> {
	return await invoke<SubscriptionRefresh>('add_subscription', { name, url });
}

export async function refreshSubscription(id: string): Promise<SubscriptionRefresh> {
	return await invoke<SubscriptionRefresh>('refresh_subscription', { id });
}

export async function deleteSubscription(
	id: string,
	deleteServers: boolean
): Promise<void> {
	await invoke<void>('delete_subscription', { id, deleteServers });
}

// VLESS URI
export async function parseVlessUri(uri: string): Promise<ServerConfig> {
	return await invoke<ServerConfig>('parse_vless_uri_cmd', { uri });
}

export async function exportVlessUri(config: ServerConfig): Promise<string> {
	return await invoke<string>('export_vless_uri', { serverConfig: config });
}

export async function getSpeedStats(): Promise<SpeedStats> {
	return await invoke<SpeedStats>('get_speed_stats');
}

// Logs
export async function getLogs(): Promise<LogEntry[]> {
	return await invoke<LogEntry[]>('get_logs');
}

export async function clearLogs(): Promise<void> {
	await invoke<void>('clear_logs');
}

// Settings
export async function getSettings(): Promise<AppSettings> {
	return await invoke<AppSettings>('get_settings');
}

export async function updateSettings(settings: AppSettings): Promise<void> {
	await invoke<void>('update_settings', { settings });
}

/**
 * Persist a new bypass-domain list and, if the VPN is currently active, restart
 * the xray+TUN stack so the change takes effect immediately. Returns true if a
 * reconnect was performed, false if only the setting was saved.
 */
export async function applyBypassDomains(domains: string[]): Promise<boolean> {
	return await invoke<boolean>('apply_bypass_domains', { domains });
}

// VPN detection
export async function detectVpnInterfaces(): Promise<DetectedVpn[]> {
	return await invoke<DetectedVpn[]>('detect_vpn_interfaces');
}


