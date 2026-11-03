//! Exit proxies: where traffic leaves the node.
//!
//! A sandbox's egress mode answers *how much of the traffic is observed*. This
//! answers *where it leaves*: through the firewall's DE exit, the CH one, the US
//! one, Tor, or the node's own route. The two are independent, so they are two
//! columns and not one enum with eight values.
//!
//! A proxy row holds no credential. `password_ref` is a locator (ADR 0005), and
//! the value is resolved where the connection is made.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// What protocol the exit speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    /// An HTTP proxy that understands `CONNECT` (the common case).
    Http,
    Socks5,
}

impl ProxyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Socks5 => "socks5",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "http" | "https" => Self::Http,
            "socks5" | "socks" => Self::Socks5,
            _ => return None,
        })
    }

    /// The URL scheme a client uses for this kind.
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Socks5 => "socks5h",
        }
    }
}

/// One exit proxy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Proxy {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: i32,
    /// DE, CH, US, AT, `tor`, or `None` when the exit is not a country.
    pub country: Option<String>,
    /// Where it comes from: `mullvad`, `tor`, `custom`.
    pub provider: Option<String>,
    pub username: Option<String>,
    /// Whether a password is configured. The value is never returned.
    pub password_configured: bool,
    pub password_ref: Option<String>,
    pub enabled: bool,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Proxy {
    /// The URL a client sets as `HTTP_PROXY` to use this exit.
    ///
    /// No credential in the URL: a password in a URL ends up in logs, in process
    /// listings and in the console. A proxy that needs authentication gets it
    /// from the secret locator where the connection is made.
    pub fn url(&self) -> String {
        format!("{}://{}:{}", self.kind.scheme(), self.host, self.port)
    }

    /// A label for a selector: `DE — exit 1 (HTTP)` reads better than a UUID.
    pub fn label(&self) -> String {
        self.name.clone()
    }
}

/// A proxy as it is created or edited.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewProxy {
    pub name: String,
    pub kind: ProxyKind,
    pub host: String,
    pub port: i32,
    pub country: Option<String>,
    pub provider: Option<String>,
    pub username: Option<String>,
    /// A locator such as `file:///etc/nestbox/secrets/exit-de`. Never a value.
    pub password_ref: Option<String>,
}

/// Whether an exit proxy is used at all, as the selectors show it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SandboxProxy {
    pub sandbox_id: Uuid,
    /// `None` means the node's own route.
    pub proxy: Option<Proxy>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Proxy {
        Proxy {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            name: "DE — exit 1 (HTTP)".into(),
            kind: ProxyKind::Http,
            host: "192.168.55.1".into(),
            port: 22541,
            country: Some("DE".into()),
            provider: Some("mullvad".into()),
            username: None,
            password_configured: false,
            password_ref: None,
            enabled: true,
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn an_http_proxy_url_names_the_host_and_port() {
        assert_eq!(sample().url(), "http://192.168.55.1:22541");
    }

    #[test]
    fn a_socks_proxy_uses_the_scheme_that_resolves_names_remotely() {
        let mut proxy = sample();
        proxy.kind = ProxyKind::Socks5;
        // `socks5h` and not `socks5`: the exit should resolve the name, so DNS
        // does not leak from the node that is supposed to be hidden.
        assert_eq!(proxy.url(), "socks5h://192.168.55.1:22541");
    }

    #[test]
    fn a_credential_never_appears_in_the_url() {
        let mut proxy = sample();
        proxy.username = Some("operator".into());
        proxy.password_configured = true;
        assert!(!proxy.url().contains("operator"));
        assert!(!proxy.url().contains('@'));
    }

    #[test]
    fn kinds_round_trip_through_their_wire_names() {
        for kind in [ProxyKind::Http, ProxyKind::Socks5] {
            assert_eq!(ProxyKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ProxyKind::parse("carrier-pigeon"), None);
    }
}
