//! Who is signed in where: the login of each running sandbox, and who is connected.
//!
//! One read for the whole overview. Each running sandbox is asked, in parallel and with
//! a short limit, what its agent is signed in with (files and environment inside it); a
//! sandbox that does not answer is reported as unknown rather than slowing the page.
//! Open SSH and terminal sessions come from the session audit.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::agent_auth::{self, AgentKind};
use crate::domain::auth::{Permission, Principal};
use crate::domain::login::{self, LoginState, Probe};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::postgres::agent_auth::{self as agent_auth_store, AgentAuthRow};
use crate::store::TenantScope;

const PROBE_SECONDS: u32 = 6;

/// What runs inside the sandbox. python3 is in the agent images; its absence is an
/// "unknown", not a failure.
const PROBE: &str = r#"python3 - <<'PY'
import json, os
home = os.path.expanduser("~")
out = {"gateway_key": bool(os.environ.get("ANTHROPIC_AUTH_TOKEN") or os.environ.get("OPENAI_API_KEY")), "account": None}
cfg = os.environ.get("CLAUDE_CONFIG_DIR") or home + "/.claude"
try:
    oauth = json.load(open(cfg + "/.credentials.json")).get("claudeAiOauth") or {}
    state = {}
    try:
        state = json.load(open(cfg + "/.claude.json"))
    except Exception:
        pass
    out["account"] = {"provider": "claude", "email": (state.get("oauthAccount") or {}).get("emailAddress"), "expires_at_ms": oauth.get("expiresAt")}
except Exception:
    pass
if out["account"] is None:
    try:
        keys = list(json.load(open(home + "/.local/share/opencode/auth.json")).keys())
        if keys:
            out["account"] = {"provider": "opencode:" + ",".join(keys), "email": None, "expires_at_ms": None}
    except Exception:
        pass
accounts = {}
state = home + "/.agent-state"
cdir = os.environ.get("CLAUDE_CONFIG_DIR") or (state + "/claude" if os.path.isdir(state) else home + "/.claude")
try:
    oauth = json.load(open(cdir + "/.credentials.json")).get("claudeAiOauth") or {}
    if oauth:
        email = None
        try:
            email = (json.load(open(cdir + "/.claude.json")).get("oauthAccount") or {}).get("emailAddress")
        except Exception:
            pass
        accounts["claude"] = email or "signed in"
except Exception:
    pass
for path in (state + "/codex/auth.json", home + "/.codex/auth.json"):
    try:
        data = json.load(open(path))
        if data:
            accounts["codex"] = "signed in"
            break
    except Exception:
        pass
for path in (state + "/xdg-data/opencode/auth.json", home + "/.local/share/opencode/auth.json"):
    try:
        keys = list(json.load(open(path)).keys())
        if keys:
            accounts["opencode"] = "providers: " + ",".join(keys)
            break
    except Exception:
        pass
out["accounts"] = accounts
print(json.dumps(out))
PY"#;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SandboxLogin {
    pub sandbox_id: Uuid,
    pub sandbox_name: String,
    pub pack_key: String,
    pub login: LoginState,
    /// The switchable agents of the sandbox, each with the chosen sign-in mode
    /// and whether an own login was found: `{"agent", "mode", "signed_in"}`.
    /// `signed_in` is `null` when the sandbox did not answer the probe.
    pub agents: Vec<serde_json::Value>,
    /// The supervisor is connected to the gateway, so the sandbox is mediated by OpenShell.
    pub openshell_connected: bool,
    pub ssh_sessions: i64,
    pub terminal_sessions: i64,
}

/// The switchable agents of one sandbox, for the overview: which agents it has,
/// the stored modes and the old create-time login that decides the default.
struct SandboxAgents {
    agents: Vec<AgentKind>,
    rows: Vec<AgentAuthRow>,
    login: Option<String>,
}

impl SandboxAgents {
    /// One item per switchable agent: the mode it uses and whether the probe
    /// found a login of the user's own (`null` when the probe did not run).
    fn items(&self, probe: Option<&Probe>) -> Vec<serde_json::Value> {
        let default = agent_auth::default_mode(self.login.as_deref());
        let mut out = Vec::new();
        for agent in AgentKind::ALL {
            if !self.agents.contains(&agent) {
                continue;
            }
            let mode = self
                .rows
                .iter()
                .find(|row| row.agent == agent)
                .map_or(default, |row| row.mode);
            let signed_in = match probe {
                Some(probe) => serde_json::Value::Bool(probe.accounts.contains_key(agent.as_str())),
                None => serde_json::Value::Null,
            };
            out.push(serde_json::json!({
                "agent": agent.as_str(),
                "mode": mode.as_str(),
                "signed_in": signed_in,
            }));
        }
        out
    }
}

/// The login overview: one row per running sandbox, with the login state of its
/// agent and now also the chosen mode of every switchable agent.
pub async fn overview(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<SandboxLogin>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let db = |error: sqlx::Error| ApiError::from(StoreError::Database(error));
    let rows: Vec<(Uuid, String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT s.id, s.name, s.pack_key, v.status, s.spec->>'login'
           FROM sandboxes s LEFT JOIN supervisors v ON v.id = s.supervisor_id
          WHERE s.status = 'running' ORDER BY s.name",
    )
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let sessions: Vec<(Uuid, String, i64)> = sqlx::query_as(
        "SELECT sandbox_id, kind, count(*) FROM sandbox_sessions WHERE closed_at IS NULL GROUP BY sandbox_id, kind",
    )
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let gateway_reachable: bool = sqlx::query_scalar(
        "SELECT COALESCE(bool_or(status = 'reachable'), false) FROM provider_profiles WHERE kind = 'omniroute'",
    )
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;
    let mut agents = Vec::with_capacity(rows.len());
    for (id, _, pack_key, _, login) in &rows {
        agents.push(agents_of(&mut scope, *id, pack_key, login.clone()).await?);
    }
    scope.commit().await.map_err(ApiError::from)?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let probes = rows
        .iter()
        .map(|(_, name, ..)| async move { probe_once(ctx, name).await.ok() });
    let probed = futures::future::join_all(probes).await;

    let count = |id: Uuid, kind: &str| {
        sessions.iter().find(|(sandbox, k, _)| *sandbox == id && k == kind).map_or(0, |(_, _, n)| *n)
    };
    Ok(rows
        .into_iter()
        .zip(probed)
        .zip(agents)
        .map(|(((id, name, pack_key, supervisor_status, _), probe), agents)| SandboxLogin {
            sandbox_id: id,
            sandbox_name: name,
            pack_key,
            login: login::assess(probe.as_ref(), now_ms, gateway_reachable),
            agents: agents.items(probe.as_ref()),
            openshell_connected: supervisor_status.as_deref() == Some("running"),
            ssh_sessions: count(id, "ssh"),
            terminal_sessions: count(id, "terminal"),
        })
        .collect())
}

/// The switchable agents of one sandbox: the agents its pack can sign in, plus
/// those of the packs its recipes were made from, and the stored modes.
async fn agents_of(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    pack_key: &str,
    login: Option<String>,
) -> Result<SandboxAgents, ApiError> {
    let mut list = agent_auth_store::pack_agents(scope, pack_key).await.map_err(ApiError::from)?;
    let recipes =
        agent_auth_store::sandbox_agent_packs(scope, sandbox_id).await.map_err(ApiError::from)?;
    for pack in recipes {
        for agent in agent_auth_store::pack_agents(scope, &pack).await.map_err(ApiError::from)? {
            if !list.contains(&agent) {
                list.push(agent);
            }
        }
    }
    let rows = agent_auth_store::list(scope, sandbox_id).await.map_err(ApiError::from)?;
    Ok(SandboxAgents { agents: list, rows, login })
}

/// Ask one sandbox what its agents are signed in with. `Err` means the probe
/// could not run at all (no container, no python3, no answer in time).
async fn probe_once(ctx: &ServiceContext, sandbox_name: &str) -> Result<Probe, ApiError> {
    let command = vec!["sh".to_string(), "-c".to_string(), PROBE.to_string()];
    let answer = tokio::time::timeout(
        Duration::from_secs(u64::from(PROBE_SECONDS) + 2),
        ctx.runtime.exec(sandbox_name, &command, PROBE_SECONDS),
    )
    .await;
    match answer {
        Ok(Ok(result)) if result.exit_code == Some(0) => {
            let parsed = result
                .output
                .lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Probe>(line.trim()).ok());
            parsed.ok_or_else(|| {
                ApiError::unavailable(
                    ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                    "The sandbox answered the login probe with no readable result.",
                )
                .with_internal(result.output.clone())
            })
        }
        Ok(Ok(result)) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The sandbox could not run the login probe.",
        )
        .with_internal(format!("the probe exited with {:?}", result.exit_code))),
        Ok(Err(error)) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The node did not answer the login probe.",
        )
        .with_internal(error.to_string())),
        Err(_) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The sandbox did not answer the login probe in time.",
        )),
    }
}

/// The account label of each switchable agent inside one sandbox, when there is
/// a login of the user's own; `None` means no own login was found.
///
/// The map always has the keys `claude`, `codex` and `opencode`, so a caller can
/// tell "no own login" from "an agent this build does not know".
pub async fn agent_accounts(
    ctx: &ServiceContext,
    sandbox_name: &str,
) -> Result<BTreeMap<String, Option<String>>, ApiError> {
    let probe = probe_once(ctx, sandbox_name).await?;
    let mut out = BTreeMap::new();
    for agent in AgentKind::ALL {
        out.insert(agent.as_str().to_string(), probe.accounts.get(agent.as_str()).cloned());
    }
    Ok(out)
}
