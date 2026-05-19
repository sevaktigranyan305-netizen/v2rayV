# Changelog

## [1.0.1] - 2026-05-19

### Added

- **Linux L3 mode** — Linux desktops can now run xray-core in L3 (`vnet=1`)
  mode the same way macOS does. xray is spawned under `sudo -S` so it can
  claim `/dev/net/tun`; the sudo password is captured once via the existing
  modal and persisted in the OS Secret Service (gnome-keyring, KWallet,
  KeePassXC, …). SOCKS-mode servers are refused up front on Linux with a
  clear message — matching the macOS gate.
- **Native arm64 Linux build** — `aarch64-unknown-linux-gnu` is now a
  first-class CI target (both `check` and `bundle` matrices). arm64 Linux
  users no longer need Rosetta, and Apple Silicon Parallels Desktop VMs
  running arm64 Ubuntu get a native binary that doesn't trip Rosetta's
  AOT segment-count limit.

### macOS

- **Disconnect now actually terminates xray** — sudo's pty monitor calls
  `setsid()` on the child, so `killpg(pgid)` only reaped the sudo wrapper
  and left xray running as an orphan. Disconnect now does
  `sudo -S pkill -9 -f <xray_path>` (path resolved at spawn time, no
  hard-coding) and treats a signal-killed sudo wrapper as success.
- **App leaves the Dock when its window closes** — closing the main
  window now demotes the activation policy to `Accessory`. Clicking the
  tray icon promotes it back to `Regular` before re-showing the window.
- **Template tray icon** — the menu-bar icon is now a monochrome paw
  with `set_icon_as_template(true)`, so macOS recolors it for light /
  dark menubars (Windows tray is unchanged).
- **Tray Connect/Disconnect syncs with the in-window UI** — fixed a
  case where refreshing a subscription generated new internal server
  IDs and silently invalidated the stored `last_server_id`, making the
  tray's Connect button behave like Show Window. IDs are now preserved
  across refresh, and the tray Connect path delegates to the same
  frontend handler as the in-window button.
- **`Copy vless:// URL` works** — replaced `navigator.clipboard.writeText`
  (lost its user-activation gesture across IPC awaits) with the Tauri
  clipboard plugin. Logs view migrated to the same plugin.

### Windows

- **NSIS uninstaller cleans up `wintun.dll`** — installer hooks now
  remove the runtime-copied `wintun.dll` next to `xray.exe` and the
  install directory when empty, so uninstall no longer leaves orphaned
  files behind.

### Fixed

- **Auto-connect on startup no longer bypasses the L3 / sudo gate**
  on macOS and Linux. A stale `last_server_id` pointing at a SOCKS
  server or a missing credential is now caught up-front with a clear
  log line instead of silently falling through to the Tauri sidecar.

## [0.6.0] - 2026-03-15

### Fixed

- **Corporate VPN DNS resolution in TUN mode** — Internal corporate hostnames (e.g.
  `gitlab-paygate.paywb.info`) that only exist on corporate DNS now resolve correctly.
  xray detects corporate DNS servers from `/etc/resolv.conf` and queries them with
  `expectIPs` scoped to VPN-routed subnets, accepting both private and public IPs
  returned by corporate DNS.

- **Corporate VPN traffic source IP** — Traffic to corporate subnets (including
  public IPs routed through the corporate VPN) was being sent with the wrong source IP
  (LAN IP via `sendThrough`) causing the corporate server to reject connections. A new
  `direct-vpn` xray outbound without `sendThrough` lets the kernel assign the correct
  VPN-assigned source IP via `ip rule to SUBNET lookup main`.

- **Corporate VPN breaks after RustVPN disconnect** — NetworkManager was recalculating
  routing and DNS for all connections when the RustVPN TUN device (`rvpn0`) appeared
  or disappeared, corrupting corporate VPN routes and dropping its DNS servers from
  `/etc/resolv.conf`. Fixed by marking `rvpn0` as unmanaged (`nmcli device set rvpn0
  managed no`) and restoring `/etc/resolv.conf` + reloading NM DNS on TUN teardown.

- **RFC-1918 traffic bypass** — When a corporate VPN is active, all RFC-1918 ranges are
  now added to the kernel `ip rule` bypass list, ensuring corporate DNS servers and
  LAN resources use the main routing table instead of the RustVPN TUN.

---

## [0.5.0] - 2026-03-07

### Fixed

- **TUN mode DNS hang** — Dropped `localhost` from xray DNS in TUN mode; only use
  `1.1.1.1` and `8.8.8.8` to avoid 30-second hangs caused by corporate VPN DNS
  being unroutable through the TUN.

- **TUN routing loops** — Added source-based policy routing (`ip rule add from
  LOCAL_IP lookup main`) so xray's `sendThrough` traffic exits via the physical
  interface rather than looping back through the TUN.

- **TUN cleanup on crash** — Added watchdog process and startup cleanup to remove
  stale TUN devices left behind by app crashes.

---

## [0.4.0] - 2026-02-20

### Added

- TUN mode for Linux — routes all traffic through the VPN, not just browser traffic.
- Corporate VPN detection — automatically detects active VPN interfaces and their
  subnets to configure bypass routing.
- Speed statistics — real-time upload/download counters via xray gRPC stats API.

### Fixed

- VLESS server IP bypass route — prevents the VLESS server itself from being routed
  through the TUN, which would create a routing loop.
