# v2rayV — Developer Setup Guide

## Prerequisites

### Required tools

| Tool | Version | Install |
|------|---------|---------|
| Rust + Cargo | >= 1.77.2 | `curl https://sh.rustup.rs -sSf \| sh` |
| Node.js | >= 18 | via system package manager or nvm |
| pnpm | >= 9 | `npm install -g pnpm` |
| Tauri CLI (bundled) | 2.x | installed as dev dependency via pnpm |

### System dependencies (Linux)

Tauri requires several system libraries for the WebView and desktop integration:

```bash
# Debian/Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev patchelf polkit

# Arch / Manjaro
sudo pacman -S webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg polkit
```

Refer to the [official Tauri prerequisites](https://tauri.app/start/prerequisites/) for macOS and Windows.

### Privileges per platform

| Platform | What needs privileges                          | How v2rayV handles it                                                                                  |
| -------- | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Windows  | `WintunCreateAdapter()` for L3 mode             | The `.exe` embeds a UAC manifest (`src-tauri/manifest.xml`) declaring `requireAdministrator`. UAC prompt fires on every launch. For `pnpm tauri dev`, run the terminal as Administrator. |
| macOS    | utun in L3 mode                                 | Run with `sudo` for now, or wire up your own privileged helper. Not yet smoothed out.                   |
| Linux    | Native TUN in L3 mode                           | Either give xray `CAP_NET_ADMIN`, or stay in legacy SOCKS-only mode.                                    |
| Linux    | Legacy SOCKS+TUN via `hev-socks5-tunnel`        | `sudo ./scripts/install-helper.sh` installs `/usr/local/sbin/v2rayv-helper` + a polkit rule, invoked via `pkexec` at runtime. |

Legacy Linux helper install (only needed if you want SOCKS-mode TUN; L3 mode does not use this path):

```bash
sudo ./scripts/install-helper.sh
```

This places `/usr/local/sbin/v2rayv-helper` and the policy file from `polkit/`. Without it, the app stays in plain system-proxy mode (works for most apps, but not every TCP/UDP source).

### xray-core fork binary (required at runtime)

v2rayV uses a [forked xray-core](https://github.com/sevaktigranyan305-netizen/Xray-core) that adds the `vless/l3client` outbound device backends (`device_windows.go` for wintun, `device_darwin.go` for utun, `device_linux.go` for native TUN). Upstream xray-core does not have those.

The binary is not committed. It must be placed at:

```
src-tauri/binaries/xray-<target-triple>
```

where `<target-triple>` is your platform identifier, for example:

- Linux x86_64: `xray-x86_64-unknown-linux-gnu`
- macOS Apple Silicon: `xray-aarch64-apple-darwin`
- Windows x86_64: `xray-x86_64-pc-windows-msvc.exe`

The convenient way is to use the bundled downloader, which pulls from [our fork's Releases](https://github.com/sevaktigranyan305-netizen/Xray-core/releases) and saves with the correct sidecar suffix:

```bash
./scripts/download-xray.sh                  # Pinned default version
./scripts/download-xray.sh v0.0.14-test     # Specific tag
```

On Windows the release zip also contains `wintun.dll`, which the script places under `src-tauri/binaries/`. Tauri then bundles it as a Windows resource; at runtime `lib.rs::ensure_wintun_next_to_exe()` copies it next to the running `.exe` so the loader can find it.

## Clone and Run

```bash
git clone <repo-url>
cd v2rayV

# Install frontend dependencies
pnpm install

# Start development mode (builds frontend + Rust backend, opens app window)
pnpm tauri dev
```

`pnpm tauri dev` runs `pnpm dev` (Vite dev server on http://localhost:5173) and the Tauri Rust backend concurrently. The app window connects to the Vite dev server for hot module reload.

**Windows note**: dev mode must be launched from an elevated terminal (`Run as administrator`), otherwise wintun adapter creation will fail with `ERROR_ACCESS_DENIED` whenever you connect to a vnet=1 server. Production installer-installed builds prompt for UAC automatically via the embedded manifest.

## Releases

The version is duplicated in three files — they must match before tagging:

```
package.json                    "version": "x.y.z"
src-tauri/Cargo.toml            version = "x.y.z"
src-tauri/tauri.conf.json       "version": "x.y.z"
```

Then:

```bash
git tag vX.Y.Z
git push --tags
```

`release.yml` builds and uploads `.msi` / `.exe` / `.dmg` / `.AppImage` / `.deb` artifacts to the new GitHub Release.

## Available Commands

### Development

```bash
pnpm tauri dev          # Run full app in dev mode (hot reload)
pnpm dev                # Frontend dev server only (Vite, port 5173)
```

### Building

```bash
pnpm tauri build        # Production build: compiles Rust in release mode,
                        # bundles frontend, produces installer/AppImage
```

### Frontend

```bash
pnpm check              # svelte-check type checking
pnpm check:watch        # Type check in watch mode
```

### Rust

Run from `src-tauri/`:

```bash
cargo test              # Run all Rust unit tests
cargo clippy            # Lint (must be clean, no warnings)
cargo fmt               # Auto-format Rust code
```

## Project Structure

```
v2rayV/
├── src-tauri/                    # Rust backend (Tauri)
│   ├── src/
│   │   ├── main.rs               # Binary entry point
│   │   ├── lib.rs                # Tauri builder, plugin registration, startup recovery, command handler
│   │   ├── models.rs             # ServerConfig, RealitySettings, ConnectionInfo, SpeedStats,
│   │   │                         #   LogEntry, AppSettings, DetectedVpn, AppError
│   │   ├── commands.rs           # All #[tauri::command] handlers
│   │   ├── xray.rs               # XrayManager: sidecar lifecycle, stats poller, log buffer
│   │   ├── config.rs             # generate_client_config()
│   │   ├── network.rs            # Corporate VPN detection (ip -j route show), DNS scrape
│   │   ├── proxy.rs              # System proxy enable/disable (Linux/Win/macOS) — desktop only, legacy mode
│   │   ├── tun.rs                # Linux TUN mode via v2rayv-helper / pkexec — legacy mode only
│   │   ├── tray.rs               # System tray menu (desktop only)
│   │   ├── storage.rs            # Load/save servers.json + subscriptions.json + settings.json
│   │   ├── subscription.rs       # HTTPS fetch + base64 decode + per-line vless:// parsing
│   │   └── uri.rs                # VLESS URI parse and serialize (incl. vnet= params)
│   ├── manifest.xml              # Windows UAC manifest (requireAdministrator)
│   ├── build.rs                  # Tauri build script + Windows manifest embedding
│   ├── binaries/
│   │   └── xray-<triple>         # xray-core binary (gitignored)
│   ├── icons/                    # App icons for all platforms
│   ├── Cargo.toml                # Rust dependencies
│   └── tauri.conf.json           # Tauri configuration (window, bundle, sidecar)
│
├── scripts/                      # Helper installer + xray downloader
│   ├── install-helper.sh         # Installs v2rayv-helper for Linux TUN mode
│   ├── v2rayv-helper            # The privileged TUN helper itself
├── polkit/                       # polkit rule for v2rayv-helper
│
├── src/                          # Svelte 5 + SvelteKit frontend
│   ├── routes/
│   │   ├── +layout.ts            # prerender=true, ssr=false
│   │   ├── +layout.svelte        # Root layout (CSS, favicon)
│   │   ├── +page.svelte          # Main page: composes all components
│   │   └── logs/+page.svelte     # Log viewer route
│   ├── lib/
│   │   ├── api/
│   │   │   └── tauri.ts          # invoke() wrappers for all Tauri commands
│   │   ├── types/
│   │   │   └── index.ts          # TypeScript interfaces (mirrors Rust structs)
│   │   ├── stores/
│   │   │   ├── connection.svelte.ts      # Connection state, polling, speed stats, 8 s disconnect watchdog
│   │   │   ├── servers.svelte.ts         # Server CRUD + selection + import/export
│   │   │   ├── subscriptions.svelte.ts   # Subscription list + add/refresh/delete
│   │   │   └── settings.svelte.ts        # AppSettings with rollback-on-save-failure
│   │   ├── components/
│   │   │   ├── ConnectButton.svelte
│   │   │   ├── StatusDisplay.svelte
│   │   │   ├── ServerList.svelte             # Manually-added servers only
│   │   │   ├── ServerForm.svelte
│   │   │   ├── SubscriptionList.svelte       # Saved subscriptions + nested servers + 25 s refresh watchdog
│   │   │   ├── SubscriptionModal.svelte      # Add subscription form (autofill suppressed)
│   │   │   ├── ImportExportBar.svelte        # Top-right header toolbar
│   │   │   ├── UriInputModal.svelte
│   │   │   ├── LogViewer.svelte              # Tail of in-memory log buffer + Copy button
│   │   │   ├── ThemeToggle.svelte
│   │   │   └── ui/                           # shadcn-svelte primitives
│   │   ├── hooks/                # (reserved)
│   │   ├── assets/
│   │   │   └── favicon.svg
│   │   ├── utils/
│   │   │   └── index.ts          # cn() = clsx + tailwind-merge
│   │   └── index.ts              # Barrel export
│   ├── app.css                   # Tailwind CSS base styles + CSS variables
│   ├── app.d.ts                  # SvelteKit ambient types
│   └── app.html                  # HTML shell
│
├── docs/                         # Project documentation
├── .claude/                      # Claude Code agents/skills/hooks
├── creds/                        # Per-host VLESS credentials (gitignored)
├── release-assets/               # Per-platform release artifacts (gitignored)
├── svelte.config.js              # SvelteKit adapter-static config
├── vite.config.ts                # Vite + Tailwind plugin config
├── tsconfig.json                 # TypeScript config
├── package.json                  # Frontend scripts and dependencies
└── pnpm-lock.yaml
```

## How the xray Sidecar Works

### Binary naming convention

Tauri's `tauri-plugin-shell` sidecar feature expects the binary to be named with the target triple suffix. At build time the CLI resolves your current target triple and looks for:

```
src-tauri/binaries/xray-<target-triple>[.exe on Windows]
```

This is configured in `tauri.conf.json`:

```json
"bundle": {
  "externalBin": ["binaries/xray"]
}
```

The `"binaries/xray"` entry is the base name; Tauri automatically appends the triple.

### Runtime lifecycle

1. `XrayManager::start()` calls `app.shell().sidecar("xray")` to get a managed sidecar handle.
2. The sidecar is launched with args `["run", "-c", "<path_to_config>"]`.
3. xray logs to stderr. The manager monitors stderr for the string `"started"` to detect successful startup.
4. `XrayManager::stop()` calls `child.kill()` and removes the temporary config file from `app_data_dir`.

### Config file location

The generated xray JSON config is written to:

```
<app_data_dir>/xray_config.json
```

On Linux this is typically `~/.local/share/com.v2rayv.app/xray_config.json`. The file is deleted on disconnect.

### Server list, subscriptions and settings storage

Three JSON files are persisted in the OS app config directory:

```
<app_config_dir>/servers.json         # Vec<ServerConfig> (each carries optional virtualnet + subscription_id)
<app_config_dir>/subscriptions.json   # Vec<Subscription> (id, name, url, last_updated_at, last_server_count)
<app_config_dir>/settings.json        # AppSettings (auto_connect, last_server_id, bypass_domains)
```

On Linux: `~/.config/com.v2rayv.app/`. On Windows: `%APPDATA%\com.v2rayv.app\`. All files are written with mode 0600 on Unix.

On startup, `lib.rs` reads `settings.json` and, if `auto_connect` is true, immediately reconnects to `last_server_id`.

All storage I/O in async commands is wrapped in `tauri::async_runtime::spawn_blocking` so file writes (which can be slow on Windows under AV scanning) don't block the IPC worker thread.

### Hide-to-tray and auto-connect

Closing the main window does **not** quit the app — `lib.rs` intercepts `WindowEvent::CloseRequested`, calls `prevent_close()`, and hides the window. The system tray (configured in `tray.rs`) keeps the connection alive in the background. Use the tray's **Quit** entry to actually exit, or send `SIGINT` via the terminal during `pnpm tauri dev`.

## Tauri Plugins Used

| Plugin | Purpose |
|--------|---------|
| `tauri-plugin-shell` | Spawns xray sidecar process (desktop only) |
| `tauri-plugin-dialog` | Open/save file dialogs for JSON import/export |
| `tauri-plugin-fs` | Read/write files for JSON import/export |
| `tauri-plugin-log` | Structured logging (debug builds only) |

The `tauri` crate itself is enabled with the `tray-icon` feature so `tray.rs` can register a system tray menu on desktop builds.

## Key Rust Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tauri` | 2.10.0 (feat. `tray-icon`) | Desktop app framework |
| `tauri-build` | 2.5.4 | Build-script support for Tauri |
| `serde` / `serde_json` | 1.0 | JSON serialization |
| `thiserror` | 2 | Ergonomic error types |
| `uuid` | 1 (v4 feature) | UUID generation for server IDs |
| `log` | 0.4 | Logging facade |

## Key Frontend Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| `svelte` | 5.x | UI framework (runes API) |
| `@sveltejs/kit` | 2.x | App framework / routing |
| `tailwindcss` | 4.x | Utility-first CSS |
| `@tauri-apps/api` | 2.x | `invoke()` and Tauri JS API |
| `@tauri-apps/plugin-dialog` | 2.x | JS bindings for dialog plugin |
| `@tauri-apps/plugin-fs` | 2.x | JS bindings for fs plugin |
| `clsx` + `tailwind-merge` | latest | Class name utility (`cn()`) |
