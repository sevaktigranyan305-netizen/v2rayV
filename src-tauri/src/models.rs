use serde::{Deserialize, Serialize};
use thiserror::Error;

fn generate_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "generate_id")]
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub uuid: String,
    pub flow: String,
    pub reality: RealitySettings,
    /// L3 virtual-network hints from our 3x-ui fork. When present and
    /// `enabled == true`, the xray-core fork's `l3client` outbound takes
    /// over and we run **without** SOCKS/HTTP inbounds or system proxy —
    /// xray itself owns the TUN adapter (wintun on Windows, utun on
    /// macOS, native Linux TUN). Mirrors `vnet=` / `vnetSubnet=` /
    /// `vnetIp=` / `vnetDefaultRoute=` query params from the share-link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtualnet: Option<VirtualNetSettings>,
    /// Tag set on every server pulled from a saved subscription. Used by
    /// `refresh_subscription` to know which servers to wipe before
    /// re-importing the freshly-fetched list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<String>,
}

/// L3 virtual-network configuration matching the 3x-ui inbound's
/// `virtualNetwork` block. Only `subnet` and `vnet_ip` are required; the
/// rest have sensible defaults so a minimal `vnet=1&vnetIp=10.0.0.5`
/// share-link is enough to bring the tunnel up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VirtualNetSettings {
    pub enabled: bool,
    /// CIDR like "10.0.0.0/24". Must be IPv4 — the xray-core fork's
    /// `l3client` only supports IPv4 subnets and the panel's IPAM only
    /// hands out IPv4 addresses.
    pub subnet: String,
    /// Pre-allocated per-uuid IPv4 from the panel's IPAM (e.g. "10.0.0.5").
    pub vnet_ip: String,
    /// When true, route 0.0.0.0/0 through the TUN. Mirrors
    /// `vnetDefaultRoute=1` from the share-link; on the panel side this
    /// is forced to true so the safe default here is also true.
    #[serde(default = "default_default_route")]
    pub default_route: bool,
    /// TUN interface name, defaults to "v2rayV" inside xray-core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    /// Optional MTU override; xray-core picks a sane default if 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
}

fn default_default_route() -> bool {
    true
}

/// A saved subscription: name + URL, with last-refresh timestamp. The
/// servers it produced are linked via `ServerConfig.subscription_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    #[serde(default = "generate_id")]
    pub id: String,
    pub name: String,
    pub url: String,
    /// Unix-seconds timestamp of the last successful refresh, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_updated_at: Option<u64>,
    /// Number of servers the last refresh produced. Surfaced in the UI
    /// so the user can sanity-check that the URL still returns configs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_server_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealitySettings {
    pub public_key: String,
    pub short_id: String,
    pub server_name: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub status: ConnectionStatus,
    pub server_name: Option<String>,
    pub server_address: Option<String>,
    pub connected_since: Option<u64>,
    pub error_message: Option<String>,
}

impl Default for ConnectionInfo {
    fn default() -> Self {
        Self {
            status: ConnectionStatus::Disconnected,
            server_name: None,
            server_address: None,
            connected_since: None,
            error_message: None,
        }
    }
}

impl ServerConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.address.trim().is_empty() {
            return Err("Server address must not be empty".to_string());
        }

        if self.port == 0 {
            return Err("Server port must be greater than 0".to_string());
        }

        if !Self::is_valid_uuid(&self.uuid) {
            return Err(
                "UUID must match format xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx (hex characters)"
                    .to_string(),
            );
        }

        if self.reality.public_key.trim().is_empty() {
            return Err("Reality public_key must not be empty".to_string());
        }

        if self.reality.short_id.trim().is_empty() {
            return Err("Reality short_id must not be empty".to_string());
        }

        Ok(())
    }

    fn is_valid_uuid(s: &str) -> bool {
        // Expected format: 8-4-4-4-12 hex chars separated by hyphens
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 5 {
            return false;
        }

        let expected_lengths = [8, 4, 4, 4, 12];
        for (part, &expected_len) in parts.iter().zip(&expected_lengths) {
            if part.len() != expected_len || !part.chars().all(|c| c.is_ascii_hexdigit()) {
                return false;
            }
        }

        true
    }
}

impl Default for RealitySettings {
    fn default() -> Self {
        Self {
            public_key: String::new(),
            short_id: String::new(),
            server_name: "www.google.com".to_string(),
            fingerprint: "chrome".to_string(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            id: generate_id(),
            name: String::new(),
            address: String::new(),
            port: 443,
            uuid: String::new(),
            flow: "xtls-rprx-vision".to_string(),
            reality: RealitySettings::default(),
            virtualnet: None,
            subscription_id: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpeedStats {
    pub upload_speed: u64,
    pub download_speed: u64,
    pub total_upload: u64,
    pub total_download: u64,
}

/// A detected VPN interface with its routed subnets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedVpn {
    /// Interface name (e.g. "tun0", "wg0")
    pub interface: String,
    /// Human-readable VPN type (e.g. "OpenVPN", "WireGuard")
    pub vpn_type: String,
    /// Subnets routed through this interface (e.g. ["10.8.0.0/24", "172.20.0.0/16"])
    pub subnets: Vec<String>,
    /// VPN server endpoint IP if detected
    pub server_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub auto_connect: bool,
    pub last_server_id: Option<String>,
    #[serde(default = "default_bypass_domains")]
    pub bypass_domains: Vec<String>,
}

fn default_bypass_domains() -> Vec<String> {
    vec![
        "claude.ai".to_string(),
        "anthropic.com".to_string(),
        "api.anthropic.com".to_string(),
        "wb.ru".to_string(),
        "wildberries.ru".to_string(),
    ]
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_connect: false,
            last_server_id: None,
            bypass_domains: default_bypass_domains(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Xray process error: {0}")]
    XrayProcess(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),
}

impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        err.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_server_config() -> ServerConfig {
        ServerConfig {
            id: "test-id-1234".to_string(),
            name: "Test".to_string(),
            address: "1.2.3.4".to_string(),
            port: 443,
            uuid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            flow: "xtls-rprx-vision".to_string(),
            reality: RealitySettings {
                public_key: "abc123".to_string(),
                short_id: "def456".to_string(),
                server_name: "example.com".to_string(),
                fingerprint: "chrome".to_string(),
            },
            virtualnet: None,
            subscription_id: None,
        }
    }

    #[test]
    fn server_config_roundtrip() {
        let config = sample_server_config();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ServerConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, config.name);
        assert_eq!(deserialized.address, config.address);
        assert_eq!(deserialized.port, config.port);
        assert_eq!(deserialized.uuid, config.uuid);
        assert_eq!(deserialized.flow, config.flow);
        assert_eq!(deserialized.reality.public_key, config.reality.public_key);
        assert_eq!(deserialized.reality.short_id, config.reality.short_id);
        assert_eq!(deserialized.reality.server_name, config.reality.server_name);
        assert_eq!(deserialized.reality.fingerprint, config.reality.fingerprint);
    }

    #[test]
    fn connection_status_serialization() {
        // Verify snake_case rename
        let json = serde_json::to_string(&ConnectionStatus::Disconnected).unwrap();
        assert_eq!(json, "\"disconnected\"");

        let json = serde_json::to_string(&ConnectionStatus::Connecting).unwrap();
        assert_eq!(json, "\"connecting\"");

        let json = serde_json::to_string(&ConnectionStatus::Connected).unwrap();
        assert_eq!(json, "\"connected\"");

        let json = serde_json::to_string(&ConnectionStatus::Disconnecting).unwrap();
        assert_eq!(json, "\"disconnecting\"");

        let json = serde_json::to_string(&ConnectionStatus::Error).unwrap();
        assert_eq!(json, "\"error\"");
    }

    #[test]
    fn connection_status_deserialization() {
        let status: ConnectionStatus = serde_json::from_str("\"disconnected\"").unwrap();
        assert_eq!(status, ConnectionStatus::Disconnected);

        let status: ConnectionStatus = serde_json::from_str("\"connected\"").unwrap();
        assert_eq!(status, ConnectionStatus::Connected);

        let status: ConnectionStatus = serde_json::from_str("\"error\"").unwrap();
        assert_eq!(status, ConnectionStatus::Error);
    }

    #[test]
    fn connection_info_roundtrip() {
        let info = ConnectionInfo {
            status: ConnectionStatus::Connected,
            server_name: Some("My Server".to_string()),
            server_address: Some("1.2.3.4".to_string()),
            connected_since: Some(1700000000),
            error_message: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ConnectionInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.status, ConnectionStatus::Connected);
        assert_eq!(deserialized.server_name, Some("My Server".to_string()));
        assert_eq!(deserialized.server_address, Some("1.2.3.4".to_string()));
        assert_eq!(deserialized.connected_since, Some(1700000000));
        assert_eq!(deserialized.error_message, None);
    }

    #[test]
    fn connection_info_default() {
        let info = ConnectionInfo::default();
        assert_eq!(info.status, ConnectionStatus::Disconnected);
        assert!(info.server_name.is_none());
        assert!(info.server_address.is_none());
        assert!(info.connected_since.is_none());
        assert!(info.error_message.is_none());
    }

    #[test]
    fn server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 443);
        assert_eq!(config.flow, "xtls-rprx-vision");
        assert_eq!(config.reality.server_name, "www.google.com");
        assert_eq!(config.reality.fingerprint, "chrome");
    }

    #[test]
    fn app_error_to_string() {
        let err = AppError::XrayProcess("failed to start".to_string());
        let s: String = err.into();
        assert_eq!(s, "Xray process error: failed to start");

        let err = AppError::Config("bad value".to_string());
        let s: String = err.into();
        assert_eq!(s, "Configuration error: bad value");
    }

    #[test]
    fn app_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let app_err: AppError = io_err.into();
        let s: String = app_err.into();
        assert!(s.contains("IO error"));
        assert!(s.contains("file missing"));
    }

    #[test]
    fn validate_valid_config() {
        let config = sample_server_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_empty_address() {
        let mut config = sample_server_config();
        config.address = "".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("address"));
    }

    #[test]
    fn validate_whitespace_address() {
        let mut config = sample_server_config();
        config.address = "   ".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("address"));
    }

    #[test]
    fn validate_port_zero() {
        let mut config = sample_server_config();
        config.port = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("port"));
    }

    #[test]
    fn validate_invalid_uuid_too_short() {
        let mut config = sample_server_config();
        config.uuid = "not-a-uuid".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("UUID"));
    }

    #[test]
    fn validate_invalid_uuid_bad_chars() {
        let mut config = sample_server_config();
        config.uuid = "gggggggg-hhhh-iiii-jjjj-kkkkkkkkkkkk".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("UUID"));
    }

    #[test]
    fn validate_valid_uuid_lowercase() {
        let mut config = sample_server_config();
        config.uuid = "01234567-89ab-cdef-0123-456789abcdef".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_valid_uuid_uppercase() {
        let mut config = sample_server_config();
        config.uuid = "01234567-89AB-CDEF-0123-456789ABCDEF".to_string();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_empty_public_key() {
        let mut config = sample_server_config();
        config.reality.public_key = "".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("public_key"));
    }

    #[test]
    fn validate_empty_short_id() {
        let mut config = sample_server_config();
        config.reality.short_id = "  ".to_string();
        let err = config.validate().unwrap_err();
        assert!(err.contains("short_id"));
    }
}
