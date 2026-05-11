//! macOS-specific helpers: Keychain-backed sudo password storage plus a
//! validator that confirms a candidate password against the running
//! system before we hand it to `sudo -S` for real.
//!
//! Why a separate module: the L3 (virtualnet) path needs xray-core to
//! run as root so it can claim a `utun` device. Windows solves this with
//! a UAC manifest baked into the .exe; macOS has no equivalent for
//! GUI-launched .app bundles, so we ask the user for their account
//! password once, stash it in the user's login Keychain, and re-use it
//! on every subsequent connect by piping it into `sudo -S`.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use log::warn;

use crate::models::AppError;

/// Keychain entry name. The account is fixed (`v2rayv-sudo`) and the
/// service label is the app name. `security find-generic-password -s
/// v2rayV -a v2rayv-sudo -w` reads it back.
const KEYCHAIN_SERVICE: &str = "v2rayV";
const KEYCHAIN_ACCOUNT: &str = "v2rayv-sudo";

/// Read the saved sudo password from the user's login Keychain. Returns
/// `None` if no entry exists yet or `security` failed for any reason.
pub fn read_password() -> Option<String> {
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }
    let pw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pw.is_empty() {
        None
    } else {
        Some(pw)
    }
}

/// True if a Keychain entry currently exists. Cheaper than reading the
/// password because we don't materialize the secret in our memory.
pub fn has_password() -> bool {
    let status = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success())
}

/// Write the password to Keychain, replacing any existing entry
/// (`-U`). The shape mirrors the Apple-documented usage pattern for
/// background daemons that need to recover an admin credential without
/// re-prompting on every launch.
pub fn write_password(password: &str) -> Result<(), AppError> {
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
            password,
            "-U",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| AppError::XrayProcess(format!("security add-generic-password failed: {e}")))?;
    if !status.success() {
        return Err(AppError::XrayProcess(format!(
            "security add-generic-password exited with {status}"
        )));
    }
    Ok(())
}

/// Drop the Keychain entry. Called when we discover the saved password
/// no longer authenticates (e.g. the user changed their macOS password)
/// so the UI can re-prompt without leaving stale credentials behind.
pub fn delete_password() -> Result<(), AppError> {
    let status = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| {
            AppError::XrayProcess(format!("security delete-generic-password failed: {e}"))
        })?;
    // exit 44 == "no such item" — treat as success so callers can use
    // this as an idempotent "make sure it's gone" without checking
    // existence first.
    if !status.success() && status.code() != Some(44) {
        return Err(AppError::XrayProcess(format!(
            "security delete-generic-password exited with {status}"
        )));
    }
    Ok(())
}

/// Confirm `password` is the user's actual sudo password by running
/// `sudo -S -p "" -k -v` and piping the candidate into stdin. The `-k`
/// flag invalidates any cached ticket first so we genuinely re-check
/// against the system, not against a stale 5-minute timestamp.
///
/// Returns `Ok(true)` if sudo accepted the password, `Ok(false)` if it
/// rejected the password, and `Err(_)` if `sudo` couldn't be spawned at
/// all (highly unusual on macOS).
pub fn validate_password(password: &str) -> Result<bool, AppError> {
    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "-k", "-v"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AppError::XrayProcess(format!("Failed to spawn sudo: {e}")))?;

    if let Some(stdin) = child.stdin.as_mut() {
        // Trailing newline so sudo's read() returns instead of
        // blocking forever on an unterminated line.
        if let Err(e) = writeln!(stdin, "{password}") {
            warn!("Failed to write password to sudo stdin: {e}");
        }
    }

    // Give sudo a moment to read the password and decide. If it lingers
    // for any reason (it shouldn't with `-S` non-interactive), kill it
    // and report a hard failure so we don't deadlock the UI.
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(5) {
                    let _ = child.kill();
                    return Err(AppError::XrayProcess(
                        "sudo -v did not return within 5 seconds".to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(AppError::XrayProcess(format!("sudo wait failed: {e}"))),
        }
    }
}
