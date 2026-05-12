//! Privileged spawn path for xray-core on Unix desktops. Runs xray
//! under `sudo -S` so it can claim the platform's TUN device for L3
//! mode (utun on macOS, /dev/net/tun on Linux), and exposes the same
//! "monitor stdout for 'started' / push log lines / emit terminate
//! events" surface that the cross-platform Tauri-sidecar path in
//! `xray.rs` uses on Windows.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Runtime};

use crate::models::{AppError, ConnectionInfo, ConnectionStatus, LogEntry};

/// Locate the bundled xray sidecar binary. In a production install
/// Tauri places sidecars next to the main executable (inside
/// `Contents/MacOS/` on macOS, in the install directory or AppImage
/// usr/bin on Linux), named with the target-triple suffix. In `pnpm
/// tauri dev` mode the bundled copy lives under
/// `src-tauri/binaries/`. We look in the production location first
/// and fall back to dev only if needed.
pub fn locate_xray_binary() -> Result<std::path::PathBuf, AppError> {
    let triple = xray_target_triple();
    let exe = std::env::current_exe()
        .map_err(|e| AppError::Config(format!("current_exe failed: {e}")))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| AppError::Config("current_exe has no parent".to_string()))?;

    let bundled = exe_dir.join(format!("xray-{triple}"));
    if bundled.exists() {
        return Ok(bundled);
    }

    // Some Tauri 2 layouts also expose sidecars without the target
    // suffix once installed. Try the unsuffixed name too.
    let bundled_plain = exe_dir.join("xray");
    if bundled_plain.exists() {
        return Ok(bundled_plain);
    }

    // Dev fallback: `src-tauri/binaries/xray-<triple>`.
    let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join(format!("xray-{triple}"));
    if dev.exists() {
        return Ok(dev);
    }

    Err(AppError::XrayProcess(format!(
        "xray sidecar not found (looked at {} and {})",
        bundled.display(),
        dev.display()
    )))
}

/// Tauri-style target-triple suffix used on the sidecar file name. We
/// pick this at compile time so a universal build still does the
/// right thing per slice.
const fn xray_target_triple() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
}

/// Handle returned from `spawn_xray_sudo` and stored on `XrayManager`
/// while a connection is active. We keep around:
///   * `pgid`  — the process group of the sudo wrapper, used for the
///     cheap cleanup of the sudo process itself.
///   * `xray_path` — the absolute path to the bundled xray binary,
///     used as the `pkill -f` pattern on disconnect (see
///     `stop_sudo_child` for why we can't just signal the process
///     group).
///
/// The actual `std::process::Child` is owned by the background wait
/// thread so it can detect spontaneous xray crashes without needing
/// to grab a mutex.
pub struct SudoChild {
    pub pgid: i32,
    pub xray_path: PathBuf,
}

/// Spawn xray under `sudo -S` with the supplied password and wire up
/// the same stdout/stderr/terminate monitor pattern the cross-platform
/// sidecar path uses on Windows and Linux. Drives the shared `state` /
/// `logs` mutexes from `XrayManager` so the rest of the app doesn't
/// have to care which platform produced the child.
///
/// `signals_started` / `mark_connected` / `push_log_entry` are passed
/// in by the caller as function pointers to avoid duplicating the
/// startup-detection / state-transition / log-rotation logic that
/// already lives in `xray.rs`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_xray_sudo<R: Runtime>(
    app: &AppHandle<R>,
    xray_path: &Path,
    config_path: &Path,
    password: &str,
    state: Arc<Mutex<ConnectionInfo>>,
    logs: Arc<Mutex<VecDeque<LogEntry>>>,
    server_name: String,
    server_address: String,
    started_flag: Arc<AtomicBool>,
    output_seen: Arc<AtomicBool>,
    signals_started: fn(&str) -> bool,
    mark_connected: fn(&AtomicBool, &Arc<Mutex<ConnectionInfo>>, &str, &str) -> bool,
    push_log_entry: fn(&Arc<Mutex<VecDeque<LogEntry>>>, &str, &str),
) -> Result<SudoChild, AppError> {
    let mut cmd = Command::new("sudo");
    cmd.arg("-S")
        .arg("-p")
        .arg("")
        .arg("-k")
        .arg(xray_path)
        .arg("run")
        .arg("-c")
        .arg(config_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Put the child into its own process group so the disconnect path
    // can SIGKILL the whole group (sudo + xray) in one call instead of
    // racing the kernel for the orphaned xray child.
    //
    // SAFETY: `setpgid(0, 0)` only touches the just-forked child's own
    // process group membership; it cannot reach back into the parent
    // address space and has no other side effects.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::XrayProcess(format!("Failed to spawn sudo+xray: {e}")))?;

    // Feed the password into sudo's stdin and immediately close it so
    // sudo can't block waiting for "more". A trailing newline so sudo's
    // line-buffered read() returns.
    if let Some(stdin) = child.stdin.take() {
        let mut stdin = stdin;
        if let Err(e) = writeln!(stdin, "{password}") {
            warn!("Failed to write password to sudo stdin: {e}");
        }
        drop(stdin);
    }

    let pid = child.id();
    let pgid = pid as i32;
    info!("Spawned sudo+xray (pid={pid}, pgid={pgid})");

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::XrayProcess("sudo+xray missing stdout pipe".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::XrayProcess("sudo+xray missing stderr pipe".to_string()))?;

    // Monitor stdout in a dedicated OS thread. Pattern mirrors the
    // tauri-plugin-shell CommandEvent::Stdout branch in xray.rs so the
    // same "look for started, then mark Connected and emit
    // connection-status-changed" plumbing keeps working on macOS.
    {
        let logs = logs.clone();
        let state = state.clone();
        let started_flag = started_flag.clone();
        let output_seen = output_seen.clone();
        let app = app.clone();
        let server_name = server_name.clone();
        let server_address = server_address.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        warn!("xray stdout read error: {e}");
                        break;
                    }
                };
                let trimmed = line.trim();
                info!("xray stdout: {trimmed}");
                push_log_entry(&logs, "info", trimmed);
                output_seen.store(true, Ordering::Release);

                if !started_flag.load(Ordering::Acquire)
                    && signals_started(trimmed)
                    && mark_connected(&started_flag, &state, &server_name, &server_address)
                {
                    info!("xray connected successfully (detected from stdout)");
                    let _ = app.emit("connection-status-changed", "connected");
                }
            }
        });
    }

    // stderr carries both xray-core's actual diagnostics AND sudo's
    // "Sorry, try again" / "no tty present" complaints. We surface
    // sudo-auth failures as our own structured error event so the UI
    // can wipe the stale Keychain entry and re-prompt without waiting
    // for the 15s timeout.
    {
        let logs = logs.clone();
        let state = state.clone();
        let started_flag = started_flag.clone();
        let output_seen = output_seen.clone();
        let app = app.clone();
        let server_name = server_name.clone();
        let server_address = server_address.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(e) => {
                        warn!("xray stderr read error: {e}");
                        break;
                    }
                };
                let trimmed = line.trim();
                info!("xray stderr: {trimmed}");
                output_seen.store(true, Ordering::Release);

                // Sudo auth failure detection: kill the rest of the
                // group, wipe Keychain, signal the UI.
                let low = trimmed.to_lowercase();
                if low.contains("incorrect password attempt")
                    || low.contains("sorry, try again")
                    || low.contains("3 incorrect password attempts")
                {
                    warn!("sudo rejected the stored password");
                    if let Err(e) = crate::secret_store::delete_password() {
                        warn!("Failed to clear stale credential store entry: {e}");
                    }
                    push_log_entry(
                        &logs,
                        "error",
                        "Saved sudo password is no longer valid — \
                         please re-enter it next time you connect.",
                    );
                    let _ = app.emit("sudo-auth-failed", ());
                    // Knock down the process group so we don't leave a
                    // half-prompting sudo hanging around.
                    //
                    // SAFETY: killpg with a positive pgid only signals
                    // the named group; nothing else in this process is
                    // affected.
                    unsafe {
                        libc::killpg(pgid, libc::SIGKILL);
                    }
                    continue;
                }

                let level = if trimmed.contains("[Warning]") {
                    "warning"
                } else if trimmed.contains("[Error]") {
                    "error"
                } else {
                    "info"
                };
                push_log_entry(&logs, level, trimmed);

                if !started_flag.load(Ordering::Acquire)
                    && signals_started(trimmed)
                    && mark_connected(&started_flag, &state, &server_name, &server_address)
                {
                    info!("xray connected successfully (detected from stderr)");
                    let _ = app.emit("connection-status-changed", "connected");
                }
            }
        });
    }

    // Wait thread: owns the Child, blocks on .wait(), then drives the
    // same state.status transitions and `connection-status-changed`
    // disconnected event that CommandEvent::Terminated handles on
    // Windows/Linux. When the user clicks Disconnect, we send SIGKILL
    // to the process group from elsewhere and this thread observes the
    // exit naturally.
    {
        let logs = logs.clone();
        let state = state.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let exit = child.wait();
            let code = match exit {
                Ok(s) => s.code(),
                Err(e) => {
                    error!("waiting on sudo+xray failed: {e}");
                    None
                }
            };
            warn!("sudo+xray exited with code: {code:?}");
            push_log_entry(
                &logs,
                "warning",
                &format!("xray terminated (code: {code:?})"),
            );

            let mut s = state.lock().unwrap();
            if s.status == ConnectionStatus::Disconnecting {
                s.status = ConnectionStatus::Disconnected;
            } else if s.status != ConnectionStatus::Disconnected
                && s.status != ConnectionStatus::Error
            {
                s.status = ConnectionStatus::Error;
                s.error_message = Some(format!("xray exited unexpectedly (code: {code:?})"));
            }
            s.connected_since = None;
            drop(s);

            let _ = app.emit("connection-status-changed", "disconnected");
        });
    }

    Ok(SudoChild {
        pgid,
        xray_path: xray_path.to_path_buf(),
    })
}

/// Stop a sudo-rooted xray child. This is harder than it looks on
/// macOS, for two reasons:
///
///   1. xray is `setuid(0)` because sudo ran it. A signal from our
///      non-root GUI process returns EPERM. We must re-elevate via
///      `sudo -S kill ...` using the cached Keychain password.
///
///   2. Modern macOS sudo runs the command under a pty monitor that
///      calls `setsid()` on the actual command process — so xray ends
///      up in a NEW process group and a NEW session, distinct from
///      the pgid we captured at spawn time. That means
///      `killpg(captured_pgid)` only kills the sudo wrapper; xray
///      survives as an orphan re-parented to launchd and keeps the
///      VPN tunnel up. This was the root cause of the
///      "disconnect-but-still-connected" bug observed on macOS L3.
///
/// The fix is to target xray by its absolute binary path with
/// `pkill -9 -f <xray_path>`. The path lives inside our `.app`
/// bundle and is unique enough that we won't accidentally signal a
/// user-managed xray instance running from a different prefix.
/// We still SIGKILL the captured process group as cheap cleanup so
/// the sudo wrapper doesn't linger.
///
/// The wait thread launched in `spawn_xray_sudo` observes the sudo
/// wrapper's exit and drives the state / disconnect event. The kill
/// is idempotent: if the target is already gone we treat that as
/// success.
///
/// KNOWN LIMITATION: if the user changes their macOS account password
/// while connected, the cached Keychain entry becomes stale.
/// Disconnect then can't elevate (sudo rejects the stale password),
/// so the xray process is leaked. The returned `Err` includes manual
/// recovery instructions (`sudo killall xray`). Mitigations:
///   1. The stderr handler in `spawn_xray_sudo` proactively wipes
///      the stale Keychain entry the moment sudo's "Sorry, try
///      again" hits, which catches the much more common spawn-time
///      auth failure.
///   2. A future iteration should use a privileged SMAppService
///      daemon so kill no longer needs the user's password at all.
pub fn stop_sudo_child(sc: &SudoChild) -> Result<(), AppError> {
    // Cheap cleanup of the sudo wrapper. We don't really care if it
    // succeeds because the real xray we kill below — this just keeps
    // the captured pgid from lingering on EPERM/ESRCH paths. SAFETY:
    // killpg with a positive pgid only signals processes in that
    // group; it cannot affect anything else in this process.
    unsafe {
        let rc = libc::killpg(sc.pgid, libc::SIGKILL);
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            // EPERM (root-owned) and ESRCH (already gone) are both
            // expected and don't tell us anything about whether xray
            // itself is alive.
            match err.raw_os_error() {
                Some(libc::EPERM) | Some(libc::ESRCH) => {}
                _ => warn!("killpg({}) returned unexpected error: {err}", sc.pgid),
            }
        }
    }

    let password = crate::secret_store::read_password().ok_or_else(|| {
        AppError::XrayProcess(
            "Cannot kill root-owned xray: no sudo password in the OS \
             credential store. The xray process may need to be killed \
             manually with `sudo killall xray`."
                .to_string(),
        )
    })?;

    let xray_path_str = sc.xray_path.to_string_lossy().to_string();
    info!(
        "Killing xray via sudo pkill -f {} (captured sudo pgid {})",
        xray_path_str, sc.pgid
    );

    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "pkill", "-9", "-f", &xray_path_str])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::XrayProcess(format!("Failed to spawn sudo pkill: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = writeln!(stdin, "{password}") {
            warn!("Failed to write password to sudo pkill stdin: {e}");
        }
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| AppError::XrayProcess(format!("waiting for sudo pkill failed: {e}")))?;

    // pkill exit codes: 0 — one or more matched and signalled;
    //                   1 — nothing matched (i.e. xray already gone);
    //                   2 — syntax error;
    //                   3 — fatal error.
    // The "already gone" case is a perfectly valid disconnect
    // outcome (xray died on its own), so accept 0 and 1.
    //
    // BUT there's an extra wrinkle: `pkill -f <xray_path>` also
    // matches its own `sudo` wrapper, because sudo's argv includes
    // the xray path as one of its arguments (`-f` matches the full
    // cmdline as a regex). When pkill signals its parent sudo, sudo
    // is killed by SIGKILL before exiting normally, and our
    // `wait_with_output` reports `code = None` (signal-killed)
    // instead of a numeric exit status. By the time pkill walks
    // through PIDs in order, the lower-PID xray has already been
    // signalled, so xray IS dead — we just lost visibility into the
    // numeric pkill exit code. Treat None the same as "0 or 1": a
    // valid disconnect outcome.
    let code = output.status.code();
    let success = matches!(code, Some(0) | Some(1)) || code.is_none();

    if !success {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        warn!(
            "sudo pkill exited with {}: {}",
            output.status,
            stderr.trim()
        );

        // If sudo rejected the password, the saved credential is
        // stale (user changed their account password while
        // connected). Wipe it so the next connect surfaces the
        // modal. We can't kill xray ourselves — point the user at
        // manual recovery.
        let low = stderr.to_lowercase();
        if low.contains("incorrect password attempt")
            || low.contains("sorry, try again")
            || low.contains("3 incorrect password attempts")
        {
            if let Err(e) = crate::secret_store::delete_password() {
                warn!("Failed to clear stale credential store entry: {e}");
            }
            return Err(AppError::XrayProcess(
                "Saved sudo password is no longer valid. The xray \
                 process is still running and must be killed manually \
                 with `sudo killall xray`. You'll be re-prompted for \
                 your password on the next connect."
                    .to_string(),
            ));
        }

        return Err(AppError::XrayProcess(format!(
            "sudo pkill failed ({}): {}",
            output.status,
            stderr.trim()
        )));
    }

    match code {
        Some(1) => info!("xray was already gone (pkill matched nothing)"),
        None => info!(
            "SIGKILL'd xray via sudo pkill -f {} (sudo wrapper also signal-killed by its own pkill)",
            xray_path_str
        ),
        _ => info!("SIGKILL'd xray via sudo pkill -f {}", xray_path_str),
    }
    Ok(())
}
