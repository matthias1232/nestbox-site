//! Which models a sandbox may use, granted to the environment, a user or one
//! sandbox.
//!
//! A grant names a provider profile (which holds the endpoint and the locator
//! for the key) and the name the agent uses. Inheritance is environment → user
//! (of the sandbox's owner) → sandbox, and the narrower scope wins (ADR 0056).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One grant row.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelGrant {
    /// Row id.
    pub id: Uuid,
    /// `environment`, `user` or `sandbox`.
    pub scope: String,
    /// The user for a user grant.
    pub user_id: Option<Uuid>,
    /// The sandbox for a sandbox grant.
    pub sandbox_id: Option<Uuid>,
    /// The name the agent uses for the model.
    pub model_name: String,
    /// The provider profile that serves it.
    pub provider_profile_id: Uuid,
    /// The model id at the provider, when it differs from `model_name`.
    pub upstream_model: Option<String>,
    /// True for the model agents start with.
    pub is_default: bool,
    /// Keep the model loaded even when no sandbox runs.
    pub keep_warm: bool,
    /// When it was granted.
    pub created_at: DateTime<Utc>,
}

/// The body of a new grant.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NewModelGrant {
    /// `environment`, `user` or `sandbox`.
    pub scope: String,
    /// The user for a user grant.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// The sandbox for a sandbox grant.
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    /// The name the agent uses.
    pub model_name: String,
    /// The provider profile that serves it.
    pub provider_profile_id: Uuid,
    /// The model id at the provider.
    #[serde(default)]
    pub upstream_model: Option<String>,
    /// Default model.
    #[serde(default)]
    pub is_default: bool,
    /// Keep warm.
    #[serde(default)]
    pub keep_warm: bool,
}

/// A model as one sandbox sees it after inheritance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct EffectiveModel {
    /// The name the agent uses.
    pub model_name: String,
    /// The provider profile.
    pub provider_profile_id: Uuid,
    /// The model id at the provider.
    pub upstream_model: String,
    /// Default model of the sandbox.
    pub is_default: bool,
    /// Keep warm.
    pub keep_warm: bool,
    /// Where it came from: `environment`, `user` or `sandbox`.
    pub source: String,
}

/// The rank of a scope: a narrower grant wins.
fn scope_rank(scope: &str) -> u8 {
    match scope {
        "sandbox" => 0,
        "user" => 1,
        _ => 2,
    }
}

/// Merge the grants that apply to one sandbox into the list it sees.
///
/// Same `model_name` from two scopes keeps the narrower one; exactly one model
/// is the default (the narrowest grant that asks for it, else the first name).
/// The list reads by name so that two lists built from the same rows cannot
/// order differently, which would reorder the console and the agent's model list
/// on every restart.
pub fn effective(grants: &[ModelGrant]) -> Vec<EffectiveModel> {
    let mut winners: Vec<&ModelGrant> = Vec::new();
    for grant in grants {
        match winners
            .iter()
            .position(|kept| kept.model_name == grant.model_name)
        {
            Some(index) => {
                if scope_rank(&grant.scope) < scope_rank(&winners[index].scope) {
                    winners[index] = grant;
                }
            }
            None => winners.push(grant),
        }
    }
    winners.sort_by(|left, right| left.model_name.cmp(&right.model_name));

    // Several scopes may ask for a default; the narrowest one decides. Ties keep
    // the first of the name-sorted list, so the answer is stable.
    // Several scopes may ask for a default; the narrowest one decides. Within one
    // scope the newest wins, because "I just marked this one" is what an operator
    // means, and a name ordering would silently pick the older grant.
    let default_name = winners
        .iter()
        .filter(|grant| grant.is_default)
        .min_by_key(|grant| (scope_rank(&grant.scope), std::cmp::Reverse(grant.created_at)))
        .map(|grant| grant.model_name.clone())
        .or_else(|| winners.first().map(|grant| grant.model_name.clone()));

    winners
        .iter()
        .map(|grant| EffectiveModel {
            model_name: grant.model_name.clone(),
            provider_profile_id: grant.provider_profile_id,
            upstream_model: grant
                .upstream_model
                .clone()
                .unwrap_or_else(|| grant.model_name.clone()),
            is_default: default_name.as_deref() == Some(grant.model_name.as_str()),
            keep_warm: grant.keep_warm,
            source: grant.scope.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(scope: &str, name: &str, profile: u128, is_default: bool) -> ModelGrant {
        ModelGrant {
            id: Uuid::from_u128(profile),
            scope: scope.to_string(),
            user_id: None,
            sandbox_id: None,
            model_name: name.to_string(),
            provider_profile_id: Uuid::from_u128(profile),
            upstream_model: None,
            is_default,
            keep_warm: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn a_narrower_grant_wins_and_one_model_is_default() {
        let grants = vec![
            grant("environment", "a", 1, false),
            grant("user", "a", 2, false),
            grant("sandbox", "b", 3, true),
        ];
        let models = effective(&grants);
        assert_eq!(models.len(), 2);
        let a = models.iter().find(|m| m.model_name == "a").expect("a");
        assert_eq!(a.provider_profile_id, Uuid::from_u128(2));
        assert_eq!(a.source, "user");
        assert!(!a.is_default);
        let b = models.iter().find(|m| m.model_name == "b").expect("b");
        assert!(b.is_default);
    }

    #[test]
    fn without_a_default_the_first_name_wins() {
        let models = effective(&[grant("environment", "z", 1, false), grant("environment", "a", 2, false)]);
        assert_eq!(models[0].model_name, "a");
        assert!(models[0].is_default);
        assert!(!models[1].is_default);
    }

    #[test]
    fn the_upstream_name_defaults_to_the_model_name() {
        let mut row = grant("sandbox", "fast", 1, false);
        row.upstream_model = Some("qwen3:1.7b".to_string());
        let models = effective(&[row]);
        assert_eq!(models[0].upstream_model, "qwen3:1.7b");
    }

    #[test]
    fn the_list_reads_by_name_whatever_scope_a_model_came_from() {
        let models = effective(&[
            grant("sandbox", "zeta", 1, false),
            grant("environment", "alpha", 2, false),
            grant("user", "kilo", 3, false),
        ]);
        let names: Vec<&str> = models.iter().map(|m| m.model_name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "kilo", "zeta"]);
    }

    #[test]
    fn two_defaults_make_the_narrower_scope_the_default() {
        let models = effective(&[
            grant("environment", "a", 1, true),
            grant("sandbox", "b", 2, true),
            grant("user", "c", 3, true),
        ]);
        assert!(!models[0].is_default);
        assert!(models[1].is_default, "the sandbox grant is the narrowest");
        assert!(!models[2].is_default);
    }

    #[test]
    fn no_grant_is_no_model() {
        assert!(effective(&[]).is_empty());
    }
}
