//! OS-managed credential store for the sudo password we need so xray
//! can claim the platform's TUN device (utun on macOS, /dev/net/tun on
//! Linux). On macOS we shell out to the `security` CLI to talk to the
//! login Keychain; on Linux we use the `keyring` crate to talk to the
//! Secret Service D-Bus protocol (gnome-keyring, kwallet, keepassxc,
//! etc.). Public API is identical on both platforms.
//!
//! Validation (`validate_password`) is shared: it shells out to `sudo
//! -S -p "" -k -v` and pipes the candidate password into stdin.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use log::warn;

use crate::models::AppError;

/// Service / account labels used in both backends. macOS:
/// `security find-generic-password -s v2rayV -a v2rayv-sudo -w` reads
/// it back. Linux: same pair becomes the Secret Service attributes.
const SERVICE_NAME: &str = "v2rayV";
const ACCOUNT_NAME: &str = "v2rayv-sudo";

// ----- macOS backend: `security` CLI ----------------------------------------

#[cfg(target_os = "macos")]
mod backend {
    use super::{ACCOUNT_NAME, SERVICE_NAME};
    use crate::models::AppError;
    use std::process::{Command, Stdio};

    pub fn read() -> Option<String> {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                SERVICE_NAME,
                "-a",
                ACCOUNT_NAME,
                "-w",
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let pw = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        if pw.is_empty() {
            None
        } else {
            Some(pw)
        }
    }

    pub fn has() -> bool {
        let status = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                SERVICE_NAME,
                "-a",
                ACCOUNT_NAME,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        matches!(status, Ok(s) if s.success())
    }

    pub fn write(password: &str) -> Result<(), AppError> {
        let status = Command::new("security")
            .args([
                "add-generic-password",
                "-s",
                SERVICE_NAME,
                "-a",
                ACCOUNT_NAME,
                "-w",
                password,
                "-U",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| {
                AppError::XrayProcess(format!("security add-generic-password failed: {e}"))
            })?;
        if !status.success() {
            return Err(AppError::XrayProcess(format!(
                "security add-generic-password exited with {status}"
            )));
        }
        Ok(())
    }

    pub fn delete() -> Result<(), AppError> {
        let status = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                SERVICE_NAME,
                "-a",
                ACCOUNT_NAME,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| {
                AppError::XrayProcess(format!("security delete-generic-password failed: {e}"))
            })?;
        // exit 44 == "no such item" — idempotent success.
        if !status.success() && status.code() != Some(44) {
            return Err(AppError::XrayProcess(format!(
                "security delete-generic-password exited with {status}"
            )));
        }
        Ok(())
    }
}

// ----- Linux backend: Secret Service via the `keyring` crate ----------------

#[cfg(target_os = "linux")]
mod backend {
    use super::{ACCOUNT_NAME, SERVICE_NAME};
    use crate::models::AppError;

    fn entry() -> Result<keyring::Entry, AppError> {
        keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME).map_err(|e| {
            AppError::XrayProcess(format!(
                "Failed to build Secret Service entry ({SERVICE_NAME}/{ACCOUNT_NAME}): {e}. \
                 Is a Secret Service provider (gnome-keyring, kwallet, keepassxc, ...) running?"
            ))
        })
    }

    pub fn read() -> Option<String> {
        let entry = entry().ok()?;
        match entry.get_password() {
            Ok(pw) if !pw.is_empty() => Some(pw),
            _ => None,
        }
    }

    pub fn has() -> bool {
        entry()
            .ok()
            .and_then(|e| e.get_password().ok())
            .map(|pw| !pw.is_empty())
            .unwrap_or(false)
    }

    pub fn write(password: &str) -> Result<(), AppError> {
        let entry = entry()?;
        entry.set_password(password).map_err(|e| {
            AppError::XrayProcess(format!("Failed to store password in Secret Service: {e}"))
        })
    }

    pub fn delete() -> Result<(), AppError> {
        let entry = match entry() {
            Ok(e) => e,
            // Treat "can't reach Secret Service" as already-deleted so
            // callers can use this as an idempotent cleanup.
            Err(_) => return Ok(()),
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::XrayProcess(format!(
                "Failed to delete password from Secret Service: {e}"
            ))),
        }
    }
}

// ----- Public API (shared) --------------------------------------------------

/// Read the saved sudo password from the OS credential store. Returns
/// `None` if no entry exists yet or the backend failed.
pub fn read_password() -> Option<String> {
    backend::read()
}

/// True if a saved entry currently exists. Cheaper than reading the
/// password because we don't materialize the secret in our memory.
pub fn has_password() -> bool {
    backend::has()
}

/// Persist `password` in the OS credential store, replacing any
/// existing entry.
pub fn write_password(password: &str) -> Result<(), AppError> {
    backend::write(password)
}

/// Drop the credential. Called when we discover the saved password no
/// longer authenticates (e.g. the user changed their account password)
/// so the UI can re-prompt without leaving stale credentials behind.
pub fn delete_password() -> Result<(), AppError> {
    backend::delete()
}

/// Confirm `password` is the user's actual sudo password by running
/// `sudo -S -p "" -k -v` and piping the candidate into stdin. The `-k`
/// flag invalidates any cached ticket first so we genuinely re-check
/// against the system, not against a stale 5-minute timestamp.
///
/// Returns `Ok(true)` if sudo accepted the password, `Ok(false)` if it
/// rejected the password, and `Err(_)` if `sudo` couldn't be spawned
/// at all.
pub fn validate_password(password: &str) -> Result<bool, AppError> {
    let mut child = Command::new("sudo")
        .args(["-S", "-p", "", "-k", "-v"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AppError::XrayProcess(format!("Failed to spawn sudo: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        // Trailing newline so sudo's read() returns instead of
        // blocking forever on an unterminated line.
        if let Err(e) = writeln!(stdin, "{password}") {
            warn!("Failed to write password to sudo stdin: {e}");
        }
        // Close stdin so sudo gets EOF immediately after reading the
        // password. Without this, a wrong password causes sudo to
        // block waiting for a second attempt until the 5 s timeout.
        drop(stdin);
    }

    // Give sudo a moment to read the password and decide. If it
    // lingers for any reason (it shouldn't with `-S` non-interactive),
    // kill it and report a hard failure so we don't deadlock the UI.
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
