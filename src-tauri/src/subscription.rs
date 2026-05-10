use std::time::Duration;

use base64::Engine;

use crate::models::{AppError, ServerConfig};
use crate::uri::parse_vless_uri;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Fetch a subscription URL and parse out every `vless://` server it contains.
///
/// The body is treated either as base64-encoded plaintext (the de-facto v2ray
/// subscription format) or as raw plaintext if base64 decoding fails. Each line
/// is then trimmed, filtered for the `vless://` prefix, and fed through the
/// existing URI parser. Lines that don't parse are skipped silently — partial
/// failures shouldn't block the rest of the list.
pub async fn fetch_subscription(url: &str) -> Result<Vec<ServerConfig>, AppError> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::Config(
            "Subscription URL must use http:// or https://".to_string(),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("v2rayV/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| AppError::Config(format!("HTTP client init failed: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Config(format!("Subscription fetch failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Config(format!(
            "Subscription returned HTTP {}",
            resp.status().as_u16()
        )));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Config(format!("Failed to read subscription body: {e}")))?;

    if bytes.len() > MAX_BODY_BYTES {
        return Err(AppError::Config(format!(
            "Subscription body too large: {} bytes (limit {})",
            bytes.len(),
            MAX_BODY_BYTES
        )));
    }

    Ok(parse_subscription_body(&bytes))
}

/// Decode body as base64 if possible, else treat as plaintext, then collect
/// every parseable `vless://` URI. Public for unit tests.
pub fn parse_subscription_body(body: &[u8]) -> Vec<ServerConfig> {
    let text = decode_body(body);
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("vless://"))
        .filter_map(|line| parse_vless_uri(line).ok())
        .collect()
}

fn decode_body(body: &[u8]) -> String {
    // Most v2ray subscriptions are base64-encoded — try that first, falling
    // back to raw text if it isn't valid base64. Whitespace is tolerated by
    // the engine but newlines are not, so strip them before decoding.
    let trimmed: Vec<u8> = body
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&trimmed) {
        if let Ok(s) = String::from_utf8(decoded) {
            return s;
        }
    }
    // URL-safe base64 is the other common variant.
    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE.decode(&trimmed) {
        if let Ok(s) = String::from_utf8(decoded) {
            return s;
        }
    }
    String::from_utf8_lossy(body).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_URI: &str = "vless://aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee@1.2.3.4:443?type=tcp&security=reality&pbk=abc&sid=def&sni=example.com&fp=chrome&flow=xtls-rprx-vision#Test";

    #[test]
    fn parses_plaintext_subscription() {
        let body = format!("{SAMPLE_URI}\nnot-a-vless-line\n{SAMPLE_URI}");
        let servers = parse_subscription_body(body.as_bytes());
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].address, "1.2.3.4");
    }

    #[test]
    fn parses_base64_subscription() {
        let inner = format!("{SAMPLE_URI}\n{SAMPLE_URI}\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(inner.as_bytes());
        let servers = parse_subscription_body(encoded.as_bytes());
        assert_eq!(servers.len(), 2);
    }

    #[test]
    fn skips_unparseable_lines() {
        let body = "vless://garbage\n".to_string() + SAMPLE_URI;
        let servers = parse_subscription_body(body.as_bytes());
        assert_eq!(servers.len(), 1);
    }
}
