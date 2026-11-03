//! Alerts: refused destinations nobody answered yet (contract C9).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

/// One alert as the API and the `alert` live event carry it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlertView {
    pub id: String,
    pub sandbox_id: Uuid,
    pub sandbox_name: String,
    pub host: String,
    pub port: i32,
    pub protocol: String,
    /// `proxy`, `dns`, `mcp` or `gateway`.
    pub channel: String,
    pub count: i64,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub decision: String,
    pub reason: String,
    pub would_deny: bool,
    pub owner: Option<String>,
}

/// A stable id for "this sandbox was refused this destination".
///
/// Hex digest of `sandbox|host|port`, so the same destination always groups into one alert
/// and the console can remember an ignored one.
pub fn alert_id(sandbox: Uuid, host: &str, port: i32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{sandbox}|{}|{port}", host.to_ascii_lowercase()));
    let digest = hasher.finalize();
    digest.iter().take(20).map(|byte| format!("{byte:02x}")).collect()
}

/// The length of a window word: `5m`, `15m`, `1h` or `24h`; anything else is `None`.
pub fn window(spec: &str) -> Option<Duration> {
    match spec {
        "5m" => Some(Duration::minutes(5)),
        "15m" => Some(Duration::minutes(15)),
        "1h" => Some(Duration::hours(1)),
        "24h" => Some(Duration::hours(24)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_four_window_words_parse() {
        assert_eq!(window("15m"), Some(Duration::minutes(15)));
        assert_eq!(window("24h"), Some(Duration::hours(24)));
        assert!(window("2h").is_none());
        assert!(window("").is_none());
    }

    #[test]
    fn the_same_destination_gets_the_same_id() {
        let sandbox = Uuid::nil();
        assert_eq!(
            alert_id(sandbox, "API.example.net", 443),
            alert_id(sandbox, "api.example.net", 443)
        );
        assert_ne!(alert_id(sandbox, "a.net", 443), alert_id(sandbox, "a.net", 80));
        assert_eq!(alert_id(sandbox, "a.net", 443).len(), 40);
    }
}
