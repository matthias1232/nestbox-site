//! Alerts: refused destinations nobody answered yet (contract C9).

use chrono::{DateTime, Utc};
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

#[cfg(test)]
mod tests {
    use super::*;

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
