//! macOS-specific spawn path for xray-core. Runs xray under `sudo -S`
//! so it can claim a `utun` device for L3 mode, and exposes the same
//! "monitor stdout for 'started' / push log lines / emit terminate
//! events" surface that the cross-platform sidecar path in `xray.rs`
//! uses on Windows and Linux.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Runtime};

use crate::models::{AppError, ConnectionInfo, ConnectionStatus, LogEntry};

/// Locate the bundled xray sidecar binary. In a production .app bundle
/// Tauri places sidecars next to the main executable inside
/// `Contents/MacOS/`, named with the target triple suffix. In `pnpm
/// tauri dev` mode the bundled copy lives under
/// `src-tauri/binaries/`. We look in the production location first and
/// fall back to dev only if needed.
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
/// pick this at compile time so a universal build still does the right
/// thing per slice.
const fn xray_target_triple() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64-apple-darwin"
    }
}

/// Handle returned from `spawn_xray_sudo` and stored on `XrayManager`
/// while a connection is active. The only thing we keep around is the
/// process group id; the actual `std::process::Child` is owned by the
/// background wait thread so it can detect spontaneous xray crashes
/// without needing to grab a mutex.
pub struct SudoChild {
    pub pgid: i32,
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
                    if let Err(e) = crate::macos_helper::delete_password() {
                        warn!("Failed to clear stale Keychain entry: {e}");
                    }
                    push_log_entry(
                        &logs,
                        "error",
                        "Stored macOS password is no longer valid — \
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

    Ok(SudoChild { pgid })
}

/// Stop a sudo-rooted xray child by SIGKILL-ing its whole process
/// group. The wait thread launched in `spawn_xray_sudo` will observe
/// the exit on its own and update state / emit the disconnect event.
/// Idempotent: if the group is already gone (ESRCH) we treat that as
/// success.
///
/// Tricky bit: because `sudo` ran `setuid(0)` before exec'ing xray, the
/// whole process group is owned by root. A non-root sender cannot
/// signal a root target, so the cheap `killpg` from our GUI-user process
/// returns EPERM. When that happens we re-elevate via `sudo -S kill -9
/// -<pgid>` using the cached Keychain password — the negative pid tells
/// `kill(1)` to target the entire group at once.
///
/// KNOWN LIMITATION: if the user changes their macOS account password
/// while connected, the cached Keychain entry becomes stale. Disconnect
/// then can't elevate (sudo rejects the stale password), so the xray
/// process is leaked. The returned `Err` includes manual recovery
/// instructions (`sudo killall xray`). Mitigations:
///   1. The stderr handler in `spawn_xray_sudo` proactively wipes the
///      stale Keychain entry the moment sudo's "Sorry, try again" hits,
///      which catches the much more common spawn-time auth failure.
///   2. A future iteration should use a privileged SMAppService daemon
///      so kill no longer needs the user's password at all.
pub fn stop_sudo_child(sc: &SudoChild) -> Result<(), AppError> {
    // SAFETY: killpg with a positive pgid only signals processes in
    // that group; it cannot affect anything else in this process.
    let raw_err = unsafe {
        if libc::killpg(sc.pgid, libc::SIGKILL) == 0 {
            info!("SIGKILL'd sudo+xray process group {}", sc.pgid);
            return Ok(());
        }
        std::io::Error::last_os_error()
    };

    match raw_err.raw_os_error() {
        Some(libc::ESRCH) => {
            info!("sudo+xray process group {} already gone", sc.pgid);
            return Ok(());
        }
        Some(libc::EPERM) => {
            // Expected when xray is running as root; fall through to
            // the sudo-elevated kill below.
        }
        _ => {
            warn!("killpg({}) failed unexpectedly: {raw_err}", sc.pgid);
            return Err(AppError::XrayProcess(format!(
                "killpg({}) failed: {raw_err}",
                sc.pgid
            )));
        }
    }

    let password = crate::macos_helper::read_password().ok_or_else(|| {
        AppError::XrayProcess(
            "Cannot kill root-owned xray: no sudo password in Keychain. \
             The xray process may need to be killed manually with \
             `sudo killall xray`."
                .to_string(),
        )
    })?;

    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "kill", "-9", &format!("-{}", sc.pgid)])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| AppError::XrayProcess(format!("Failed to spawn sudo kill: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = writeln!(stdin, "{password}") {
            warn!("Failed to write password to sudo kill stdin: {e}");
        }
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| AppError::XrayProcess(format!("waiting for sudo kill failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        warn!("sudo kill exited with {}: {}", output.status, stderr.trim());

        // If sudo rejected the password, the Keychain entry is stale
        // (user changed their macOS password while connected). Wipe it
        // so the next connect surfaces the modal. We can't kill xray
        // ourselves — point the user at manual recovery in the error.
        let low = stderr.to_lowercase();
        if low.contains("incorrect password attempt")
            || low.contains("sorry, try again")
            || low.contains("3 incorrect password attempts")
        {
            if let Err(e) = crate::macos_helper::delete_password() {
                warn!("Failed to clear stale Keychain entry: {e}");
            }
            return Err(AppError::XrayProcess(
                "Saved macOS password is no longer valid. The xray \
                 process is still running and must be killed manually \
                 with `sudo killall xray`. You'll be re-prompted for \
                 your password on the next connect."
                    .to_string(),
            ));
        }

        return Err(AppError::XrayProcess(format!(
            "sudo kill failed ({}): {}",
            output.status,
            stderr.trim()
        )));
    }

    info!(
        "SIGKILL'd sudo+xray process group {} via sudo elevation",
        sc.pgid
    );
    Ok(())
}
