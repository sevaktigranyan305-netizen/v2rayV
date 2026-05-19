# v2rayV

[![Release](https://img.shields.io/github/v/release/sevaktigranyan305-netizen/v2rayV?display_name=tag)](https://github.com/sevaktigranyan305-netizen/v2rayV/releases)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Cross-platform desktop VPN client for my xray-core fork. Built with **Tauri v2** and **Svelte 5**

---

## Two operation modes

v2rayV picks the operating mode automatically from the share-link the user imports. There is no UI toggle — the URI tells the client what to do.

### L3 (virtual network) mode — preferred
The share-link contains `vnet=1&vnetIp=10.10.0.5/24` (and optionally `vnetSubnet`, `vnetMtu`, `vnetDefaultRoute`). v2rayV then:

- Generates a config with **no SOCKS5 / HTTP inbound** at all.
- Hands a `virtualNetwork{...}` block to the bundled xray-core fork.
- xray-core opens a TUN/utun/wintun adapter itself (see platform notes below).
- All system traffic is routed through that adapter; no system-proxy registry / `gsettings` / `networksetup` calls happen.
- The panel-allocated `vnetIp` is bound to the adapter so per-device usage is visible in [3x-ui](https://github.com/sevaktigranyan305-netizen/3x-ui).

### Legacy SOCKS + system-proxy mode
not sure about this socks one, devin didnt touched this at all i think
The share-link does not contain `vnet=1` (or `vnetIp` is empty). v2rayV falls back to:

- xray-core exposes SOCKS5 on `127.0.0.1:10808` and HTTP on `127.0.0.1:10809`.
- The OS-level proxy is pointed at those listeners (`gsettings` on Linux, registry on Windows, `networksetup` on macOS).
- (Optional, Linux only) a privileged helper runs `hev-socks5-tunnel` against the SOCKS port for full TUN-style behaviour.

Both modes use the same underlying VLESS+REALITY transport — only the local edge of the tunnel differs.

---

## Features

- **One-click connect / disconnect**, hard-kill on disconnect (no waiting for xray's logger drain).
- **VLESS + REALITY** (XTLS Vision, TLS 1.3) via [bundled xray-core fork](https://github.com/sevaktigranyan305-netizen/Xray-core).
- **L3 virtualnet mode** — no SOCKS5 inbound, no system proxy, xray owns the TUN. Per-device IPv4 from the panel.
- **Subscription persistence** — name + URL stored on disk; Refresh re-fetches and replaces only the servers from that subscription, leaving manually-added servers untouched.
- **`vless://` URI import / export** with full `vnet=`, `vnetIp=`, `vnetSubnet=`, `vnetMtu=`, `vnetDefaultRoute=` round-trip.
- **System tray** with Connect / Disconnect toggle and hide-to-tray on close.
- **Auto-connect on startup** to the last-used server.
- **Searchable log viewer** (`/logs`) with level filter and a Copy-to-clipboard button.
- **Dark / light theme** toggle.
- **Cross-platform** — Windows, macOS (Apple Silicon + Intel), Linux (x86_64).

## Platform notes

| Platform | TUN backend            | Privileges                                                                                                                       |
| -------- | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| Windows  | wintun                 | The installer triggers a UAC prompt on every launch (manifest declares `requireAdministrator`). Required by `WintunCreateAdapter`. |
| macOS    | utun                   | xray runs under `sudo -S`; the password is captured once via a modal and persisted in the login Keychain.                          |
| Linux    | /dev/net/tun           | xray runs under `sudo -S`; the password is captured once via a modal and persisted in the Secret Service.                          |

`wintun.dll` (v0.14, WireGuard) is bundled inside the Windows installer and copied next to `xray.exe` at first launch by `ensure_wintun_next_to_exe()` so the loader can find it.

### Linux runtime requirements

The L3 spawn path persists the sudo password in the OS credential
store via the Secret Service D-Bus protocol. Most desktop
environments ship a provider out of the box (GNOME Keyring on
GNOME/Cinnamon/XFCE, KWallet on KDE). Minimal/tiling-WM setups
(Hyprland, Sway, i3, niri, …) often have none — install one of the
providers below before launching v2rayV, otherwise the password
modal will keep re-appearing on every connect.

| Distribution    | Required packages                                          |
| --------------- | ---------------------------------------------------------- |
| Debian / Ubuntu | `libsecret-1-0 gnome-keyring procps` (or `kwalletmanager`) |
| Fedora / RHEL   | `libsecret gnome-keyring procps-ng`                        |
| Arch / Manjaro  | `libsecret gnome-keyring procps-ng`                        |

`procps` provides `pkill`, which the disconnect path uses to signal
the root-owned xray process. `sudo` (from any standard distribution)
is also required.

The keyring must be **unlocked** when v2rayV reads the saved
password — otherwise the Secret Service backend will block the
calling thread on a D-Bus prompt asking the user to unlock it. On
GNOME/Cinnamon/XFCE the keyring is unlocked at login via PAM, so
in practice this just works. On minimal/tiling WM setups you may
need to run `gnome-keyring-daemon --start --components=secrets`
(or the equivalent for KWallet / KeePassXC) before launching
v2rayV.

---

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (stable, ≥ 1.77.2)
- [Node.js](https://nodejs.org/) ≥ 22
- [pnpm](https://pnpm.io/) ≥ 9
- Linux system dependencies:
  ```bash
  # Arch / Manjaro
  sudo pacman -S webkit2gtk-4.1 libappindicator-gtk3 librsvg patchelf

  # Ubuntu / Debian
  sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
  ```

### Setup

```bash
git clone https://github.com/sevaktigranyan305-netizen/v2rayV.git
cd v2rayV

# Install frontend dependencies
pnpm install

# Download the xray-core fork binary (with wintun + L3 client) for your platform
./scripts/download-xray.sh

# Run in development mode
# Windows: must be launched from an elevated terminal — wintun won't open without it
pnpm tauri dev
```

### Production build

```bash
pnpm tauri build
```

The signed installer / bundle ends up under `src-tauri/target/release/bundle/`:
- Windows: `.msi` and NSIS `.exe` (both unsigned by default — SmartScreen will warn on first launch).
- macOS: `.dmg` and `.app.tar.gz` (unsigned by default).
- Linux: `.deb` and `.AppImage`.

### CI artifacts

Every PR triggers `cargo check` + `cargo clippy` + `pnpm check`. Pushes to `main` and tag pushes (`v*`) additionally trigger the **bundle** matrix that produces the per-platform installers and uploads them as 14-day GitHub Actions artifacts (`build.yml`) or attaches them to a public Release (`release.yml`).

---

## Development

```bash
pnpm tauri dev         # Full dev mode with hot reload
cargo test             # Run Rust tests (from src-tauri/) — currently 92 tests
cargo clippy --all-targets -- -D warnings
cargo fmt              # Rust formatting
pnpm check             # Svelte / TypeScript type checking
pnpm lint              # Frontend linting
pnpm format            # Frontend formatting
```

### xray-core fork

The xray-core sidecar (~30 MB per platform) is **not** committed. `scripts/download-xray.sh` pulls a pinned release of [our Xray-core fork](https://github.com/sevaktigranyan305-netizen/Xray-core/releases) into `src-tauri/binaries/`:

```bash
./scripts/download-xray.sh                # Pinned default version
./scripts/download-xray.sh v0.0.14-test   # Specific tag
```

The fork adds the `vless/l3client` outbound device backends (`device_windows.go` for wintun, `device_darwin.go` for utun, `device_linux.go` for native TUN) that L3 mode relies on. Upstream xray-core does not have them.

Supported sidecar binary names (Tauri's `tauri-plugin-shell` expects the `<target-triple>` suffix):

- `xray-x86_64-pc-windows-msvc.exe`
- `xray-x86_64-unknown-linux-gnu`
- `xray-aarch64-apple-darwin`
- `xray-x86_64-apple-darwin`

---

## Project structure

```
src-tauri/                Rust backend
  src/
    lib.rs                App builder, plugin registration, startup recovery
    commands.rs           Tauri IPC command handlers
    xray.rs               XrayManager — sidecar lifecycle, stats, logs, L3 gating
    config.rs             generate_client_config / generate_l3_config
    models.rs             ServerConfig (with virtualnet + subscription_id), Subscription, ...
    network.rs            Corporate VPN detection (legacy mode)
    storage.rs            Persistence (servers.json, subscriptions.json, settings.json)
    subscription.rs       HTTPS fetch + base64 decode for subscription URLs
    tray.rs               System tray integration
    uri.rs                vless:// URI parser + serializer (incl. vnet= params)
  manifest.xml            Windows UAC manifest (requireAdministrator)
  build.rs                Tauri build script + Windows manifest embedding
  binaries/               xray-core fork sidecar (gitignored)
  capabilities/           Tauri permissions

src/                      Svelte 5 frontend
  lib/
    components/           UI components (SubscriptionList, ServerList, LogViewer, ...)
    stores/               connection / servers / subscriptions / settings (Svelte 5 runes)
    api/tauri.ts          invoke() wrappers for all IPC commands
    types/index.ts        TypeScript interfaces mirroring Rust
    utils/                Formatting and platform utilities
  routes/
    +page.svelte          Main dashboard (subscriptions + servers + connect button)
    logs/+page.svelte     Searchable log viewer with Copy button

scripts/                  Build utilities
  download-xray.sh        Cross-platform xray-core fork binary downloader

docs/                     Documentation
  ARCHITECTURE.md         System design, IPC contract, state machine
  DEVELOPMENT.md          Setup guide
  API.md                  Tauri IPC API reference
  XRAY_CONFIG.md          REALITY protocol, vnet=/vnetIp= handling
```

---

## Server setup

v2rayV needs a VLESS+REALITY server. The fastest path is the [3x-ui panel fork](https://github.com/sevaktigranyan305-netizen/3x-ui) — it knows how to allocate per-device `vnetIp` addresses and emit share-links with `vnet=1&vnetIp=...` populated automatically.

For a quick manual setup, install xray-core on your VDS and configure a VLESS+REALITY inbound on port 443:

| Parameter      | Example                                     |
| -------------- | ------------------------------------------- |
| Server address | `45.151.233.107`                            |
| Port           | `443`                                       |
| UUID           | (generated with `xray uuid`)                |
| Public key     | (generated with `xray x25519`)              |
| Short ID       | (generated with `openssl rand -hex 8`)      |
| SNI            | `www.microsoft.com`                         |
| Fingerprint    | `chrome`                                    |

Servers can be added through the UI (Import → Single VLESS URI) or by saving a subscription URL.

### `vless://` URI format

Legacy (SOCKS+system-proxy mode):

```
vless://UUID@HOST:PORT?encryption=none&flow=xtls-rprx-vision&type=tcp&security=reality&sni=SNI&fp=chrome&pbk=PUBLIC_KEY&sid=SHORT_ID#NAME
```

L3 (virtualnet) mode adds the `vnet*` params:

```
vless://UUID@HOST:PORT?...&vnet=1&vnetIp=10.10.0.5/24&vnetSubnet=10.10.0.0/24&vnetDefaultRoute=1#NAME
```

Activation gating mirrors the Android client (`v2rayVN`): L3 mode requires **both** `vnet=1` AND a non-empty `vnetIp`. If either is missing, v2rayV falls back to legacy SOCKS+system-proxy mode for that server.

---

## Architecture (high-level)

```
┌─────────────┐     IPC      ┌────────────────┐   sidecar   ┌─────────────────┐
│  Svelte UI  │ ──invoke()──→│  Rust / Tauri   │ ──spawn()──→│  xray-core fork │
│  (WebView)  │ ←──events────│  Backend        │ ←──stdout───│  (VLESS+REALITY)│
└─────────────┘              └────────────────┘              └─────────────────┘
                                                                     │
                                                              REALITY tunnel
                                                                     │
                                                            ┌────────▼────────┐
                                                            │  3x-ui server   │
                                                            └─────────────────┘
```

In L3 mode, the xray-core fork additionally owns a TUN/utun/wintun adapter on the local machine and routes system traffic through it. In legacy mode, the OS proxy points at xray's local SOCKS/HTTP listeners.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full design.

---

## CI / CD

| Workflow      | Trigger              | Output                                                        |
| ------------- | -------------------- | ------------------------------------------------------------- |
| `build.yml`   | PR / push to `main`  | Lint matrix (Linux/Win/Mac) + bundle matrix (14-day artifacts) |
| `release.yml` | Tag push (`v*`)      | Multi-platform installers attached to a new GitHub Release     |

A release is cut by bumping the version in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` (must match), then pushing a `v<version>` tag.

---

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — system design, data flow, state machine, L3 vs legacy mode
- [Development](docs/DEVELOPMENT.md) — prerequisites, setup, project structure, sidecar lifecycle
- [API Reference](docs/API.md) — every IPC command with signatures and error cases
- [xray Config](docs/XRAY_CONFIG.md) — REALITY protocol, `virtualNetwork` block, URI format

## Related repositories

| Repo                                                                                              | Role                                       |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| [Xray-core](https://github.com/sevaktigranyan305-netizen/Xray-core)                               | xray-core fork with `l3client` device backends and bundled `wintun.dll` |
| [3x-ui](https://github.com/sevaktigranyan305-netizen/3x-ui)                                       | Server panel that allocates per-device `vnetIp` and emits the share-link |
| [v2rayNG / v2rayVN](https://github.com/sevaktigranyan305-netizen/v2rayNG)                         | Android client that consumes the same L3 share-links |
| [AndroidLibXrayLite](https://github.com/sevaktigranyan305-netizen/AndroidLibXrayLite)             | Go shim used by the Android client to embed the xray-core fork |

## License

MIT
