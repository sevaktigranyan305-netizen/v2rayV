# v2rayV — Tauri IPC API Reference

All commands are invoked from the frontend via `src/lib/api/tauri.ts` using `@tauri-apps/api/core`'s `invoke()`. On the Rust side, commands return `Result<T, String>` — on error, the `invoke()` call rejects with the error string.

## TypeScript Interfaces

Defined in `src/lib/types/index.ts`. These mirror the Rust structs in `src-tauri/src/models.rs`.

```typescript
export interface RealitySettings {
  public_key: string;   // X25519 public key (Base64url)
  short_id: string;     // Hex short ID (max 16 chars)
  server_name: string;  // TLS SNI domain (e.g. "www.microsoft.com")
  fingerprint: string;  // TLS fingerprint (e.g. "chrome")
}

export interface VirtualNetSettings {
  enabled: boolean;
  subnet: string;                  // CIDR like "10.10.0.0/24" (IPv4 only)
  vnet_ip: string;                 // Pre-allocated per-uuid IPv4 from the panel (e.g. "10.10.0.5")
  default_route: boolean;          // Route 0.0.0.0/0 through the TUN; mirrors vnetDefaultRoute=1
  interface_name: string | null;   // Defaults to "v2rayV" inside xray-core
  mtu: number | null;              // 0/null = let xray-core pick
}

export interface ServerConfig {
  id: string;                              // UUID v4 (generated internally, not the VLESS user UUID)
  name: string;                            // Display name (optional, defaults to address)
  address: string;                         // Server IP or hostname
  port: number;                            // Server port (1–65535)
  uuid: string;                            // VLESS user UUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)
  flow: string;                            // XTLS flow (e.g. "xtls-rprx-vision")
  reality: RealitySettings;
  virtualnet: VirtualNetSettings | null;   // L3 mode hints from vnet=/vnetIp= URI params
  subscription_id: string | null;          // Set if this server came from a saved subscription
}

export interface Subscription {
  id: string;                          // UUID v4
  name: string;                        // User-chosen display name
  url: string;                         // HTTPS subscription URL
  last_updated_at: number | null;      // Unix-seconds of the last successful refresh
  last_server_count: number | null;    // Number of servers the last refresh produced
}

export interface DetectedVpn {
  interface: string;    // Interface name (e.g. "tun0", "wg0")
  vpn_type: string;     // Human-readable type (e.g. "OpenVPN", "WireGuard")
  subnets: string[];    // Routed subnets (e.g. ["10.8.0.0/24"])
  server_ip: string | null; // VPN server endpoint IP if detected
}

export type ConnectionStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'disconnecting'
  | 'error';

export interface ConnectionInfo {
  status: ConnectionStatus;
  server_name: string | null;      // Display name of the connected server
  server_address: string | null;   // IP/host of the connected server
  connected_since: number | null;  // Unix timestamp (seconds) of connect time
  error_message: string | null;    // Set when status is 'error'
}

export interface SpeedStats {
  upload_speed: number;     // Bytes/second since last poll
  download_speed: number;   // Bytes/second since last poll
  total_upload: number;     // Cumulative bytes uploaded since connect
  total_download: number;   // Cumulative bytes downloaded since connect
}

export interface LogEntry {
  timestamp: number;        // Unix epoch seconds
  level: string;            // "info" | "warn" | "error" (from xray output)
  message: string;
}

export interface AppSettings {
  auto_connect: boolean;             // If true, reconnect to last_server_id on startup
  last_server_id: string | null;     // Internal UUID of the last-used server
  bypass_domains: string[];          // Domains that must skip the VPN (direct route)
}
```

---

## Subscription Commands

Subscriptions are saved to `subscriptions.json` (mode 0600). Each `ServerConfig` produced from a subscription is tagged with the subscription's id via `subscription_id`. `refresh_subscription` re-fetches the URL and atomically replaces only the servers belonging to that subscription.

### `list_subscriptions`

**Rust signature:**
```rust
pub fn list_subscriptions<R: Runtime>(app: AppHandle<R>) -> Result<Vec<Subscription>, String>
```

**TypeScript wrapper:**
```typescript
export async function listSubscriptions(): Promise<Subscription[]>
```

### `add_subscription`

Fetch + parse + save in one go.

**Rust signature:**
```rust
pub async fn add_subscription<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    url: String,
) -> Result<Subscription, String>
```

**TypeScript wrapper:**
```typescript
export async function addSubscription(name: string, url: string): Promise<Subscription>
```

**Behavior:** HTTPS-fetches the URL, base64-decodes (or treats as plaintext) the body, splits into `vless://` lines, parses each with `uri::parse_vless_uri`, tags every parsed server with the new subscription's id, atomically writes both `subscriptions.json` and `servers.json`. All file I/O runs inside `tauri::async_runtime::spawn_blocking`.

**Error cases:** network errors from the fetch, base64 decode errors, per-line URI parse errors (each is collected; the command fails only if zero servers parsed successfully).

### `refresh_subscription`

Re-fetch a saved subscription and replace its servers.

**Rust signature:**
```rust
pub async fn refresh_subscription<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<(Subscription, Vec<ServerConfig>), String>
```

**TypeScript wrapper:**
```typescript
export async function refreshSubscription(id: string): Promise<{
    subscription: Subscription;
    servers: ServerConfig[];
}>
```

**Behavior:** re-fetches the saved URL, drops every `ServerConfig` whose `subscription_id == id`, appends the freshly-parsed (and re-tagged) servers, bumps `last_updated_at` (Unix seconds) and `last_server_count` on the subscription. Manually-added servers (`subscription_id == None`) and servers from other subscriptions are not touched. Returns the updated subscription and the *new* server list belonging to it.

**Frontend mitigation:** `SubscriptionList.svelte` races the IPC against a 25 s watchdog; on timeout it manually reloads the subscriptions and servers stores so the UI converges with the on-disk state even if the IPC reply stalls (observed under Parallels x86_64 emulation).

### `delete_subscription`

**Rust signature:**
```rust
pub fn delete_subscription<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    delete_servers: bool,
) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function deleteSubscription(id: string, deleteServers: boolean): Promise<void>
```

**Behavior:** removes the subscription entry. If `delete_servers == true`, also removes every `ServerConfig` whose `subscription_id == id`. Both writes are atomic.

---

## Connection Commands

### `connect`

Validates the server config and starts the xray sidecar process.

**Rust signature:**
```rust
pub fn connect(app: AppHandle<R>, manager: State<'_, XrayManager>, server_config: ServerConfig) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function connect(config: ServerConfig): Promise<void>
// invoke('connect', { serverConfig: config })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `serverConfig` | `ServerConfig` | Full server configuration to connect to |

**Returns:** `Promise<void>`

**Error cases:**
- `"Server address must not be empty"` — address field is blank or whitespace
- `"Server port must be greater than 0"` — port is 0
- `"UUID must match format xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx (hex characters)"` — invalid UUID
- `"Reality public_key must not be empty"` — public_key is blank
- `"Reality short_id must not be empty"` — short_id is blank
- `"Already connected or connecting"` — xray is already running
- `"Failed to get app data dir: ..."` — OS path resolution failure
- `"Failed to create sidecar command: ..."` — xray binary not found in bundles
- `"Failed to spawn xray: ..."` — OS process spawn failure

**Behavior:** Sets status to `connecting`. If `serverConfig.virtualnet` is `Some(VirtualNetSettings { enabled: true, .. })`, generates an L3 config (no SOCKS/HTTP inbound, only a VLESS+REALITY outbound with a `virtualNetwork{}` block) and skips system-proxy / `hev-socks5-tunnel` setup. Otherwise generates the legacy proxy/TUN config. In both cases writes the config to disk and spawns xray. Status transitions to `connected` asynchronously when xray's stderr matches `"started"`, `"core: Xray "`, or `"virtualNetwork: l3client created"`. A 6 s fallback timer also flips the state if any output has been observed; a 15 s hard timeout fires for genuinely stuck launches.

---

### `disconnect`

Kills the xray process and cleans up the config file.

**Rust signature:**
```rust
pub fn disconnect(manager: State<'_, XrayManager>) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function disconnect(): Promise<void>
// invoke('disconnect')
```

**Returns:** `Promise<void>`

**Error cases:**
- `"Failed to kill xray: ..."` — OS-level kill failure (rare)

**Behavior:** Sets status to `disconnecting`, hard-kills xray via `child.kill()` (`TerminateProcess` on Windows, `SIGKILL` on Unix), deletes the temp config file, sets status to `disconnected`. In L3 mode, `proxy::disable_system_proxy()` and `tun::stop_tun()` are skipped (we never enabled them).

**Frontend mitigation:** `connection.svelte.ts disconnectVpn()` arms an 8 s watchdog and optimistically flips the UI to `disconnecting`. If the IPC reply lags, the watchdog fires and the store flips `status = disconnected` itself. The 1 Hz `get_connection_info` poll loop confirms.

---

### `get_status`

Returns the current connection status as a string.

**Rust signature:**
```rust
pub fn get_status(manager: State<'_, XrayManager>) -> Result<ConnectionStatus, String>
```

**TypeScript wrapper:**
```typescript
export async function getStatus(): Promise<string>
// invoke('get_status')
```

**Returns:** `Promise<string>` — one of: `"disconnected"`, `"connecting"`, `"connected"`, `"disconnecting"`, `"error"`

**Error cases:** None (infallible).

**Note:** The `connectionStore` uses `get_connection_info` (which includes more data) for polling rather than this simpler command.

---

### `get_connection_info`

Returns the full connection state including server name, address, uptime timestamp, and error message.

**Rust signature:**
```rust
pub fn get_connection_info(manager: State<'_, XrayManager>) -> Result<ConnectionInfo, String>
```

**TypeScript wrapper:**
```typescript
export async function getConnectionInfo(): Promise<ConnectionInfo>
// invoke('get_connection_info')
```

**Returns:** `Promise<ConnectionInfo>`

**Error cases:** None (infallible).

**Usage:** Called on page mount and then polled every 1 second by `connectionStore` while connected.

---

### `test_connection`

Checks whether the local SOCKS5 proxy port is open and accepting TCP connections.

**Rust signature:**
```rust
pub fn test_connection(manager: State<'_, XrayManager>) -> Result<bool, String>
```

**TypeScript wrapper:** Not currently exposed in `tauri.ts` (available for future use).

**Returns:** `Promise<boolean>` — `true` if TCP connect to `127.0.0.1:10808` succeeds within 3 seconds, `false` otherwise.

**Error cases:** None (returns `false` on failure rather than throwing).

---

### `get_socks_port`

Returns the SOCKS5 port xray listens on.

**Rust signature:**
```rust
pub fn get_socks_port(manager: State<'_, XrayManager>) -> Result<u16, String>
```

**TypeScript wrapper:** Not currently exposed in `tauri.ts` (available for future use).

**Returns:** `Promise<number>` — always `10808` in the current implementation.

**Error cases:** None (infallible).

---

### `validate_config`

Validates a `ServerConfig` without starting a connection. Useful for form validation before saving.

**Rust signature:**
```rust
pub fn validate_config(server_config: ServerConfig) -> Result<(), String>
```

**TypeScript wrapper:** Not currently exposed in `tauri.ts` (available for future use; frontend does its own validation in `ServerForm.svelte`).

**Returns:** `Promise<void>`

**Error cases:** Same validation errors as `connect` (address empty, port zero, invalid UUID, empty public_key, empty short_id).

---

### `detect_vpn_interfaces`

Runs fresh detection of corporate VPN interfaces by parsing `ip -j route show` output. Identifies VPN interfaces (tun, tap, wg, ppp, nordlynx, tailscale) and their routed subnets.

**Rust signature:**
```rust
pub fn detect_vpn_interfaces() -> Result<Vec<DetectedVpn>, String>
```

**TypeScript wrapper:**
```typescript
export async function detectVpnInterfaces(): Promise<DetectedVpn[]>
// invoke('detect_vpn_interfaces')
```

**Returns:** `Promise<DetectedVpn[]>` — array of detected VPN interfaces. Empty array if no VPN interfaces found or if `ip` command is unavailable.

**Error cases:** None (returns empty array on failure).

**Behavior:** Executes `ip -j route show`, parses the JSON output, identifies VPN interfaces by name prefix, collects their non-default routed subnets, and detects VPN server endpoint IPs from static `/32` host routes. This is also called automatically during `connect` — detected subnets are added to both gsettings ignore-hosts and xray routing rules.



---

### `get_speed_stats`

Polls xray's StatsService (`127.0.0.1:10085`) for cumulative outbound traffic counters and computes instantaneous up/down speed by diffing against the previous poll.

**Rust signature:**
```rust
pub async fn get_speed_stats(app: AppHandle<R>, manager: State<'_, XrayManager>) -> Result<SpeedStats, String>
```

**TypeScript wrapper:**
```typescript
export async function getSpeedStats(): Promise<SpeedStats>
// invoke('get_speed_stats')
```

**Returns:** `Promise<SpeedStats>` — zeroed when no session is active. `connectionStore` polls this once per second while connected; `SpeedGraph.svelte` renders the result as a sparkline.

**Error cases:** Returns the cached snapshot if the StatsService poll fails (e.g. xray is starting up).

---

## Server CRUD Commands

### `get_servers`

Loads the full server list from the persisted `servers.json` file.

**Rust signature:**
```rust
pub fn get_servers(app: AppHandle<R>) -> Result<Vec<ServerConfig>, String>
```

**TypeScript wrapper:**
```typescript
export async function getServers(): Promise<ServerConfig[]>
// invoke('get_servers')
```

**Returns:** `Promise<ServerConfig[]>` — empty array if no file exists yet.

**Error cases:**
- `"Failed to get app config dir: ..."` — OS path resolution failure
- `"..."` — JSON deserialization failure (corrupted `servers.json`)

---

### `add_server`

Appends a new server to the list. Always assigns a fresh UUID v4 as the server's internal `id`.

**Rust signature:**
```rust
pub fn add_server(app: AppHandle<R>, server_config: ServerConfig) -> Result<ServerConfig, String>
```

**TypeScript wrapper:**
```typescript
export async function addServer(config: ServerConfig): Promise<ServerConfig>
// invoke('add_server', { serverConfig: config })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `serverConfig` | `ServerConfig` | Server to add. The `id` field is ignored and replaced. |

**Returns:** `Promise<ServerConfig>` — the saved server with its new `id`.

**Error cases:**
- Storage read/write errors.

---

### `update_server`

Replaces an existing server entry identified by `server_config.id`.

**Rust signature:**
```rust
pub fn update_server(app: AppHandle<R>, server_config: ServerConfig) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function updateServer(config: ServerConfig): Promise<void>
// invoke('update_server', { serverConfig: config })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `serverConfig` | `ServerConfig` | Updated server. Must include the correct `id`. |

**Returns:** `Promise<void>`

**Error cases:**
- `"Server with id <id> not found"` — no server with that ID exists.
- Storage read/write errors.

---

### `delete_server`

Removes a server by its internal ID.

**Rust signature:**
```rust
pub fn delete_server(app: AppHandle<R>, id: String) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function deleteServer(id: string): Promise<void>
// invoke('delete_server', { id })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `id` | `string` | Internal UUID of the server to delete |

**Returns:** `Promise<void>`

**Error cases:**
- `"Server with id <id> not found"` — no server with that ID exists.
- Storage read/write errors.

---

## Import / Export Commands

### `export_servers`

Serializes the full server list to a pretty-printed JSON string.

**Rust signature:**
```rust
pub fn export_servers(app: AppHandle<R>) -> Result<String, String>
```

**TypeScript wrapper:**
```typescript
export async function exportServers(): Promise<string>
// invoke('export_servers')
```

**Returns:** `Promise<string>` — pretty-printed JSON array of `ServerConfig` objects.

**Error cases:** Storage read errors; JSON serialization errors (should not occur in practice).

**Usage in UI:** `ImportExportBar` calls this, then uses `tauri-plugin-fs` to write the string to a user-chosen file path via the save dialog.

---

### `import_servers`

Parses a JSON array of server configs and appends them to the existing list. All imported servers receive fresh IDs.

**Rust signature:**
```rust
pub fn import_servers(app: AppHandle<R>, json: String) -> Result<Vec<ServerConfig>, String>
```

**TypeScript wrapper:**
```typescript
export async function importServers(json: string): Promise<ServerConfig[]>
// invoke('import_servers', { json })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `json` | `string` | JSON array string of `ServerConfig` objects |

**Returns:** `Promise<ServerConfig[]>` — the newly added servers with their assigned IDs.

**Error cases:**
- `"Invalid JSON: ..."` — input is not valid JSON or does not match `Vec<ServerConfig>` shape.
- Storage read/write errors.

---

### `parse_vless_uri_cmd`

Parses a `vless://` URI string into a `ServerConfig`. Does not save — use `add_server` afterward to persist.

**Rust signature:**
```rust
pub fn parse_vless_uri_cmd(uri: String) -> Result<ServerConfig, String>
```

**TypeScript wrapper:**
```typescript
export async function parseVlessUri(uri: string): Promise<ServerConfig>
// invoke('parse_vless_uri_cmd', { uri })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `uri` | `string` | A `vless://` URI string |

**Returns:** `Promise<ServerConfig>` — parsed server config with a fresh internal `id`.

**Error cases:**
- `"Configuration error: URI must start with vless://"` — wrong scheme
- `"Configuration error: Missing @ in vless URI"` — malformed authority
- `"Configuration error: Missing port in vless URI"` — no port
- `"Configuration error: Invalid port: <value>"` — port not a valid u16
- `"Configuration error: Missing closing ] for IPv6 address"` — malformed IPv6

**Query parameter mapping:**

| URI param | Field |
|-----------|-------|
| `flow` | `flow` |
| `sni` | `reality.server_name` |
| `fp` | `reality.fingerprint` (default: `"chrome"`) |
| `pbk` | `reality.public_key` |
| `sid` | `reality.short_id` |
| `#fragment` | `name` (URL-decoded) |

Unknown parameters (`encryption`, `type`, `security`, etc.) are silently ignored.

---

### `export_vless_uri`

Serializes a `ServerConfig` into a `vless://` URI string.

**Rust signature:**
```rust
pub fn export_vless_uri(server_config: ServerConfig) -> Result<String, String>
```

**TypeScript wrapper:**
```typescript
export async function exportVlessUri(config: ServerConfig): Promise<string>
// invoke('export_vless_uri', { serverConfig: config })
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `serverConfig` | `ServerConfig` | Server to serialize |

**Returns:** `Promise<string>` — a `vless://` URI.

**Error cases:** None (infallible).

**Output format:**
```
vless://UUID@ADDRESS:PORT?encryption=none&flow=FLOW&type=tcp&security=reality&sni=SNI&fp=FP&pbk=PBK&sid=SID#NAME
```

All string values are percent-encoded. The `name` fragment uses `%20` for spaces (uppercase hex in percent-encoded output).

---

## Settings & Logs Commands

### `get_settings`

Loads `AppSettings` from `<app_config_dir>/settings.json`. Returns defaults if no file exists.

**Rust signature:**
```rust
pub fn get_settings(app: AppHandle<R>) -> Result<AppSettings, String>
```

**TypeScript wrapper:**
```typescript
export async function getSettings(): Promise<AppSettings>
// invoke('get_settings')
```

**Defaults:** `auto_connect = false`, `last_server_id = null`, `bypass_domains = ["claude.ai", "anthropic.com", "api.anthropic.com", "wb.ru", "wildberries.ru"]`.

---

### `update_settings`

Writes the full `AppSettings` blob to disk. Use this for `auto_connect` and `last_server_id` changes — for `bypass_domains`, prefer `apply_bypass_domains` (which also reloads a live session).

**Rust signature:**
```rust
pub fn update_settings(app: AppHandle<R>, settings: AppSettings) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function updateSettings(settings: AppSettings): Promise<void>
// invoke('update_settings', { settings })
```

---

### `apply_bypass_domains`

Persists a new bypass-domain list. If a VPN session is currently active, the running xray + TUN stack is torn down and restarted with the new list — otherwise edits in the UI silently do nothing until the user reconnects manually.

**Rust signature:**
```rust
pub fn apply_bypass_domains(app: AppHandle<R>, manager: State<'_, XrayManager>, domains: Vec<String>) -> Result<bool, String>
```

**TypeScript wrapper:**
```typescript
export async function applyBypassDomains(domains: string[]): Promise<boolean>
// invoke('apply_bypass_domains', { domains })
```

**Returns:** `true` if the live session was reloaded; `false` if only the setting was saved (no active session, or the new list is identical to the saved one).

**Error cases:**
- `"No last_server_id; cannot reload bypass without a known server"`
- `"Server with id <id> not found"`
- xray start/stop errors

---

### `get_logs`

Returns the in-memory ring buffer of xray log lines (capacity 1000).

**Rust signature:**
```rust
pub fn get_logs(manager: State<'_, XrayManager>) -> Result<Vec<LogEntry>, String>
```

**TypeScript wrapper:**
```typescript
export async function getLogs(): Promise<LogEntry[]>
// invoke('get_logs')
```

---

### `clear_logs`

Empties the in-memory log buffer. Does not stop log collection.

**Rust signature:**
```rust
pub fn clear_logs(manager: State<'_, XrayManager>) -> Result<(), String>
```

**TypeScript wrapper:**
```typescript
export async function clearLogs(): Promise<void>
// invoke('clear_logs')
```

---

## Error Handling Pattern

All Tauri commands return `Result<T, String>` in Rust. On the TypeScript side, a command failure causes `invoke()` to throw a string error. The stores and UI catch these errors:

```typescript
// In connectionStore
try {
  await connect(config);
} catch (err) {
  info = {
    ...info,
    status: 'error',
    error_message: err instanceof Error ? err.message : String(err)
  };
}
```

```typescript
// In +page.svelte
try {
  const imported = await servers.importFromJson(json);
  showToast(`Imported ${imported.length} server(s)`);
} catch (e) {
  showToast(`Import failed: ${e}`, 'error');
}
```

The UI surfaces errors as toast notifications (3-second auto-dismiss) or as a persistent error state in `StatusDisplay` when the connection itself is in error state.
