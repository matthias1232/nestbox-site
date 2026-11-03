//! Policies, versions, network rules, drafts, and traffic windows.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::policy::{
    DnsRule, EgressMode, EgressModeSetting, Enforcement, Matcher, NetworkRule, Policy,
    PolicyAssignment, PolicyScope, PolicyTemplate, RuleAction, RuleProtocol,
};
use crate::error::StoreError;

fn row_to_policy(row: &sqlx::postgres::PgRow) -> Result<Policy, StoreError> {
    let scope_raw: String = row.try_get("scope")?;
    Ok(Policy {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        name: row.try_get("name")?,
        scope: PolicyScope::parse(&scope_raw).unwrap_or(PolicyScope::Sandbox),
        spec: row.try_get("spec")?,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const POLICY_COLUMNS: &str =
    "id, environment_id, name, scope, spec, version, created_at, updated_at";

/// All policies in this environment.
pub async fn list(scope: &mut TenantScope<'_>) -> Result<Vec<Policy>, StoreError> {
    let sql = format!(
        "SELECT {POLICY_COLUMNS} FROM policies WHERE environment_id = $1 ORDER BY name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_policy(&row)?);
    }
    Ok(out)
}

/// One policy by id.
pub async fn get(scope: &mut TenantScope<'_>, id: Uuid) -> Result<Policy, StoreError> {
    let sql = format!(
        "SELECT {POLICY_COLUMNS} FROM policies WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_policy(&row)
}

/// The active global policy, if one exists.
///
/// At most one can exist; the schema enforces it with a partial unique index.
pub async fn global(scope: &mut TenantScope<'_>) -> Result<Option<Policy>, StoreError> {
    let sql = format!(
        "SELECT {POLICY_COLUMNS} FROM policies
          WHERE environment_id = $1 AND scope = 'global' LIMIT 1"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await?;
    match row {
        Some(row) => Ok(Some(row_to_policy(&row)?)),
        None => Ok(None),
    }
}

/// Insert a policy and record its first version.
pub async fn insert(
    scope: &mut TenantScope<'_>,
    name: &str,
    scope_value: PolicyScope,
    spec: &serde_json::Value,
    created_by: Option<Uuid>,
) -> Result<Policy, StoreError> {
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO policies (environment_id, name, scope, spec, version, created_by)
            VALUES ($1, $2, $3, $4, 1, $5)
            RETURNING {POLICY_COLUMNS}
         )
         SELECT {POLICY_COLUMNS} FROM inserted"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(name)
        .bind(scope_value.as_str())
        .bind(spec)
        .bind(created_by)
        .fetch_one(scope.connection())
        .await?;

    let policy = row_to_policy(&row)?;
    sqlx::query(
        "INSERT INTO policy_versions (policy_id, version, spec, created_by)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(policy.id)
    .bind(policy.version)
    .bind(&policy.spec)
    .bind(created_by)
    .execute(scope.connection())
    .await?;

    Ok(policy)
}

/// Replace a policy document, guarded by the expected version.
///
/// Writes a new row into `policy_versions` so rollback has something to read.
pub async fn update(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    expected_version: i64,
    spec: &serde_json::Value,
    actor: Option<Uuid>,
) -> Result<Policy, StoreError> {
    let sql = format!(
        "WITH updated AS (
            UPDATE policies
               SET spec = $3, version = version + 1, updated_at = now()
             WHERE environment_id = $1 AND id = $2 AND version = $4
         RETURNING {POLICY_COLUMNS}
         )
         SELECT {POLICY_COLUMNS} FROM updated"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(spec)
        .bind(expected_version)
        .fetch_optional(scope.connection())
        .await?;

    let Some(row) = row else {
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM policies WHERE environment_id = $1 AND id = $2",
        )
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
        return if exists.is_some() {
            Err(StoreError::VersionConflict)
        } else {
            Err(StoreError::NotFound)
        };
    };

    let policy = row_to_policy(&row)?;
    sqlx::query(
        "INSERT INTO policy_versions (policy_id, version, spec, created_by)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(policy.id)
    .bind(policy.version)
    .bind(&policy.spec)
    .bind(actor)
    .execute(scope.connection())
    .await?;

    Ok(policy)
}

/// A stored revision, for the diff view.
pub async fn version_spec(
    scope: &mut TenantScope<'_>,
    policy_id: Uuid,
    version: i64,
) -> Result<serde_json::Value, StoreError> {
    // The join to policies is what keeps this tenant-scoped: a version row alone
    // does not carry an environment id.
    let row = sqlx::query(
        "SELECT v.spec
           FROM policy_versions v
           JOIN policies p ON p.id = v.policy_id
          WHERE p.environment_id = $1 AND v.policy_id = $2 AND v.version = $3",
    )
    .bind(scope.environment_id())
    .bind(policy_id)
    .bind(version)
    .fetch_optional(scope.connection())
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row.try_get("spec")?)
}

/// One stored revision, without its document.
///
/// The list answers "what changed, when, by whom"; the document itself is read
/// on demand, because a hundred revisions of a large policy is a lot of JSON the
/// list does not need.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VersionRow {
    pub version: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_by: Option<String>,
    pub size_bytes: i64,
}

/// The revision history of one policy, newest first.
pub async fn list_versions(
    scope: &mut TenantScope<'_>,
    policy_id: Uuid,
) -> Result<Vec<VersionRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT v.version, v.created_at, u.email AS created_by,
                octet_length(v.spec::text)::bigint AS size_bytes
           FROM policy_versions v
           JOIN policies p ON p.id = v.policy_id
           LEFT JOIN users u ON u.id = v.created_by
          WHERE p.environment_id = $1 AND v.policy_id = $2
          ORDER BY v.version DESC
          LIMIT 200",
    )
    .bind(scope.environment_id())
    .bind(policy_id)
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(VersionRow {
            version: row.try_get("version")?,
            created_at: row.try_get("created_at")?,
            created_by: row.try_get("created_by")?,
            size_bytes: row.try_get("size_bytes")?,
        });
    }
    Ok(out)
}

/// Delete a policy.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM policies WHERE environment_id = $1 AND id = $2")
        .bind(scope.environment_id())
        .bind(id)
        .execute(scope.connection())
        .await?
        .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

fn row_to_rule(row: &sqlx::postgres::PgRow) -> Result<NetworkRule, StoreError> {
    let action_raw: String = row.try_get("action")?;
    let protocol_raw: String = row.try_get("protocol")?;
    let enforcement_raw: String = row.try_get("enforcement")?;
    let mut rule = NetworkRule {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        policy_id: row.try_get("policy_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        name: row.try_get("name")?,
        action: RuleAction::parse(&action_raw).unwrap_or(RuleAction::Allow),
        host: row.try_get("host")?,
        port: row.try_get("port")?,
        protocol: RuleProtocol::parse(&protocol_raw).unwrap_or(RuleProtocol::Rest),
        methods: row.try_get("methods")?,
        binaries: row.try_get("binaries")?,
    matchers: read_matchers(&row)?,
        enforcement: Enforcement::parse(&enforcement_raw).unwrap_or(Enforcement::Enforced),
        priority: row.try_get("priority")?,
        enabled: row.try_get("enabled")?,
        version: row.try_get("version")?,
    };
    crate::domain::policy::normalise_matchers(&mut rule);
    Ok(rule)
}

const RULE_COLUMNS: &str = "id, environment_id, policy_id, sandbox_id, name, action, \
     host, port, protocol, methods, binaries, matchers, enforcement, priority, enabled, version";

/// Read the matcher list of one row.
///
/// The column carries JSON and the domain carries a typed list; a malformed
/// value is a store error, not a rule that silently matches nothing. Shared
/// with `traffic.rs`, which reads the same rows for the effective-rules view.
pub(crate) fn read_matchers(row: &sqlx::postgres::PgRow) -> Result<Vec<Matcher>, StoreError> {
    let raw: serde_json::Value = row.try_get("matchers")?;
    serde_json::from_value(raw).map_err(|error| StoreError::UnexpectedData(error.to_string()))
}

/// All network rules in this environment.
pub async fn list_rules(scope: &mut TenantScope<'_>) -> Result<Vec<NetworkRule>, StoreError> {
    let sql = format!(
        "SELECT {RULE_COLUMNS} FROM network_rules
          WHERE environment_id = $1 ORDER BY priority, name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_rule(&row)?);
    }
    Ok(out)
}

/// Rules attached to one sandbox, for the detail drawer.
pub async fn rules_for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<NetworkRule>, StoreError> {
    let sql = format!(
        "SELECT {RULE_COLUMNS} FROM network_rules
          WHERE environment_id = $1 AND sandbox_id = $2 ORDER BY priority, name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_rule(&row)?);
    }
    Ok(out)
}

/// Insert a network rule.
///
/// The matchers column carries the authoritative conditions; the flat fields
/// are written from it so a reader that predates 0032 sees the same rule.
/// Deriving the flat host/port/protocol from the matchers is honest for every
/// form the console and the projection produce: one host, one port, one
/// protocol word.
pub async fn insert_rule(
    scope: &mut TenantScope<'_>,
    rule: &NetworkRule,
) -> Result<NetworkRule, StoreError> {
    let mut prepared = rule.clone();
    crate::domain::policy::normalise_matchers(&mut prepared);
    let flat = crate::domain::policy::flat_from_matchers(&prepared.matchers);
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO network_rules
                (environment_id, policy_id, sandbox_id, name, action, host, port,
                 protocol, methods, binaries, matchers, enforcement, priority, enabled)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            RETURNING {RULE_COLUMNS}
         )
         SELECT {RULE_COLUMNS} FROM inserted"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(prepared.policy_id)
        .bind(prepared.sandbox_id)
        .bind(&prepared.name)
        .bind(prepared.action.as_str())
        .bind(&flat.host)
        .bind(flat.port)
        .bind(flat.protocol.as_str())
        .bind(&flat.methods)
        .bind(&flat.binaries)
        .bind(serde_json::to_value(&prepared.matchers).unwrap_or(serde_json::Value::Array(Vec::new())))
        .bind(prepared.enforcement.as_str())
        .bind(prepared.priority)
        .bind(prepared.enabled)
        .fetch_one(scope.connection())
        .await?;
    row_to_rule(&row)
}

/// Delete a network rule.
pub async fn delete_rule(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let affected = sqlx::query("DELETE FROM network_rules WHERE environment_id = $1 AND id = $2")
        .bind(scope.environment_id())
        .bind(id)
        .execute(scope.connection())
        .await?
        .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// A stable hash of a compiled rule set.
///
/// "Deployed" is `nodes.policy_hash_reported == policies.compiled_hash`, so the
/// hash has to change when, and only when, enforcement changes. It is therefore
/// built over the fields a rule is enforced by — id, action, host, port,
/// protocol, methods, enforcement, priority, enabled, sandbox scope, version —
/// and not over presentation (a rename does not need a redeploy) or over
/// windows (they are time-bounded and read per decision, so hashing them would
/// make the badge flicker while nothing about enforcement changed).
///
/// The rules are sorted first, so the same set hashes the same whatever order
/// the query returned, and the fields are separated by a byte that cannot occur
/// in a host or a name.
pub fn compiled_hash(rules: &[NetworkRule]) -> String {
    use sha2::{Digest, Sha256};

    let mut ordered: Vec<&NetworkRule> = rules.iter().collect();
    ordered.sort_by_key(|rule| (rule.priority, rule.id));

    let mut canonical = String::new();
    for rule in ordered {
        canonical.push_str(&format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n",
            rule.id,
            rule.action.as_str(),
            rule.host.to_ascii_lowercase(),
            rule.port.map(|port| port.to_string()).unwrap_or_default(),
            rule.protocol.as_str(),
            rule.methods.join(","),
            rule.enforcement.as_str(),
            rule.priority,
            rule.enabled,
            rule.sandbox_id.map(|id| id.to_string()).unwrap_or_default(),
            rule.version,
        ));
    }

    let digest = Sha256::digest(canonical.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// A rule draft awaiting a decision.
pub struct DraftRecord {
    pub id: Uuid,
    pub source_kind: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub draft: serde_json::Value,
    pub diff: Option<serde_json::Value>,
    pub status: String,
}

/// All DNS rules in this environment.
pub async fn list_dns_rules(scope: &mut TenantScope<'_>) -> Result<Vec<DnsRule>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, environment_id, policy_id, sandbox_id, action, pattern,
                priority, enabled, version
           FROM dns_rules
          WHERE environment_id = $1
          ORDER BY priority, pattern",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_to_dns_rule).collect()
}

/// DNS rules attached to one sandbox, for the detail drawer.
pub async fn dns_rules_for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<DnsRule>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, environment_id, policy_id, sandbox_id, action, pattern,
                priority, enabled, version
           FROM dns_rules
          WHERE environment_id = $1 AND sandbox_id = $2
          ORDER BY priority, pattern",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_to_dns_rule).collect()
}

/// Insert a DNS rule.
pub async fn insert_dns_rule(
    scope: &mut TenantScope<'_>,
    rule: &DnsRule,
) -> Result<DnsRule, StoreError> {
    let row = sqlx::query(
        "INSERT INTO dns_rules
            (environment_id, policy_id, sandbox_id, action, pattern, priority, enabled)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, environment_id, policy_id, sandbox_id, action, pattern,
                   priority, enabled, version",
    )
    .bind(scope.environment_id())
    .bind(rule.policy_id)
    .bind(rule.sandbox_id)
    .bind(rule.action.as_str())
    .bind(rule.pattern.trim())
    .bind(rule.priority)
    .bind(rule.enabled)
    .fetch_one(scope.connection())
    .await?;
    row_to_dns_rule(&row)
}

/// Delete one DNS rule.
pub async fn delete_dns_rule(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<(), StoreError> {
    let affected =
        sqlx::query("DELETE FROM dns_rules WHERE environment_id = $1 AND id = $2")
            .bind(scope.environment_id())
            .bind(id)
            .execute(scope.connection())
            .await?
            .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

fn row_to_dns_rule(row: &sqlx::postgres::PgRow) -> Result<DnsRule, StoreError> {
    let action_raw: String = row.try_get("action")?;
    Ok(DnsRule {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        policy_id: row.try_get("policy_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        action: RuleAction::parse(&action_raw).unwrap_or(RuleAction::Deny),
        pattern: row.try_get("pattern")?,
        priority: row.try_get("priority")?,
        enabled: row.try_get("enabled")?,
        version: row.try_get("version")?,
    })
}

/// One sandbox's egress mode, when it has been set.
///
/// Absent means forward proxy, which is the default the proxy itself runs; a
/// row exists only when an operator has chosen something else.
pub async fn egress_mode_for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<EgressModeSetting>, StoreError> {
    let row = sqlx::query(
        "SELECT sandbox_id, mode, trust_ca_ref, enabled, version,
                changed_at, revert_at, revert_to
           FROM egress_modes
          WHERE environment_id = $1 AND sandbox_id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mode_raw: String = row.try_get("mode")?;
    Ok(Some(EgressModeSetting {
        sandbox_id: row.try_get("sandbox_id")?,
        mode: EgressMode::parse(&mode_raw).unwrap_or(EgressMode::ForwardProxy),
        trust_ca_ref: row.try_get("trust_ca_ref")?,
        enabled: row.try_get("enabled")?,
        version: row.try_get("version")?,
        changed_at: row.try_get("changed_at")?,
        revert_at: row.try_get("revert_at")?,
        revert_to: row.try_get("revert_to")?,
        needs_recreate: false,
    }))
}

/// Set one sandbox's egress mode.
///
/// The unique index makes this an upsert: a second write is the operator
/// changing their mind, not a conflict.
pub async fn set_egress_mode(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    mode: EgressMode,
    trust_ca_ref: Option<&str>,
    enabled: bool,
    revert_at: Option<chrono::DateTime<chrono::Utc>>,
    revert_to: Option<&str>,
) -> Result<EgressModeSetting, StoreError> {
    let row = sqlx::query(
        "INSERT INTO egress_modes
            (environment_id, sandbox_id, mode, trust_ca_ref, enabled, revert_at, revert_to)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (sandbox_id) DO UPDATE
            SET mode = EXCLUDED.mode,
                trust_ca_ref = EXCLUDED.trust_ca_ref,
                enabled = EXCLUDED.enabled,
                revert_at = EXCLUDED.revert_at,
                revert_to = EXCLUDED.revert_to,
                changed_at = now(),
                version = egress_modes.version + 1,
                updated_at = now()
         RETURNING sandbox_id, mode, trust_ca_ref, enabled, version,
                   changed_at, revert_at, revert_to",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(mode.as_str())
    .bind(trust_ca_ref)
    .bind(enabled)
    .bind(revert_at)
    .bind(revert_to)
    .fetch_one(scope.connection())
    .await?;
    let mode_raw: String = row.try_get("mode")?;
    Ok(EgressModeSetting {
        sandbox_id: row.try_get("sandbox_id")?,
        mode: EgressMode::parse(&mode_raw).unwrap_or(mode),
        trust_ca_ref: row.try_get("trust_ca_ref")?,
        enabled: row.try_get("enabled")?,
        version: row.try_get("version")?,
        changed_at: row.try_get("changed_at")?,
        revert_at: row.try_get("revert_at")?,
        revert_to: row.try_get("revert_to")?,
        needs_recreate: false,
    })
}

/// All policy templates of this environment, the preset library.
///
/// Templates are data (ADR 0020): a preset is a document, not code, so an
/// operator can edit one and the change needs no deploy.
pub async fn list_templates(
    scope: &mut TenantScope<'_>,
) -> Result<Vec<PolicyTemplate>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, environment_id, name, description, spec, version
           FROM policy_templates
          WHERE environment_id = $1
          ORDER BY name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_to_template).collect()
}

/// One template by id.
pub async fn get_template(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<PolicyTemplate, StoreError> {
    let row = sqlx::query(
        "SELECT id, environment_id, name, description, spec, version
           FROM policy_templates
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .fetch_optional(scope.connection())
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_template(&row)
}

fn row_to_template(row: &sqlx::postgres::PgRow) -> Result<PolicyTemplate, StoreError> {
    Ok(PolicyTemplate {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        spec: row.try_get("spec")?,
        version: row.try_get("version")?,
    })
}

/// Insert a template. Used by the seeds and by an operator saving their own.
pub async fn insert_template(
    scope: &mut TenantScope<'_>,
    name: &str,
    description: &str,
    spec: &serde_json::Value,
) -> Result<PolicyTemplate, StoreError> {
    let row = sqlx::query(
        "INSERT INTO policy_templates (environment_id, name, description, spec)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (environment_id, name) DO UPDATE
            SET description = EXCLUDED.description,
                spec = EXCLUDED.spec,
                version = policy_templates.version + 1,
                updated_at = now()
         RETURNING id, environment_id, name, description, spec, version",
    )
    .bind(scope.environment_id())
    .bind(name)
    .bind(description)
    .bind(spec)
    .fetch_one(scope.connection())
    .await?;
    row_to_template(&row)
}

/// The policy assigned to one sandbox, with what has been applied.
pub async fn assignment_for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<PolicyAssignment>, StoreError> {
    let row = sqlx::query(
        "SELECT sp.sandbox_id, sp.policy_id, sp.desired_version, sp.applied_version,
                sp.applied_at, p.name AS policy_name, p.scope AS policy_scope
           FROM sandbox_policies sp
           JOIN policies p ON p.id = sp.policy_id
          WHERE sp.sandbox_id = $1
          ORDER BY sp.precedence
          LIMIT 1",
    )
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else { return Ok(None) };
    let scope_raw: String = row.try_get("policy_scope")?;
    Ok(Some(PolicyAssignment {
        sandbox_id: row.try_get("sandbox_id")?,
        policy_id: row.try_get("policy_id")?,
        policy_name: row.try_get("policy_name")?,
        policy_scope: PolicyScope::parse(&scope_raw).unwrap_or(PolicyScope::Sandbox),
        desired_version: row.try_get("desired_version")?,
        applied_version: row.try_get("applied_version")?,
        applied_at: row.try_get("applied_at")?,
    }))
}

/// Assign a policy to a sandbox.
///
/// A sandbox carries one assignment, so the old row goes first and the new one
/// is inserted; a re-assignment is changing the assignment, not a conflict. The
/// answer carries the policy's name, because every caller shows it next to the
/// sandbox and a second lookup in each of them is where the empty names come
/// from.
pub async fn assign_policy(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    policy_id: Uuid,
) -> Result<PolicyAssignment, StoreError> {
    sqlx::query("DELETE FROM sandbox_policies WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .execute(scope.connection())
        .await?;
    // An id outside this environment matches no row, so nothing is inserted and
    // `fetch_one` refuses it. Reading the assignment back names the policy and
    // cannot disagree with what was written.
    sqlx::query(
        "INSERT INTO sandbox_policies (sandbox_id, policy_id, desired_version)
         SELECT $2, p.id, p.version FROM policies p
          WHERE p.id = $3 AND p.environment_id = $1
         RETURNING sandbox_id",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(policy_id)
    .fetch_one(scope.connection())
    .await?;

    assignment_for_sandbox(scope, sandbox_id)
        .await?
        .ok_or(StoreError::NotFound)
}

/// Remove a sandbox's policy assignment.
pub async fn unassign_policy(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<(), StoreError> {
    let affected =
        sqlx::query("DELETE FROM sandbox_policies WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .execute(scope.connection())
            .await?
            .rows_affected();
    let _ = affected;
    Ok(())
}

/// One policy in the environment that carries a name, for the seed the sandbox
/// creation path looks up.
pub async fn get_policy_by_name(
    scope: &mut TenantScope<'_>,
    name: &str,
) -> Result<Option<Policy>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {POLICY_COLUMNS} FROM policies
          WHERE environment_id = $1 AND name = $2 AND scope = 'sandbox' LIMIT 1"
    ))
    .bind(scope.environment_id())
    .bind(name)
    .fetch_optional(scope.connection())
    .await?;
    match row {
        Some(row) => Ok(Some(row_to_policy(&row)?)),
        None => Ok(None),
    }
}

/// Pending drafts, for the approval queue.
pub async fn list_pending_drafts(
    scope: &mut TenantScope<'_>,
) -> Result<Vec<DraftRecord>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, source_kind, target_type, target_id, draft, diff, status
           FROM rule_drafts
          WHERE environment_id = $1 AND status = 'pending'
          ORDER BY created_at DESC",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DraftRecord {
            id: row.try_get("id")?,
            source_kind: row.try_get("source_kind")?,
            target_type: row.try_get("target_type")?,
            target_id: row.try_get("target_id")?,
            draft: row.try_get("draft")?,
            diff: row.try_get("diff")?,
            status: row.try_get("status")?,
        });
    }
    Ok(out)
}

/// Create a draft from an event.
pub async fn insert_draft(
    scope: &mut TenantScope<'_>,
    source_event_id: Option<Uuid>,
    source_kind: &str,
    target_type: &str,
    target_id: Option<Uuid>,
    draft: &serde_json::Value,
    diff: Option<&serde_json::Value>,
    created_by: Option<Uuid>,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO rule_drafts
            (environment_id, source_event_id, source_kind, target_type, target_id,
             draft, diff, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(source_event_id)
    .bind(source_kind)
    .bind(target_type)
    .bind(target_id)
    .bind(draft)
    .bind(diff)
    .bind(created_by)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// Decide a draft. Returns the draft only if it was still pending, which is what
/// makes a double approval a conflict rather than a duplicate rule.
pub async fn decide_draft(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    approve: bool,
    reason: Option<&str>,
    decided_by: Option<Uuid>,
) -> Result<DraftRecord, StoreError> {
    let status = if approve { "approved" } else { "rejected" };
    let row = sqlx::query(
        "UPDATE rule_drafts
            SET status = $3, reason = $4, decided_by = $5, decided_at = now()
          WHERE environment_id = $1 AND id = $2 AND status = 'pending'
          RETURNING id, source_kind, target_type, target_id, draft, diff, status",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(status)
    .bind(reason)
    .bind(decided_by)
    .fetch_optional(scope.connection())
    .await?;

    let Some(row) = row else {
        // Either it does not exist or it was already decided; the caller shows a
        // different message for each, so the distinction is preserved.
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM rule_drafts WHERE environment_id = $1 AND id = $2",
        )
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
        return if exists.is_some() {
            Err(StoreError::VersionConflict)
        } else {
            Err(StoreError::NotFound)
        };
    };

    Ok(DraftRecord {
        id: row.try_get("id")?,
        source_kind: row.try_get("source_kind")?,
        target_type: row.try_get("target_type")?,
        target_id: row.try_get("target_id")?,
        draft: row.try_get("draft")?,
        diff: row.try_get("diff")?,
        status: row.try_get("status")?,
    })
}

/// One traffic window as the console lists it.
///
/// The console needs the sandbox's name and the mode/channel words, not the row
/// shape: a master switch without a label is a switch nobody can read.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowRow {
    pub id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: Option<String>,
    pub scope: String,
    pub mode: String,
    pub channel: String,
    pub reason: String,
    /// `None` means the window stays open until somebody closes it.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The longest window this environment allows; `None` means no limit.
pub async fn max_window_minutes_for_environment(
    scope: &mut TenantScope<'_>,
) -> Result<Option<i32>, StoreError> {
    let value: Option<Option<i32>> =
        sqlx::query_scalar("SELECT max_window_minutes FROM environments WHERE id = $1")
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await?;
    Ok(value.flatten())
}

/// The windows in force now, the open-ended ones first and then the one that
/// lives longest.
///
/// The order is the order of the decider (`egress::open_windows`): when two
/// windows cover the same request, the console must show the same one the
/// decision names, so both reads sort the same way. `NULL` is "until it is
/// closed", which outlives every deadline, hence `DESC NULLS FIRST`.
pub async fn list_windows(scope: &mut TenantScope<'_>) -> Result<Vec<WindowRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT w.id, w.sandbox_id, s.name AS sandbox_name, w.scope, w.mode, w.channel,
                w.reason, w.expires_at
           FROM traffic_windows w
           LEFT JOIN sandboxes s ON s.id = w.sandbox_id AND s.environment_id = w.environment_id
          WHERE w.environment_id = $1
            AND w.revoked_at IS NULL
            AND (w.expires_at IS NULL OR w.expires_at > now())
          ORDER BY w.expires_at DESC NULLS FIRST
          LIMIT 100",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(WindowRow {
            id: row.try_get("id")?,
            sandbox_id: row.try_get("sandbox_id")?,
            sandbox_name: row.try_get("sandbox_name")?,
            scope: row.try_get("scope")?,
            mode: row.try_get("mode")?,
            channel: row.try_get("channel")?,
            reason: row.try_get("reason")?,
            expires_at: row.try_get("expires_at")?,
        });
    }
    Ok(out)
}

/// Open a temporary traffic window.
pub async fn open_window(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
    scope_kind: &str,
    reason: &str,
    minutes: Option<i32>,
    mode: &str,
    channel: &str,
    created_by: Option<Uuid>,
) -> Result<(Uuid, Option<chrono::DateTime<chrono::Utc>>), StoreError> {
    let row = sqlx::query(
        "INSERT INTO traffic_windows
            (environment_id, sandbox_id, scope, reason, expires_at, mode, channel, created_by)
         VALUES ($1, $2, $3, $4,
                 CASE WHEN $5::int IS NULL THEN NULL ELSE now() + make_interval(mins => $5) END,
                 $6, $7, $8)
         RETURNING id, expires_at",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(scope_kind)
    .bind(reason)
    .bind(minutes)
    .bind(mode)
    .bind(channel)
    .bind(created_by)
    .fetch_one(scope.connection())
    .await?;
    Ok((row.try_get("id")?, row.try_get("expires_at")?))
}

/// Whether an unexpired window currently applies to a sandbox.
pub async fn window_active(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic_windows
          WHERE environment_id = $1
            AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > now())
            AND (sandbox_id = $2 OR sandbox_id IS NULL)",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_one(scope.connection())
    .await?;
    Ok(count > 0)
}

/// Revoke a window early.
pub async fn revoke_window(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "UPDATE traffic_windows SET revoked_at = now()
          WHERE environment_id = $1 AND id = $2 AND revoked_at IS NULL",
    )
    .bind(scope.environment_id())
    .bind(id)
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// Expire windows that have passed their deadline. Called by the schedule.
///
/// Uses the admin path because it sweeps every environment.
pub async fn expire_windows(pool: &sqlx::PgPool) -> Result<u64, StoreError> {
    let affected = sqlx::query(
        "UPDATE traffic_windows SET revoked_at = now()
          WHERE revoked_at IS NULL AND expires_at <= now()",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(host: &str, action: RuleAction, priority: i32) -> NetworkRule {
        NetworkRule {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: "test".into(),
            action,
            host: host.into(),
            port: None,
            protocol: RuleProtocol::Rest,
            methods: Vec::new(),
            binaries: Vec::new(),
            matchers: Vec::new(),
            enforcement: Enforcement::Enforced,
            priority,
            enabled: true,
            version: 1,
        }
    }

    #[test]
    fn the_same_rule_set_hashes_the_same_whatever_the_order() {
        let a = rule("example.com", RuleAction::Allow, 10);
        let b = rule("api.example.com", RuleAction::Deny, 5);
        assert_eq!(
            compiled_hash(&[a.clone(), b.clone()]),
            compiled_hash(&[b, a])
        );
    }

    #[test]
    fn every_change_to_a_rule_changes_the_hash() {
        let base = rule("example.com", RuleAction::Allow, 10);
        let original = compiled_hash(std::slice::from_ref(&base));

        let mut renamed = base.clone();
        renamed.name = "another name".into();
        assert_eq!(
            compiled_hash(&[renamed]),
            original,
            "a rename is not an enforcement change and must not force a redeploy"
        );

        let mut host = base.clone();
        host.host = "other.example.com".into();
        assert_ne!(compiled_hash(&[host]), original);

        let mut action = base.clone();
        action.action = RuleAction::Deny;
        assert_ne!(compiled_hash(&[action]), original);

        let mut port = base.clone();
        port.port = Some(443);
        assert_ne!(compiled_hash(&[port]), original);

        let mut priority = base.clone();
        priority.priority = 11;
        assert_ne!(compiled_hash(&[priority]), original);

        let mut disabled = base.clone();
        disabled.enabled = false;
        assert_ne!(compiled_hash(&[disabled]), original);

        let mut audit = base.clone();
        audit.enforcement = Enforcement::Audit;
        assert_ne!(compiled_hash(&[audit]), original);

        let mut methods = base.clone();
        methods.methods = vec!["GET".into()];
        assert_ne!(compiled_hash(&[methods]), original);

        let mut scoped = base.clone();
        scoped.sandbox_id = Some(Uuid::nil());
        assert_ne!(compiled_hash(&[scoped]), original);

        assert_ne!(
            compiled_hash(&[base.clone(), rule("a.b", RuleAction::Allow, 30)]),
            original,
            "adding a rule changes the set"
        );
    }

    #[test]
    fn the_hash_is_hex_and_long_enough_to_not_collide_by_accident() {
        let hash = compiled_hash(&[rule("example.com", RuleAction::Allow, 1)]);
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

