use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

use crate::config;
use crate::config::generate_client_config;
#[cfg(target_os = "macos")]
use crate::macos_helper;
#[cfg(target_os = "macos")]
use crate::macos_xray;
use crate::models::{
    AppError, ConnectionInfo, ConnectionStatus, DetectedVpn, LogEntry, ServerConfig, SpeedStats,
};
use crate::network;
use crate::proxy;
#[cfg(target_os = "linux")]
use crate::tun;

const DEFAULT_SOCKS_PORT: u16 = 10808;
const MAX_LOG_ENTRIES: usize = 1000;

pub struct XrayManager {
    child: Arc<Mutex<Option<CommandChild>>>,
    /// macOS-only sudo+xray handle. Lives next to `child` (which stays
    /// None on macOS) so the rest of XrayManager doesn't need to know
    /// which spawn path produced the running process.
    #[cfg(target_os = "macos")]
    macos_child: Arc<Mutex<Option<macos_xray::SudoChild>>>,
    state: Arc<Mutex<ConnectionInfo>>,
    config_path: Arc<Mutex<Option<std::path::PathBuf>>>,
    stats: Arc<Mutex<SpeedStats>>,
    prev_uplink: Arc<Mutex<u64>>,
    prev_downlink: Arc<Mutex<u64>>,
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
    bypass_domains: Arc<Mutex<Vec<String>>>,
    bypass_subnets: Arc<Mutex<Vec<String>>>,
    detected_vpns: Arc<Mutex<Vec<DetectedVpn>>>,
    /// Whether the *current* connection is L3 (virtualnet). Set in `start`
    /// and cleared in `stop_desktop` so that the stop path can skip
    /// system-proxy disable / TUN cleanup that was never set up.
    l3_mode: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for XrayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl XrayManager {
    pub fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "macos")]
            macos_child: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ConnectionInfo::default())),
            config_path: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(SpeedStats::default())),
            prev_uplink: Arc::new(Mutex::new(0)),
            prev_downlink: Arc::new(Mutex::new(0)),
            logs: Arc::new(Mutex::new(VecDeque::new())),
            bypass_domains: Arc::new(Mutex::new(Vec::new())),
            bypass_subnets: Arc::new(Mutex::new(Vec::new())),
            detected_vpns: Arc::new(Mutex::new(Vec::new())),
            l3_mode: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Return the last detected VPN interfaces.
    pub fn get_detected_vpns(&self) -> Vec<DetectedVpn> {
        self.detected_vpns.lock().unwrap().clone()
    }

    pub fn status(&self) -> ConnectionInfo {
        self.state.lock().unwrap().clone()
    }

    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }

    pub fn clear_logs(&self) {
        self.logs.lock().unwrap().clear();
    }

    pub fn start<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        server: &ServerConfig,
        bypass_domains: &[String],
    ) -> Result<(), AppError> {
        // Don't start if already running
        {
            let current = self.state.lock().unwrap();
            if current.status == ConnectionStatus::Connected
                || current.status == ConnectionStatus::Connecting
            {
                return Err(AppError::XrayProcess(
                    "Already connected or connecting".to_string(),
                ));
            }
        }

        // Reset stats counters for new connection
        self.reset_stats();

        // Update status to connecting
        self.update_status(ConnectionStatus::Connecting, Some(server), None);

        if let Err(e) = self.start_desktop(app, server, bypass_domains) {
            self.update_status(ConnectionStatus::Error, None, Some(e.to_string()));
            return Err(e);
        }

        Ok(())
    }

    // The allow attributes here apply only on macOS: when
    // target_os = "macos" the function returns inside the cfg block
    // below and clippy/rustc would otherwise flag the Tauri-sidecar
    // path as unreachable / its locals as unused. The sidecar path is
    // still type-checked on every target, which is what we want.
    #[cfg_attr(
        target_os = "macos",
        allow(unreachable_code, unused_variables, unused_mut, unused_assignments)
    )]
    fn start_desktop<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        server: &ServerConfig,
        bypass_domains: &[String],
    ) -> Result<(), AppError> {
        // Kill any stale xray process from a previous run
        {
            let mut guard = self.child.lock().unwrap();
            if let Some(child) = guard.take() {
                let _ = child.kill();
                info!("Killed stale xray process");
            }
        }
        #[cfg(target_os = "macos")]
        {
            let mut guard = self.macos_child.lock().unwrap();
            if let Some(sc) = guard.take() {
                if let Err(e) = macos_xray::stop_sudo_child(&sc) {
                    warn!("Failed to kill stale sudo+xray process group: {e}");
                } else {
                    info!("Killed stale sudo+xray process group");
                }
            }
        }

        // L3 (virtualnet) mode lets xray-core's `l3client` own the TUN
        // adapter directly, so we skip system-proxy and the
        // hev-socks5-tunnel sidecar entirely. Threaded into the stdout
        // monitor and the Linux TUN launcher below so they branch on it.
        let l3_mode = config::virtualnet_enabled(server).is_some();
        self.l3_mode
            .store(l3_mode, std::sync::atomic::Ordering::Release);

        // Detect corporate VPN interfaces and bypass subnets
        let vpns = network::detect_vpn_routes();
        let bypass_subnet_list = network::collect_bypass_subnets(&vpns);

        // Store detected VPNs and bypass subnets
        {
            let mut dv = self.detected_vpns.lock().unwrap();
            *dv = vpns;
        }
        {
            let mut bs = self.bypass_subnets.lock().unwrap();
            *bs = bypass_subnet_list.clone();
        }

        // Store bypass domains for proxy setup
        {
            let mut bd = self.bypass_domains.lock().unwrap();
            *bd = bypass_domains.to_vec();
        }

        // Detect physical interface gateway and IP (for TUN routing on Linux)
        #[cfg(target_os = "linux")]
        let gateway_info = network::detect_default_gateway_and_ip();

        #[cfg(target_os = "linux")]
        let send_through = gateway_info.as_ref().map(|(_, _, ip)| ip.as_str());
        #[cfg(not(target_os = "linux"))]
        let send_through: Option<&str> = None;

        // Detect corporate VPN DNS servers from resolv.conf (private IPs only).
        // Filter out DNS servers inside VPN-routed subnets — xray can't reach them
        // correctly with sendThrough (wrong source IP). Only LAN-reachable DNS
        // (e.g. home router) survives. With the direct-vpn outbound (no sendThrough),
        // DNS to private IPs is routed via `ip rule to SUBNET lookup main`, using
        // the correct VPN-assigned source IP.
        #[cfg(target_os = "linux")]
        let vpn_dns_servers = if !bypass_subnet_list.is_empty() {
            let detected = network::detect_vpn_dns_servers();
            if !detected.is_empty() {
                info!("Detected corporate VPN DNS servers: {:?}", detected);
            }
            detected
        } else {
            Vec::new()
        };
        #[cfg(not(target_os = "linux"))]
        let vpn_dns_servers: Vec<String> = Vec::new();

        // Generate xray config
        let config_json = generate_client_config(
            server,
            DEFAULT_SOCKS_PORT,
            bypass_domains,
            &bypass_subnet_list,
            send_through,
            &vpn_dns_servers,
        )?;

        // Write config to temp file
        let config_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| AppError::Config(format!("Failed to get app data dir: {e}")))?;
        std::fs::create_dir_all(&config_dir)?;
        let config_file = config_dir.join("xray_config.json");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&config_file)?;
            f.write_all(config_json.as_bytes())?;
            // Fsync so a crash between write and spawn can't leave xray reading
            // a truncated/empty file on the next run.
            f.sync_all()?;
        }

        info!("Wrote xray config to {}", config_file.display());

        // Store config path for cleanup
        {
            let mut path = self.config_path.lock().unwrap();
            *path = Some(config_file.clone());
        }

        // On macOS we have to launch xray under `sudo -S` so it can
        // open a utun device for L3 mode. The cross-platform
        // Tauri-sidecar spawn that follows inherits the GUI user's
        // privileges and would fail with EPERM on utun creation, so we
        // short-circuit here and let `start_desktop_macos` run xray via
        // std::process::Command instead. The macOS path is also the
        // only one that uses the Keychain-cached sudo password.
        #[cfg(target_os = "macos")]
        {
            self.start_desktop_macos(app, server, &config_file)?;
            return Ok(());
        }

        // Create sidecar command
        let config_path_str = config_file.to_string_lossy().to_string();
        let command = app
            .shell()
            .sidecar("xray")
            .map_err(|e| AppError::XrayProcess(format!("Failed to create sidecar command: {e}")))?
            .args(["run", "-c", &config_path_str]);

        // Spawn the process
        let (mut rx, child_process) = command
            .spawn()
            .map_err(|e| AppError::XrayProcess(format!("Failed to spawn xray: {e}")))?;

        info!("Spawned xray process with PID {}", child_process.pid());

        // Store child handle
        {
            let mut guard = self.child.lock().unwrap();
            *guard = Some(child_process);
        }

        // Monitor output in background
        let state = self.state.clone();
        let child_ref = self.child.clone();
        let logs_ref = self.logs.clone();
        let bypass_ref = self.bypass_domains.clone();
        let bypass_subnets_ref = self.bypass_subnets.clone();
        let app_handle = app.clone();
        let server_name = server.name.clone();
        let server_address = server.address.clone();

        // TUN mode data (Linux only)
        #[cfg(target_os = "linux")]
        let tun_data = {
            let exe = std::env::current_exe()
                .map_err(|e| AppError::Config(format!("Failed to get exe path: {e}")))?;
            let exe_dir = exe.parent().unwrap();
            let sidecar_name = "hev-socks5-tunnel-x86_64-unknown-linux-gnu";
            let path = exe_dir.join(sidecar_name);
            let hev_bin = if path.exists() {
                path
            } else {
                // Dev mode: try binaries directory
                let dev_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("binaries")
                    .join(sidecar_name);
                if dev_path.exists() {
                    dev_path
                } else {
                    warn!("hev-socks5-tunnel not found, TUN mode unavailable");
                    path
                }
            };
            (
                hev_bin,
                config_dir.clone(),
                server.address.clone(),
                bypass_subnet_list.clone(),
                gateway_info.clone(),
            )
        };

        // Clone refs for post-connection verification
        let verify_logs = self.logs.clone();
        let verify_state = self.state.clone();
        let verify_stats = self.stats.clone();

        // Shared flag for timeout coordination
        let started_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_flag_clone = started_flag.clone();
        let started_flag_verify = started_flag.clone();
        let started_flag_fallback = started_flag.clone();
        #[cfg(target_os = "linux")]
        let started_flag_tun = started_flag.clone();

        // True once we've seen at least one line of xray output. Used by
        // the fallback timer below to distinguish "xray is alive and
        // emitting logs but we missed the magic word" from "xray crashed
        // before printing anything".
        let output_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let output_seen_stdout = output_seen.clone();
        let output_seen_stderr = output_seen.clone();

        // Connection timeout: kill xray if not started within 15 seconds
        let timeout_state = self.state.clone();
        let timeout_child = self.child.clone();
        let timeout_logs = self.logs.clone();
        let timeout_app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(15));
            if !started_flag_clone.load(std::sync::atomic::Ordering::Acquire) {
                let mut s = timeout_state.lock().unwrap();
                if s.status == ConnectionStatus::Connecting {
                    warn!("Connection timeout after 15 seconds");
                    push_log_entry(
                        &timeout_logs,
                        "error",
                        "Connection timeout after 15 seconds",
                    );
                    s.status = ConnectionStatus::Error;
                    s.error_message = Some(
                        "Connection timeout — server unreachable or config invalid".to_string(),
                    );
                    s.connected_since = None;
                    drop(s);
                    // Kill the xray process
                    let child = { timeout_child.lock().unwrap().take() };
                    if let Some(child) = child {
                        let _ = child.kill();
                    }
                    let _ = timeout_app.emit("connection-status-changed", "disconnected");
                }
            }
        });

        // Fallback connection trigger: if after 6 seconds we've seen at
        // least one line of xray output but haven't observed the magic
        // "started" word (e.g. because pipe buffering on Windows
        // re-ordered the line, or xray-core changed the message), promote
        // Connecting → Connected anyway. The 15-second timeout above
        // still fires if xray produces NO output, so a truly broken
        // process still gets cleaned up.
        let fallback_state = self.state.clone();
        let fallback_logs = self.logs.clone();
        let fallback_app = app.clone();
        let fallback_server_name = server.name.clone();
        let fallback_server_address = server.address.clone();
        let fallback_l3_mode = l3_mode;
        let fallback_bypass_ref = self.bypass_domains.clone();
        let fallback_bypass_subnets_ref = self.bypass_subnets.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(6));
            if started_flag_fallback.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            if !output_seen.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            if mark_connected(
                &started_flag_fallback,
                &fallback_state,
                &fallback_server_name,
                &fallback_server_address,
            ) {
                warn!(
                    "xray fallback connection trigger: 'started' log not observed but \
                     xray is producing output, marking Connected"
                );
                push_log_entry(
                    &fallback_logs,
                    "info",
                    "Marking Connected via fallback (no 'started' log observed)",
                );
                if !fallback_l3_mode {
                    let domains = fallback_bypass_ref.lock().unwrap().clone();
                    let subnets = fallback_bypass_subnets_ref.lock().unwrap().clone();
                    proxy::enable_system_proxy(DEFAULT_SOCKS_PORT, &domains, &subnets);
                }
                let _ = fallback_app.emit("connection-status-changed", "connected");
            }
        });

        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(line) => {
                        let line_str = String::from_utf8_lossy(&line);
                        let trimmed = line_str.trim();
                        info!("xray stdout: {}", trimmed);
                        push_log_entry(&logs_ref, "info", trimmed);
                        output_seen_stdout.store(true, std::sync::atomic::Ordering::Release);

                        if !started_flag.load(std::sync::atomic::Ordering::Acquire)
                            && line_signals_started(trimmed)
                            && mark_connected(&started_flag, &state, &server_name, &server_address)
                        {
                            info!("xray connected successfully (detected from stdout)");
                            if !l3_mode {
                                let domains = bypass_ref.lock().unwrap().clone();
                                let subnets = bypass_subnets_ref.lock().unwrap().clone();
                                proxy::enable_system_proxy(DEFAULT_SOCKS_PORT, &domains, &subnets);
                            }
                            let _ = app_handle.emit("connection-status-changed", "connected");
                        }
                    }
                    CommandEvent::Stderr(line) => {
                        let line_str = String::from_utf8_lossy(&line);
                        let trimmed = line_str.trim();
                        info!("xray stderr: {}", trimmed);
                        output_seen_stderr.store(true, std::sync::atomic::Ordering::Release);

                        let level = if trimmed.contains("[Warning]") {
                            "warning"
                        } else if trimmed.contains("[Error]") {
                            "error"
                        } else {
                            "info"
                        };
                        push_log_entry(&logs_ref, level, trimmed);

                        if !started_flag.load(std::sync::atomic::Ordering::Acquire)
                            && line_signals_started(trimmed)
                            && mark_connected(&started_flag, &state, &server_name, &server_address)
                        {
                            info!("xray connected successfully");
                            if !l3_mode {
                                let domains = bypass_ref.lock().unwrap().clone();
                                let subnets = bypass_subnets_ref.lock().unwrap().clone();
                                proxy::enable_system_proxy(DEFAULT_SOCKS_PORT, &domains, &subnets);
                            }
                            let _ = app_handle.emit("connection-status-changed", "connected");
                        }
                    }
                    CommandEvent::Error(err) => {
                        error!("xray error event: {}", err);
                        push_log_entry(&logs_ref, "error", &err);
                    }
                    CommandEvent::Terminated(payload) => {
                        warn!(
                            "xray terminated with code: {:?}, signal: {:?}",
                            payload.code, payload.signal
                        );
                        let msg = format!(
                            "xray terminated (code: {:?}, signal: {:?})",
                            payload.code, payload.signal
                        );
                        push_log_entry(&logs_ref, "warning", &msg);

                        if !l3_mode {
                            proxy::disable_system_proxy();
                        }

                        let mut s = state.lock().unwrap();
                        if s.status == ConnectionStatus::Disconnecting {
                            s.status = ConnectionStatus::Disconnected;
                        } else if s.status != ConnectionStatus::Disconnected
                            && s.status != ConnectionStatus::Error
                        {
                            s.status = ConnectionStatus::Error;
                            s.error_message = Some(format!(
                                "xray exited unexpectedly (code: {:?})",
                                payload.code
                            ));
                        }
                        s.connected_since = None;

                        let mut c = child_ref.lock().unwrap();
                        *c = None;
                        drop(s);
                        drop(c);
                        let _ = app_handle.emit("connection-status-changed", "disconnected");
                        break;
                    }
                    _ => {}
                }
            }
        });

        // Post-connection verification: check SOCKS proxy and traffic flow.
        // In L3 (virtualnet) mode there is no SOCKS5 inbound at all —
        // xray-core's l3client owns the TUN and traffic never goes through
        // 127.0.0.1:10808. Probing for a non-existent listener would just
        // emit a misleading "SOCKS5 proxy NOT reachable" warning every
        // single connect, so skip the verify thread entirely.
        let verify_l3_mode = l3_mode;
        std::thread::spawn(move || {
            // Wait for connection to be established (up to 20s)
            for _ in 0..40 {
                if started_flag_verify.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if !started_flag_verify.load(std::sync::atomic::Ordering::Acquire) {
                return; // Timeout thread already handled this
            }
            if verify_l3_mode {
                info!("[verify] L3 mode active — skipping SOCKS5/system-proxy probes");
                return;
            }

            // Step 1: Verify SOCKS5 proxy is reachable
            let socks_addr = format!("127.0.0.1:{DEFAULT_SOCKS_PORT}");
            match std::net::TcpStream::connect_timeout(
                &socks_addr.parse().unwrap(),
                Duration::from_secs(3),
            ) {
                Ok(_) => {
                    push_log_entry(
                        &verify_logs,
                        "info",
                        &format!("[verify] SOCKS5 proxy reachable on port {DEFAULT_SOCKS_PORT}"),
                    );
                    info!("[verify] SOCKS5 proxy reachable on port {DEFAULT_SOCKS_PORT}");
                }
                Err(e) => {
                    push_log_entry(
                        &verify_logs,
                        "warning",
                        &format!(
                            "[verify] SOCKS5 proxy NOT reachable on port {DEFAULT_SOCKS_PORT}: {e}"
                        ),
                    );
                    warn!("[verify] SOCKS5 proxy NOT reachable: {e}");
                    return;
                }
            }

            // Step 2: Check system proxy configuration
            push_log_entry(
                &verify_logs,
                "info",
                &format!("[verify] System proxy configured for port {DEFAULT_SOCKS_PORT}"),
            );

            // Step 3: Wait and check for traffic flow
            push_log_entry(&verify_logs, "info", "[verify] Waiting for traffic flow...");
            std::thread::sleep(Duration::from_secs(5));

            // Check if still connected
            {
                let state = verify_state.lock().unwrap();
                if state.status != ConnectionStatus::Connected {
                    return;
                }
            }

            // Check cached stats (populated by the frontend's 1s polling loop)
            let cached = verify_stats.lock().unwrap().clone();
            let total_up = cached.total_upload;
            let total_down = cached.total_download;

            if total_up > 0 || total_down > 0 {
                push_log_entry(
                    &verify_logs,
                    "info",
                    &format!(
                        "[verify] Traffic flowing — upload: {} bytes, download: {} bytes",
                        total_up, total_down
                    ),
                );
                info!("[verify] Traffic flowing — up: {total_up} B, down: {total_down} B");
            } else {
                push_log_entry(
                    &verify_logs,
                    "warning",
                    "[verify] No traffic detected after 5s — VPN may not be routing correctly",
                );
                warn!("[verify] No traffic detected after 5s");

                // Wait longer and check again
                std::thread::sleep(Duration::from_secs(10));
                {
                    let state = verify_state.lock().unwrap();
                    if state.status != ConnectionStatus::Connected {
                        return;
                    }
                }
                let cached = verify_stats.lock().unwrap().clone();
                if cached.total_upload > 0 || cached.total_download > 0 {
                    push_log_entry(
                        &verify_logs,
                        "info",
                        &format!(
                            "[verify] Traffic detected after 15s — upload: {} bytes, download: {} bytes",
                            cached.total_upload, cached.total_download
                        ),
                    );
                } else {
                    push_log_entry(
                        &verify_logs,
                        "error",
                        "[verify] Still no traffic after 15s — connection may be broken. Check server config and network.",
                    );
                    error!("[verify] No traffic after 15s, connection may be broken");
                }
            }
        });

        // Start TUN mode after xray connects (Linux only). In L3 mode
        // xray-core's `l3client` already owns a native TUN, so launching
        // hev-socks5-tunnel on top of it would race for the same
        // adapter — skip it.
        #[cfg(target_os = "linux")]
        if !l3_mode {
            let (hev_bin, tun_config_dir, tun_server_ip, tun_bypass_subnets, tun_gateway_info) =
                tun_data;
            let tun_logs = self.logs.clone();
            let tun_state = self.state.clone();
            std::thread::spawn({
                let tun_logs = tun_logs.clone();
                let started = started_flag_tun.clone();
                move || {
                    // Wait for xray to connect
                    for _ in 0..40 {
                        if started.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    if !started.load(std::sync::atomic::Ordering::Acquire) {
                        return;
                    }

                    // Verify SOCKS5 proxy is ready before starting TUN
                    let socks_addr = format!("127.0.0.1:{DEFAULT_SOCKS_PORT}");
                    let mut socks_ready = false;
                    for attempt in 1..=6 {
                        match std::net::TcpStream::connect_timeout(
                            &socks_addr.parse().unwrap(),
                            Duration::from_millis(500),
                        ) {
                            Ok(_) => {
                                socks_ready = true;
                                break;
                            }
                            Err(_) if attempt < 6 => {
                                std::thread::sleep(Duration::from_millis(500));
                            }
                            Err(e) => {
                                push_log_entry(
                                    &tun_logs,
                                    "error",
                                    &format!(
                                        "[tun] SOCKS5 proxy not ready after 3s: {e}. Skipping TUN."
                                    ),
                                );
                            }
                        }
                    }
                    if !socks_ready {
                        return;
                    }

                    // Check connection status is still Connected before starting TUN
                    {
                        let state = tun_state.lock().unwrap();
                        if state.status != ConnectionStatus::Connected {
                            push_log_entry(
                                &tun_logs,
                                "warning",
                                "[tun] Connection no longer active, skipping TUN start",
                            );
                            return;
                        }
                    }

                    push_log_entry(&tun_logs, "info", "[tun] Starting TUN mode...");

                    match tun::start_tun(
                        &hev_bin,
                        DEFAULT_SOCKS_PORT,
                        &tun_server_ip,
                        &tun_bypass_subnets,
                        &tun_config_dir,
                        tun_gateway_info,
                    ) {
                        Ok(()) => {
                            push_log_entry(
                                &tun_logs,
                                "info",
                                "[tun] TUN mode started successfully",
                            );
                            info!("[tun] TUN mode active — all traffic routed through VPN");
                        }
                        Err(e) => {
                            push_log_entry(
                                &tun_logs,
                                "error",
                                &format!(
                                    "[tun] Failed to start TUN: {e}. Falling back to system proxy."
                                ),
                            );
                            error!("[tun] TUN start failed: {e}");
                        }
                    }
                }
            });
        }

        Ok(())
    }

    /// macOS L3-only spawn path. Reads the user's sudo password from
    /// Keychain (which the UI is responsible for filling before the
    /// first connect), runs xray under `sudo -S` so it can claim a
    /// utun device, and installs the same Connecting → Connected /
    /// Connecting → Error / 15 s timeout / 6 s fallback watchdogs that
    /// the cross-platform sidecar path uses on Windows and Linux. The
    /// Tauri sidecar API is not used here because it spawns xray as
    /// the GUI user, which would fail when xray tries to open utun.
    #[cfg(target_os = "macos")]
    fn start_desktop_macos<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        server: &ServerConfig,
        config_file: &std::path::Path,
    ) -> Result<(), AppError> {
        if !self.l3_mode.load(std::sync::atomic::Ordering::Acquire) {
            return Err(AppError::XrayProcess(
                "macOS only supports L3 (vnet=1) servers. The selected server is missing \
                 vnet=1/vnetIp= parameters."
                    .to_string(),
            ));
        }

        let password = macos_helper::read_password().ok_or_else(|| {
            AppError::XrayProcess(
                "No macOS sudo password saved. Please enter your password when prompted \
                 and try again."
                    .to_string(),
            )
        })?;

        let xray_bin = macos_xray::locate_xray_binary()?;
        info!("macOS xray binary: {}", xray_bin.display());

        let started_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let output_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let sudo_child = macos_xray::spawn_xray_sudo(
            app,
            &xray_bin,
            config_file,
            &password,
            self.state.clone(),
            self.logs.clone(),
            server.name.clone(),
            server.address.clone(),
            started_flag.clone(),
            output_seen.clone(),
            line_signals_started,
            mark_connected,
            push_log_entry,
        )?;

        {
            let mut guard = self.macos_child.lock().unwrap();
            *guard = Some(sudo_child);
        }

        // 15 s connection timeout: kill sudo+xray if we haven't seen
        // the "started" marker by then. Mirrors the cross-platform
        // timeout in start_desktop.
        {
            let timeout_state = self.state.clone();
            let timeout_macos_child = self.macos_child.clone();
            let timeout_logs = self.logs.clone();
            let timeout_app = app.clone();
            let started = started_flag.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(15));
                if started.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                let mut s = timeout_state.lock().unwrap();
                if s.status == ConnectionStatus::Connecting {
                    warn!("Connection timeout after 15 seconds (macOS)");
                    push_log_entry(
                        &timeout_logs,
                        "error",
                        "Connection timeout after 15 seconds",
                    );
                    s.status = ConnectionStatus::Error;
                    s.error_message = Some(
                        "Connection timeout — server unreachable or config invalid".to_string(),
                    );
                    s.connected_since = None;
                    drop(s);
                    let sc = { timeout_macos_child.lock().unwrap().take() };
                    if let Some(sc) = sc {
                        if let Err(e) = macos_xray::stop_sudo_child(&sc) {
                            warn!("Timeout watchdog: failed to kill sudo+xray: {e}");
                        }
                    }
                    let _ = timeout_app.emit("connection-status-changed", "disconnected");
                }
            });
        }

        // 6 s fallback: xray-core sometimes reorders output across
        // pipes and we don't see the "started" magic word in time. If
        // we've at least seen *some* output, promote Connecting →
        // Connected so the UI isn't stuck. The 15 s timeout above
        // still trips if there's NO output at all.
        {
            let fb_state = self.state.clone();
            let fb_app = app.clone();
            let fb_server_name = server.name.clone();
            let fb_server_address = server.address.clone();
            let fb_started = started_flag.clone();
            let fb_output_seen = output_seen.clone();
            let fb_logs = self.logs.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(6));
                if fb_started.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                if !fb_output_seen.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                if mark_connected(&fb_started, &fb_state, &fb_server_name, &fb_server_address) {
                    warn!("xray fallback connection trigger: 'started' log not observed (macOS)");
                    push_log_entry(
                        &fb_logs,
                        "info",
                        "Marking Connected via fallback (no 'started' log observed)",
                    );
                    let _ = fb_app.emit("connection-status-changed", "connected");
                }
            });
        }

        Ok(())
    }

    pub fn stop(&self) -> Result<(), AppError> {
        self.update_status(ConnectionStatus::Disconnecting, None, None);

        self.stop_desktop()?;

        // Update status
        self.update_status(ConnectionStatus::Disconnected, None, None);

        // Reset stats counters
        self.reset_stats();

        Ok(())
    }

    fn stop_desktop(&self) -> Result<(), AppError> {
        let t0 = std::time::Instant::now();
        let l3_mode = self.l3_mode.load(std::sync::atomic::Ordering::Acquire);

        // Stop TUN mode first (Linux only) — must happen before killing xray
        // so hev-socks5-tunnel can cleanly shut down while SOCKS5 is still
        // available. In L3 mode no hev-socks5-tunnel was launched in the
        // first place, so skip the cleanup.
        #[cfg(target_os = "linux")]
        if !l3_mode {
            let config_dir = self
                .config_path
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()));
            if let Some(ref dir) = config_dir {
                if let Err(e) = tun::stop_tun(dir) {
                    warn!("TUN cleanup error: {e}");
                    push_log_entry(
                        &self.logs,
                        "error",
                        &format!("TUN cleanup failed: {e} — routes may be stale"),
                    );
                }
            }
        }
        let t1 = std::time::Instant::now();

        // Disable system proxy. In L3 mode we never enabled it (xray-core's
        // l3client owns the TUN adapter and the OS resolver/proxy is
        // untouched), so the registry write would just churn for nothing
        // — skip it. On the slow Parallels x86_64 emulation this can
        // shave several seconds off the user-visible disconnect time.
        if !l3_mode {
            proxy::disable_system_proxy();
        }
        let t2 = std::time::Instant::now();

        // Hard-kill the child process. tauri-plugin-shell's CommandChild::kill
        // delegates to shared_child, which on Windows uses TerminateProcess
        // and on Unix uses SIGKILL — neither of which gives xray-core a
        // chance to run its [Debug] "Logger closing" cleanup loop. That's
        // intentional: graceful shutdown can keep the process alive for
        // several seconds while the Go logger drains, and on the next
        // launch xray-core itself reaps the orphaned wintun adapter
        // ("Removed orphaned adapter ..."). A hard kill is faster and
        // produces no observable downside.
        let child = {
            let mut guard = self.child.lock().unwrap();
            guard.take()
        };

        if let Some(child) = child {
            let pid = child.pid();
            child
                .kill()
                .map_err(|e| AppError::XrayProcess(format!("Failed to kill xray: {e}")))?;
            info!("Hard-killed xray process (pid={pid})");
        }

        // macOS uses a separate sudo+xray child (see start_desktop_macos).
        // The wait thread spawned in spawn_xray_sudo will see the kill
        // and drive the Connecting → Disconnected transition + emit
        // the connection-status-changed event on its own.
        #[cfg(target_os = "macos")]
        {
            let sc = { self.macos_child.lock().unwrap().take() };
            if let Some(sc) = sc {
                // Propagate so the UI can show a real error instead of
                // silently transitioning to Disconnected while xray is
                // still alive in the background.
                macos_xray::stop_sudo_child(&sc)?;
            }
        }
        let t3 = std::time::Instant::now();

        // Clean up config file
        let config_path = {
            let mut guard = self.config_path.lock().unwrap();
            guard.take()
        };
        if let Some(path) = config_path {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
                info!("Removed config file: {}", path.display());
            }
        }
        let t4 = std::time::Instant::now();

        self.l3_mode
            .store(false, std::sync::atomic::Ordering::Release);

        info!(
            "[stop] timings tun={:?} disable_proxy={:?} kill={:?} fs_cleanup={:?} total={:?}",
            t1 - t0,
            t2 - t1,
            t3 - t2,
            t4 - t3,
            t4 - t0
        );
        Ok(())
    }

    pub fn test_connection(&self) -> Result<bool, AppError> {
        // Only report success if we actually believe we're connected — otherwise
        // a probe against the SOCKS port can succeed against a stale/reused
        // socket from a previous session and lie to the UI.
        {
            let state = self.state.lock().unwrap();
            if state.status != ConnectionStatus::Connected {
                return Ok(false);
            }
        }
        let addr = format!("127.0.0.1:{DEFAULT_SOCKS_PORT}");
        let socket_addr = addr
            .parse()
            .map_err(|e| AppError::XrayProcess(format!("Invalid SOCKS address: {e}")))?;
        match std::net::TcpStream::connect_timeout(&socket_addr, Duration::from_secs(3)) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub fn socks_port(&self) -> u16 {
        DEFAULT_SOCKS_PORT
    }

    /// Query stats from xray's gRPC API via the sidecar binary
    pub async fn query_stats<R: Runtime>(
        &self,
        app: &AppHandle<R>,
    ) -> Result<SpeedStats, AppError> {
        // Only query if connected
        {
            let state = self.state.lock().unwrap();
            if state.status != ConnectionStatus::Connected {
                return Ok(SpeedStats::default());
            }
        }

        // Run xray api statsquery via sidecar
        let output = app
            .shell()
            .sidecar("xray")
            .map_err(|e| AppError::XrayProcess(format!("Failed to create sidecar command: {e}")))?
            .args(["api", "statsquery", "-s", config::STATS_API_ADDR])
            .output()
            .await
            .map_err(|e| AppError::XrayProcess(format!("Failed to query stats: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let combined = if stdout.contains(">>>") {
            &stdout
        } else {
            &stderr
        };

        let (uplink, downlink) = Self::parse_stats_output(combined);

        // Compute speed from delta
        let mut prev_up = self.prev_uplink.lock().unwrap();
        let mut prev_down = self.prev_downlink.lock().unwrap();

        let upload_speed = uplink.saturating_sub(*prev_up);
        let download_speed = downlink.saturating_sub(*prev_down);

        *prev_up = uplink;
        *prev_down = downlink;

        let new_stats = SpeedStats {
            upload_speed,
            download_speed,
            total_upload: uplink,
            total_download: downlink,
        };

        // Update stored stats
        {
            let mut stats = self.stats.lock().unwrap();
            *stats = new_stats.clone();
        }

        Ok(new_stats)
    }

    /// Get cached stats without querying (for non-async contexts)
    pub fn cached_stats(&self) -> SpeedStats {
        self.stats.lock().unwrap().clone()
    }

    /// Reset stats counters (called on connect/disconnect)
    fn reset_stats(&self) {
        let mut stats = self.stats.lock().unwrap();
        *stats = SpeedStats::default();
        let mut prev_up = self.prev_uplink.lock().unwrap();
        *prev_up = 0;
        let mut prev_down = self.prev_downlink.lock().unwrap();
        *prev_down = 0;
    }

    /// Parse xray statsquery JSON output
    fn parse_stats_output(output: &str) -> (u64, u64) {
        let mut uplink: u64 = 0;
        let mut downlink: u64 = 0;

        // Output is JSON: {"stat": [{"name": "...", "value": N}, ...]}
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(stats) = json.get("stat").and_then(|s| s.as_array()) {
                for entry in stats {
                    let name = entry.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let value = entry
                        .get("value")
                        .and_then(|v| {
                            v.as_u64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0);

                    if name == "outbound>>>proxy>>>traffic>>>uplink" {
                        uplink = value;
                    } else if name == "outbound>>>proxy>>>traffic>>>downlink" {
                        downlink = value;
                    }
                }
            }
        }

        (uplink, downlink)
    }

    fn update_status(
        &self,
        status: ConnectionStatus,
        server: Option<&ServerConfig>,
        error: Option<String>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.status = status;
        if let Some(srv) = server {
            state.server_name = Some(srv.name.clone());
            state.server_address = Some(srv.address.clone());
        }
        if status == ConnectionStatus::Disconnected {
            state.server_name = None;
            state.server_address = None;
            state.connected_since = None;
            state.error_message = None;
        }
        if let Some(err) = error {
            state.error_message = Some(err);
        }
    }
}

/// Look for any of the signals that xray-core has finished startup and
/// is accepting connections. The canonical message is
/// `[Warning] core: Xray <version> started`, but pipe buffering on
/// Windows can re-order or merge lines so we also accept the
/// virtualNetwork / l3client init markers as proof that startup got far
/// enough to be serving traffic.
fn line_signals_started(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    line.contains("started")
        || line.contains("core: Xray ")
        || line.contains("virtualNetwork: l3client created")
}

/// Transition state Connecting → Connected atomically under the state lock.
///
/// Returns true if this call was the one that performed the transition
/// (i.e. the caller "won" the race and should enable the system proxy and
/// emit the `connected` event). Returns false if the connection already
/// timed out, was cancelled, or another thread already marked it Connected.
fn mark_connected(
    started_flag: &std::sync::atomic::AtomicBool,
    state: &Arc<Mutex<ConnectionInfo>>,
    server_name: &str,
    server_address: &str,
) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut s = state.lock().unwrap();
    if s.status != ConnectionStatus::Connecting {
        return false;
    }
    if started_flag.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return false;
    }
    s.status = ConnectionStatus::Connected;
    s.connected_since = Some(now);
    s.server_name = Some(server_name.to_string());
    s.server_address = Some(server_address.to_string());
    s.error_message = None;
    true
}

fn push_log_entry(logs: &Arc<Mutex<VecDeque<LogEntry>>>, level: &str, message: &str) {
    let entry = LogEntry {
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        level: level.to_string(),
        message: message.to_string(),
    };
    let mut buffer = logs.lock().unwrap();
    if buffer.len() >= MAX_LOG_ENTRIES {
        buffer.pop_front();
    }
    buffer.push_back(entry);
}
