//! How an agent in a sandbox is signed in, and whether that holds.
//!
//! Two ways exist and they can coexist. The agent can go through the **gateway** (the
//! appliance's model endpoint with its key, set in the environment, reached through the
//! sandbox's supervisor), or the user can sign in with an **own account** (Claude's
//! OAuth login, stored on the persistent home). When both are present the environment
//! key wins, so the stored account is idle until the key is removed; saying so is the
//! point of this module.
//!
//! The probe reads files and environment inside the sandbox; the decision about what
//! they mean is made here, in plain functions, so it is tested without a sandbox.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What the probe found inside the sandbox.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Probe {
    /// A model endpoint key is present in the agent's environment.
    #[serde(default)]
    pub gateway_key: bool,
    /// The user's own account, when a login is stored.
    #[serde(default)]
    pub account: Option<Account>,
    /// One short label per agent that has a login of the user's own, keyed by
    /// the agent name (`claude`, `codex`, `opencode`). An absent key means the
    /// agent has no own login; the label is what the user would recognise it
    /// by (an e-mail address, "signed in", or the provider list).
    #[serde(default)]
    pub accounts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct Account {
    pub provider: String,
    pub email: Option<String>,
    /// Unix milliseconds; absent when the login does not say.
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginMode {
    /// Only the gateway key.
    Gateway,
    /// Only the user's own account.
    OwnAccount,
    /// Both: the key is used, the account is idle.
    Both,
    /// Neither: the agent cannot reach a model.
    None,
    /// The probe could not run (no python3 in the image, sandbox not answering).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Ok,
    Warn,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LoginState {
    pub mode: LoginMode,
    pub check: Check,
    /// One sentence for the operator: what is in use and what to know about it.
    pub detail: String,
    pub account: Option<Account>,
    pub account_expired: bool,
}

/// Decide the login state from what was found. `now_ms` is the current time in Unix
/// milliseconds; `gateway_reachable` is the last probe of the provider the key belongs to.
pub fn assess(probe: Option<&Probe>, now_ms: i64, gateway_reachable: bool) -> LoginState {
    let Some(probe) = probe else {
        return LoginState {
            mode: LoginMode::Unknown,
            check: Check::Unknown,
            detail: "The sandbox did not answer the check (it may be stopped, or lack python3).".into(),
            account: None,
            account_expired: false,
        };
    };
    let expired = probe
        .account
        .as_ref()
        .and_then(|account| account.expires_at_ms)
        .is_some_and(|at| at <= now_ms);
    let who = probe
        .account
        .as_ref()
        .and_then(|account| account.email.clone())
        .unwrap_or_else(|| "an account".to_string());
    let (mode, check, detail) = match (probe.gateway_key, probe.account.is_some()) {
        (true, false) => (
            LoginMode::Gateway,
            if gateway_reachable { Check::Ok } else { Check::Warn },
            if gateway_reachable {
                "Signed in through the gateway key; the model endpoint answered its last probe.".to_string()
            } else {
                "Signed in through the gateway key, but the model endpoint did not answer its last probe.".to_string()
            },
        ),
        (false, true) if expired => (
            LoginMode::OwnAccount,
            Check::Error,
            format!("Signed in as {who}, but the login has expired. Sign in again inside the sandbox."),
        ),
        (false, true) => (
            LoginMode::OwnAccount,
            Check::Ok,
            format!("Signed in as {who}. The login is stored on the persistent home and survives a restart or recreate. It is checked for expiry only: a revoked login shows when the agent next calls."),
        ),
        (true, true) => (
            LoginMode::Both,
            Check::Warn,
            format!("Both are present. The gateway key is used; {who} is stored but idle until the key is removed from the environment."),
        ),
        (false, false) => (
            LoginMode::None,
            Check::Error,
            "Not signed in: no gateway key and no stored account. The agent cannot reach a model.".to_string(),
        ),
    };
    LoginState {
        mode,
        check,
        detail,
        account: probe.account.clone(),
        account_expired: expired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(expires: Option<i64>) -> Account {
        Account { provider: "claude".into(), email: Some("me@example.test".into()), expires_at_ms: expires }
    }

    #[test]
    fn only_the_gateway_key_is_a_gateway_login_checked_by_the_provider_probe() {
        let probe = Probe { gateway_key: true, ..Probe::default() };
        assert_eq!(assess(Some(&probe), 0, true).mode, LoginMode::Gateway);
        assert_eq!(assess(Some(&probe), 0, true).check, Check::Ok);
        assert_eq!(assess(Some(&probe), 0, false).check, Check::Warn);
    }

    #[test]
    fn an_own_account_is_ok_until_it_expires_and_then_an_error() {
        let valid = Probe {
            gateway_key: false,
            account: Some(account(Some(2_000))),
            ..Probe::default()
        };
        assert_eq!(assess(Some(&valid), 1_000, true).check, Check::Ok);
        let state = assess(Some(&valid), 3_000, true);
        assert_eq!(state.check, Check::Error);
        assert!(state.account_expired);
        assert!(state.detail.contains("expired"));
    }

    #[test]
    fn an_account_without_an_expiry_is_not_called_expired() {
        let probe = Probe { gateway_key: false, account: Some(account(None)), ..Probe::default() };
        assert!(!assess(Some(&probe), i64::MAX, true).account_expired);
    }

    #[test]
    fn both_at_once_warns_that_the_key_wins() {
        let probe = Probe { gateway_key: true, account: Some(account(None)), ..Probe::default() };
        let state = assess(Some(&probe), 0, true);
        assert_eq!(state.mode, LoginMode::Both);
        assert_eq!(state.check, Check::Warn);
        assert!(state.detail.contains("idle"));
    }

    #[test]
    fn neither_is_an_error_and_no_probe_is_unknown() {
        assert_eq!(assess(Some(&Probe::default()), 0, true).mode, LoginMode::None);
        assert_eq!(assess(Some(&Probe::default()), 0, true).check, Check::Error);
        assert_eq!(assess(None, 0, true).mode, LoginMode::Unknown);
        assert_eq!(assess(None, 0, true).check, Check::Unknown);
    }

    #[test]
    fn a_probe_answer_names_the_agents_that_have_an_own_login() {
        let probe: Probe = serde_json::from_str(
            r#"{"gateway_key": false, "account": null, "accounts": {"codex": "signed in"}}"#,
        )
        .unwrap();
        assert_eq!(probe.accounts.get("codex").map(String::as_str), Some("signed in"));
        assert!(!probe.accounts.contains_key("claude"));
        assert!(!probe.gateway_key);
    }

    #[test]
    fn a_probe_answer_without_accounts_still_deserialises() {
        let probe: Probe =
            serde_json::from_str(r#"{"gateway_key": true, "account": null}"#).unwrap();
        assert!(probe.accounts.is_empty());
        assert!(probe.gateway_key);
    }
}
