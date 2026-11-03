//! Policy preview: the compiled OpenShell document and recreate warnings (contract C11).
//!
//! Nothing here writes. A preview validates a spec, compiles it the way the save path and the
//! OpenShell sync do, and says which sections need a recreate (ADR 0008).

use chrono::Utc;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::openshell_policy::{self, Compiled};
use crate::domain::policy::{self, NetworkRule, Policy, PolicyError, PolicyScope};
use crate::error::ApiError;
use crate::store::postgres::policies as policy_store;
use crate::store::postgres::sandboxes as sandbox_store;
use crate::store::TenantScope;

/// Validate and compile a spec that is not saved yet (contract C11).
///
/// `body` is `{ "spec": {...}, "sandbox_id": "uuid" | null }`. With a sandbox the answer also
/// carries a unified diff against the document that sandbox runs now.
pub async fn preview(
    ctx: &ServiceContext,
    principal: &Principal,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    let spec = body
        .get("spec")
        .cloned()
        .ok_or_else(|| ApiError::validation("The request needs a spec."))?;
    let sandbox_id = match body.get("sandbox_id") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(|text| Uuid::parse_str(text).ok())
                .ok_or_else(|| ApiError::validation("sandbox_id must be a UUID."))?,
        ),
    };
    preview_spec(ctx, principal, spec, sandbox_id, None).await
}

/// The same shape for a stored policy.
pub async fn compiled(
    ctx: &ServiceContext,
    principal: &Principal,
    policy_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let stored = policy_store::get(&mut scope, policy_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    preview_spec(ctx, principal, stored.spec.clone(), None, Some(&stored)).await
}

/// Shared body of both entry points.
async fn preview_spec(
    ctx: &ServiceContext,
    principal: &Principal,
    spec: Value,
    sandbox_id: Option<Uuid>,
    stored: Option<&Policy>,
) -> Result<Value, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if principal.can(Permission::PolicyRead) {
        // allowed
    } else if principal.can(Permission::SandboxPolicy) && sandbox_id.is_some() {
        // A sandbox-scoped caller sees the preview only for a sandbox they can see; under
        // row-level security another user's sandbox is a plain not-found.
        if let Some(id) = sandbox_id {
            sandbox_store::get(&mut scope, id).await.map_err(ApiError::from)?;
        }
    } else {
        return Err(ApiError::forbidden(Permission::PolicyRead.code()));
    }

    let errors = match policy::validate_document(&spec) {
        Ok(_) => Vec::new(),
        Err(error) => vec![error_entry(&error)],
    };
    let valid = errors.is_empty();
    let (hot, cold) = policy::hot_and_cold(&spec);

    let (compiled, diff) = if !valid {
        (empty_compiled(), None)
    } else if let Some(id) = sandbox_id {
        let current_rules = super::policy_service::effective_rules(&mut scope, id, principal.environment_id).await?;
        let governing = governing_policy(&mut scope, id).await?;
        let proposed_rules = replace_policy_rules(&current_rules, governing.as_ref(), &spec);
        let current = openshell_policy::compile(&current_rules);
        let proposed = openshell_policy::compile(&proposed_rules);
        let diff = unified_diff(&openshell_policy::to_yaml(&current), &openshell_policy::to_yaml(&proposed));
        (proposed, Some(diff))
    } else {
        let policy = stored.cloned().unwrap_or_else(|| draft_policy(principal, &spec));
        let rules = policy::rules_from_policy(&Policy { spec: spec.clone(), ..policy });
        (openshell_policy::compile(&rules), None)
    };
    scope.commit().await.map_err(ApiError::from)?;

    let document = with_cold_sections(compiled.document.clone(), &spec);
    let yaml = if valid {
        openshell_policy::to_yaml(&Compiled { document, skipped: Vec::new() })
    } else {
        String::new()
    };
    let skipped: Vec<Value> = compiled
        .skipped
        .iter()
        .map(|(rule, why)| json!({ "rule": rule, "why": why }))
        .collect();
    Ok(json!({
        "valid": valid,
        "errors": errors,
        "compiled_yaml": yaml,
        "skipped": skipped,
        "needs_recreate": cold,
        "hot": hot,
        "diff": diff,
    }))
}

/// A policy value that only carries the spec, for a draft nobody has saved.
fn draft_policy(principal: &Principal, spec: &Value) -> Policy {
    Policy {
        id: Uuid::nil(),
        environment_id: principal.environment_id,
        name: "draft".to_string(),
        scope: PolicyScope::Sandbox,
        spec: spec.clone(),
        version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

/// The policy that governs a sandbox now: the global override, else its assignment.
async fn governing_policy(scope: &mut TenantScope<'_>, sandbox_id: Uuid) -> Result<Option<Policy>, ApiError> {
    if let Some(global) = policy_store::global(scope).await.map_err(ApiError::from)? {
        return Ok(Some(global));
    }
    match policy_store::assignment_for_sandbox(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        Some(assignment) => Ok(Some(
            policy_store::get(scope, assignment.policy_id).await.map_err(ApiError::from)?,
        )),
        None => Ok(None),
    }
}

/// The sandbox's rules with those that came from its governing policy swapped for the ones
/// the new spec yields. Without a governing policy the new rules are simply added.
fn replace_policy_rules(current: &[NetworkRule], governing: Option<&Policy>, spec: &Value) -> Vec<NetworkRule> {
    let mut rules: Vec<NetworkRule> = match governing {
        Some(policy) => current
            .iter()
            .filter(|rule| !(rule.id.is_nil() && rule.policy_id == Some(policy.id)))
            .cloned()
            .collect(),
        None => current.to_vec(),
    };
    let base = governing.cloned().unwrap_or_else(|| Policy {
        id: Uuid::nil(),
        environment_id: current.first().map(|rule| rule.environment_id).unwrap_or_else(Uuid::nil),
        name: "draft".to_string(),
        scope: PolicyScope::Sandbox,
        spec: Value::Null,
        version: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    rules.extend(policy::rules_from_policy(&Policy { spec: spec.clone(), ..base }));
    rules.sort_by_key(|rule| rule.priority);
    rules
}

fn empty_compiled() -> Compiled {
    Compiled { document: json!({}), skipped: Vec::new() }
}

/// The compiler writes only `network_policies`; the document the sandbox starts with also
/// carries the cold sections of the spec, so the preview shows them too.
fn with_cold_sections(mut document: Value, spec: &Value) -> Value {
    if let (Some(target), Some(source)) = (document.as_object_mut(), spec.as_object()) {
        for name in ["filesystem_policy", "landlock", "process"] {
            if let Some(value) = source.get(name) {
                target.insert(name.to_string(), value.clone());
            }
        }
    }
    document
}

/// Map a validation error to the `{ path, message }` entry of the contract.
fn error_entry(error: &PolicyError) -> Value {
    let path = match error {
        PolicyError::Section { section, .. } | PolicyError::SectionNotAnObject { section } => section.clone(),
        PolicyError::MatcherValue { kind, .. } => kind.clone(),
        PolicyError::NotAnObject | PolicyError::Empty => String::new(),
    };
    json!({ "path": path, "message": error.to_string() })
}

/// A unified diff of two texts, or an empty string when they are equal (whole-file hunk).
fn unified_diff(before: &str, after: &str) -> String {
    if before == after {
        return String::new();
    }
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    // Longest common subsequence table; documents are small.
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut out = format!("--- current\n+++ proposed\n@@ -1,{} +1,{} @@\n", a.len(), b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            out.push_str(&format!(" {}\n", a[i]));
            i += 1;
            j += 1;
        } else if j < b.len() && (i >= a.len() || table[i][j + 1] >= table[i + 1][j]) {
            out.push_str(&format!("+{}\n", b[j]));
            j += 1;
        } else {
            out.push_str(&format!("-{}\n", a[i]));
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_diff() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n"), "");
    }

    #[test]
    fn a_changed_line_is_one_removal_and_one_addition() {
        let diff = unified_diff("a\nb\nc\n", "a\nx\nc\n");
        assert!(diff.contains("\n-b\n"));
        assert!(diff.contains("\n+x\n"));
        assert!(diff.contains("\n a\n"));
    }

    #[test]
    fn a_section_error_names_its_path() {
        let entry = error_entry(&PolicyError::Section { section: "landlock".into(), why: "bad".into() });
        assert_eq!(entry["path"], "landlock");
    }

    #[test]
    fn cold_sections_are_added_to_the_compiled_document() {
        let spec = json!({ "filesystem_policy": { "read_only": ["/etc"] }, "network_policies": {} });
        let merged = with_cold_sections(json!({ "version": 1 }), &spec);
        assert_eq!(merged["filesystem_policy"]["read_only"][0], "/etc");
    }
}
