//! Telemetry events: network, DNS, and inference.
//!
//! Three kinds because they answer three different questions, and merging them
//! into one shape would make every field optional. A DNS query has no port; a
//! model request has tokens; a connection has a decision and a rule id.
//!
//! All three share a correlation id and a timestamp, which is what lets the
//! console show them on one timeline when an operator is chasing a single
//! incident.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// What kind of event this is. The stream and the store both filter on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Network,
    Dns,
    Inference,
    Log,
    Policy,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Dns => "dns",
            Self::Inference => "inference",
            Self::Log => "log",
            Self::Policy => "policy",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "network" => Self::Network,
            "dns" => Self::Dns,
            "inference" => Self::Inference,
            "log" => Self::Log,
            "policy" => Self::Policy,
            _ => return None,
        })
    }

    /// The SSE stream that carries this kind. The console subscribes per stream
    /// so a busy firewall feed does not drown a quiet DNS feed.
    pub fn stream_name(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Dns => "dns",
            Self::Inference => "inference",
            Self::Log => "log",
            Self::Policy => "policy",
        }
    }
}

/// Direction of a connection relative to the sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}

/// What the policy decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    /// Recorded and allowed. Not a block, and the console says so.
    Audit,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Audit => "audit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "allow" => Self::Allow,
            "deny" => Self::Deny,
            "audit" => Self::Audit,
            _ => return None,
        })
    }

    /// Whether traffic actually passed.
    pub fn is_permitted(self) -> bool {
        matches!(self, Self::Allow | Self::Audit)
    }
}

/// A connection attempt observed at the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetworkEvent {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub gateway_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub direction: Direction,
    pub protocol: String,
    pub source: String,
    pub destination: String,
    pub port: Option<i32>,
    /// The executable that made the attempt. This is the field that makes the
    /// policy about the workload rather than the destination.
    pub binary: Option<String>,
    pub decision: Decision,
    pub rule_id: Option<Uuid>,
    pub rule_name: Option<String>,
    pub latency_ms: Option<i32>,
    pub correlation_id: Option<String>,
    /// `enforced` or `audit`, so an audit-mode deny is not mistaken for a wall.
    pub enforcement: Option<String>,
    /// The path the request took. Written by the ingest so the read path does
    /// not have to re-derive it from three proxy fields per row.
    pub channel: Option<String>,
    /// The name the client asked for. The only destination a tunnel reveals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sni: Option<String>,
    /// The listener that logged the line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listener: Option<String>,
    /// What the proxy recognised the application protocol as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_protocol: Option<String>,
}

/// A name lookup observed at the gateway.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DnsEvent {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub hostname: String,
    pub resolver: Option<String>,
    pub decision: Decision,
    pub binary: Option<String>,
    pub correlation_id: Option<String>,
}

/// A model request observed at the gateway.
///
/// This is the third stream, and the one that answers "what is my agent asking
/// the model, and did it cost me anything".
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InferenceEvent {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub provider_key: Option<String>,
    pub endpoint: Option<String>,
    pub model_key: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status_code: Option<i32>,
    pub latency_ms: Option<i32>,
    pub tokens_in: Option<i32>,
    pub tokens_out: Option<i32>,
    pub bytes_in: Option<i64>,
    pub bytes_out: Option<i64>,
    pub decision: Decision,
    pub correlation_id: Option<String>,
}

/// One event, whatever its kind. Used by the store and the bus, which must not
/// care which variant it is holding.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEvent {
    Network(NetworkEvent),
    Dns(DnsEvent),
    Inference(InferenceEvent),
}

impl TelemetryEvent {
    /// The environment this event belongs to.
    pub fn environment_id(&self) -> Uuid {
        match self {
            Self::Network(e) => e.environment_id,
            Self::Dns(e) => e.environment_id,
            Self::Inference(e) => e.environment_id,
        }
    }

    /// The sandbox, if the event is attributable to one.
    pub fn sandbox_id(&self) -> Option<Uuid> {
        match self {
            Self::Network(e) => e.sandbox_id,
            Self::Dns(e) => e.sandbox_id,
            Self::Inference(e) => e.sandbox_id,
        }
    }

    /// The kind, for filtering and partitioning.
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Network(_) => EventKind::Network,
            Self::Dns(_) => EventKind::Dns,
            Self::Inference(_) => EventKind::Inference,
        }
    }
}

/// A live event pushed to browsers: either a state change or a telemetry row.
///
/// Kept as one enum because a client subscribes to `events/stream` for state and
/// to a telemetry stream for data, and both must be expressible on the bus.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    Telemetry(TelemetryEvent),
    /// A model request left or was refused; `user_text` is masked.
    ChatStarted {
        environment_id: Uuid,
        sandbox_id: Uuid,
        request_ref: String,
        ts: DateTime<Utc>,
        model: Option<String>,
        host: Option<String>,
        provider: String,
        account_kind: Option<String>,
        dlp: Option<String>,
        delivered: bool,
        user_text: String,
    },
    /// The answer so far, masked. Never stored.
    ChatDelta {
        environment_id: Uuid,
        sandbox_id: Uuid,
        request_ref: String,
        text_so_far: String,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        done: bool,
    },
    /// A turn was stored in `chat_turns`.
    ChatDone {
        environment_id: Uuid,
        sandbox_id: Option<Uuid>,
        turn_id: Uuid,
        request_ref: Option<String>,
    },
    SandboxStateChanged {
        sandbox_id: Uuid,
        environment_id: Uuid,
        status: String,
        version: i64,
    },
    GatewayHealthChanged {
        gateway_id: Uuid,
        environment_id: Uuid,
        status: String,
        drifted: bool,
    },
    JobProgress {
        job_id: Uuid,
        environment_id: Uuid,
        completed: i32,
        total: i32,
        status: String,
    },
    PolicyRollout {
        policy_id: Uuid,
        environment_id: Uuid,
        applied: i32,
        total: i32,
    },
    /// Something the console shows on a sandbox changed: an agent's sign-in
    /// mode, the egress mode, a window, who is connected, a move step.
    SandboxControlChanged {
        /// The sandbox, or none for something environment-wide (a window).
        sandbox_id: Option<Uuid>,
        environment_id: Uuid,
        /// `agent_auth`, `egress_mode`, `window`, `presence`, `move`, `restart`.
        what: String,
        detail: serde_json::Value,
    },
    /// A sandbox's model list or the readiness of one model changed.
    ModelsChanged {
        /// The sandbox, or none for an environment-wide grant.
        sandbox_id: Option<Uuid>,
        environment_id: Uuid,
        detail: serde_json::Value,
    },
    /// A joined node's WireGuard tunnel changed state.
    NodeTunnelChanged {
        node_id: Uuid,
        environment_id: Uuid,
        state: String,
    },
    /// A refused destination nobody answered yet (contract C9).
    Alert {
        environment_id: Uuid,
        sandbox_id: Option<Uuid>,
        /// The alert view, shape in contract C9.
        alert: serde_json::Value,
    },
}

impl LiveEvent {
    /// Whether this is one of the three chat events.
    pub fn is_chat(&self) -> bool {
        matches!(
            self,
            Self::ChatStarted { .. } | Self::ChatDelta { .. } | Self::ChatDone { .. }
        )
    }

    /// The sandbox of a chat event.
    pub fn chat_sandbox(&self) -> Option<Uuid> {
        match self {
            Self::ChatStarted { sandbox_id, .. } | Self::ChatDelta { sandbox_id, .. } => {
                Some(*sandbox_id)
            }
            Self::ChatDone { sandbox_id, .. } => *sandbox_id,
            _ => None,
        }
    }


    /// The environment, so a stream can refuse to leak another tenant's events.
    pub fn environment_id(&self) -> Uuid {
        match self {
            Self::Telemetry(e) => e.environment_id(),
            Self::ChatStarted { environment_id, .. }
            | Self::ChatDelta { environment_id, .. }
            | Self::ChatDone { environment_id, .. } => *environment_id,
            Self::SandboxStateChanged { environment_id, .. } => *environment_id,
            Self::GatewayHealthChanged { environment_id, .. } => *environment_id,
            Self::JobProgress { environment_id, .. } => *environment_id,
            Self::PolicyRollout { environment_id, .. } => *environment_id,
            Self::SandboxControlChanged { environment_id, .. } => *environment_id,
            Self::ModelsChanged { environment_id, .. } => *environment_id,
            Self::NodeTunnelChanged { environment_id, .. } => *environment_id,
            Self::Alert { environment_id, .. } => *environment_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_permits_traffic_but_is_not_a_block() {
        assert!(Decision::Audit.is_permitted());
        assert!(Decision::Allow.is_permitted());
        assert!(!Decision::Deny.is_permitted());
    }

    #[test]
    fn kinds_map_to_their_stream_name() {
        assert_eq!(EventKind::Network.stream_name(), "network");
        assert_eq!(EventKind::Inference.stream_name(), "inference");
    }

    #[test]
    fn kind_round_trips_through_its_string() {
        for kind in [
            EventKind::Network,
            EventKind::Dns,
            EventKind::Inference,
            EventKind::Log,
            EventKind::Policy,
        ] {
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(EventKind::parse("nonsense"), None);
    }

    #[test]
    fn an_unknown_decision_is_not_defaulted_to_allow() {
        // Defaulting to allow would turn a parsing gap into a security gap.
        assert_eq!(Decision::parse("maybe"), None);
    }
}
