//! The WireGuard tunnel of a joined node as the hub sees it, and the request to
//! connect a machine by its address (ADR 0057).
//!
//! The hub writes `wg show <if> dump` to a file every few seconds; this module
//! turns that text into a state per node. Pure, so the parsing is a unit test.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The hub talks to a node whose last handshake is younger than this (seconds).
pub const TUNNEL_UP_SECONDS: i64 = 180;

/// The hub thinks a node is still reachable for this long (seconds).
pub const TUNNEL_STALE_SECONDS: i64 = 600;

/// The tunnel of one node.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TunnelStatus {
    /// The node.
    pub node_id: Uuid,
    /// The node's name.
    pub node_name: String,
    /// `up`, `stale`, `down` or `unknown`.
    pub state: String,
    /// Seconds since the last handshake, when there was one.
    pub handshake_age_seconds: Option<i64>,
    /// Bytes received by the hub from the node.
    pub rx_bytes: i64,
    /// Bytes sent by the hub to the node.
    pub tx_bytes: i64,
    /// The node's address as the hub sees it.
    pub endpoint: Option<String>,
}

/// One peer line of `wg show <if> dump`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgPeer {
    /// The peer's public key (base64).
    pub public_key: String,
    /// The endpoint the hub last saw, when it saw one.
    pub endpoint: Option<String>,
    /// Unix time of the last handshake (`0` = never).
    pub latest_handshake: i64,
    /// Bytes the hub received from the peer.
    pub rx_bytes: i64,
    /// Bytes the hub sent to the peer.
    pub tx_bytes: i64,
}

/// Connect an existing machine by address and SSH credentials. Never stored.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ConnectNodeRequest {
    /// The node name to create.
    pub node_name: String,
    /// The site the node belongs to.
    pub site_id: Uuid,
    /// Public IP or host name of the machine.
    pub host: String,
    /// SSH port.
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    /// SSH user.
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    /// One-time password.
    #[serde(default)]
    pub password: Option<String>,
    /// One-time private key text (PEM or OpenSSH).
    #[serde(default)]
    pub private_key: Option<String>,
    /// UDP port the node's WireGuard listens on.
    #[serde(default = "default_wg_port")]
    pub wg_listen_port: u16,
    /// `strict` (everything through the hub) or `exit_local`.
    #[serde(default = "default_lock")]
    pub egress_lock: String,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_user() -> String {
    "root".to_string()
}

fn default_wg_port() -> u16 {
    51820
}

fn default_lock() -> String {
    "strict".to_string()
}

/// Parse `wg show <interface> dump` into peers.
///
/// The first line describes the interface and is skipped; every further line is
/// one peer with TAB-separated fields: public key, preshared key, endpoint,
/// allowed IPs, latest handshake, rx, tx, keepalive.
pub fn parse_wg_dump(text: &str) -> Vec<WgPeer> {
    let mut peers = Vec::new();
    for line in text.lines().skip(1) {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let endpoint = match fields[2] {
            "(none)" | "" => None,
            value => Some(value.to_string()),
        };
        peers.push(WgPeer {
            public_key: fields[0].to_string(),
            endpoint,
            latest_handshake: fields[4].parse().unwrap_or(0),
            rx_bytes: fields[5].parse().unwrap_or(0),
            tx_bytes: fields[6].parse().unwrap_or(0),
        });
    }
    peers
}

/// The state of one peer at `now` (unix seconds).
pub fn state_of(peer: &WgPeer, now: i64) -> (&'static str, Option<i64>) {
    if peer.latest_handshake == 0 {
        return ("down", None);
    }
    let age = (now - peer.latest_handshake).max(0);
    let state = if age < TUNNEL_UP_SECONDS {
        "up"
    } else if age < TUNNEL_STALE_SECONDS {
        "stale"
    } else {
        "down"
    };
    (state, Some(age))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "abcdef\tprivate\t51820\toff\n\
key-one\t(none)\t1.2.3.4:51820\t10.77.1.2/32\t1700000000\t120\t340\t25\n\
key-two\t(none)\t(none)\t10.77.2.2/32\t0\t0\t0\toff\n";

    #[test]
    fn the_interface_line_is_skipped() {
        let peers = parse_wg_dump(DUMP);
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].public_key, "key-one");
    }

    #[test]
    fn an_empty_endpoint_is_no_endpoint() {
        let peers = parse_wg_dump(DUMP);
        assert_eq!(peers[0].endpoint.as_deref(), Some("1.2.3.4:51820"));
        // A peer that never answered has no endpoint, written as `(none)` by `wg`.
        assert_eq!(peers[1].endpoint, None);
        assert_eq!(peers[0].rx_bytes, 120);
        assert_eq!(peers[1].tx_bytes, 0);
    }

    #[test]
    fn handshake_age_decides_the_state() {
        let peer = parse_wg_dump(DUMP).remove(0);
        assert_eq!(state_of(&peer, 1_700_000_060).0, "up");
        assert_eq!(state_of(&peer, 1_700_000_400).0, "stale");
        assert_eq!(state_of(&peer, 1_700_001_000).0, "down");
        let never = parse_wg_dump(DUMP).remove(1);
        assert_eq!(state_of(&never, 1_700_000_000), ("down", None));
    }
}
