//! Global search: what a hit looks like and which kinds exist (contract C4).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a hit can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Sandbox,
    Node,
    Model,
    Policy,
    User,
    Site,
    DataCenter,
    Cluster,
    Job,
}

impl SearchKind {
    /// Every kind, in the order the console groups them.
    pub const ALL: [SearchKind; 9] = [
        Self::Sandbox,
        Self::Node,
        Self::Model,
        Self::Policy,
        Self::User,
        Self::Site,
        Self::DataCenter,
        Self::Cluster,
        Self::Job,
    ];

    /// The query-string spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Node => "node",
            Self::Model => "model",
            Self::Policy => "policy",
            Self::User => "user",
            Self::Site => "site",
            Self::DataCenter => "data_center",
            Self::Cluster => "cluster",
            Self::Job => "job",
        }
    }

    /// Parse the query-string spelling.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// One search result.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: Option<String>,
    pub environment_id: Option<uuid::Uuid>,
    /// Console path from contract C1.
    pub path: String,
    pub score: f64,
}

/// The console path (contract C1) of one hit.
pub fn console_path(kind: SearchKind, id: &str) -> String {
    match kind {
        SearchKind::Sandbox => format!("/sandboxes/{id}"),
        SearchKind::Node => format!("/node/{id}"),
        SearchKind::Model => format!("/models/{id}"),
        SearchKind::Policy => format!("/policies?id={id}"),
        SearchKind::User => format!("/users?id={id}"),
        SearchKind::Site => format!("/site/{id}"),
        SearchKind::DataCenter => format!("/dc/{id}"),
        SearchKind::Cluster => format!("/clusters/{id}"),
        SearchKind::Job => format!("/operations?id={id}"),
    }
}

/// How well `name` matches `q`: 1.0 exact, 0.9 prefix, 0.6 contains, 0.0 otherwise.
/// Case-insensitive; an empty query matches nothing.
pub fn score(name: &str, q: &str) -> f32 {
    let name = name.to_lowercase();
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        0.0
    } else if name == q {
        1.0
    } else if name.starts_with(&q) {
        0.9
    } else if name.contains(&q) {
        0.6
    } else {
        0.0
    }
}

/// The best of the name score and a 0.5 id-prefix score.
pub fn score_with_id(name: &str, id: &str, q: &str) -> f32 {
    let by_name = score(name, q);
    let q = q.trim().to_lowercase();
    if !q.is_empty() && id.to_lowercase().starts_with(&q) {
        by_name.max(0.5)
    } else {
        by_name
    }
}

/// The `ILIKE` pattern for `q`: wildcards and the escape character are escaped so the
/// text is matched literally (use with `ESCAPE '\'`).
pub fn like_pattern(q: &str) -> String {
    let escaped = q.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

/// The parsed query of `GET /search`.
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub q: String,
    pub kinds: Vec<SearchKind>,
    pub environment: Option<uuid::Uuid>,
    pub status: Option<String>,
    pub limit: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_string() {
        for kind in SearchKind::ALL {
            assert_eq!(SearchKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SearchKind::parse("nonsense"), None);
    }

    #[test]
    fn score_ranks_exact_over_prefix_over_contains() {
        assert_eq!(score("Research-01", "research-01"), 1.0);
        assert_eq!(score("research-01", "res"), 0.9);
        assert_eq!(score("my-research", "res"), 0.6);
        assert_eq!(score("alpha", "zz"), 0.0);
        assert_eq!(score("alpha", " "), 0.0);
    }

    #[test]
    fn an_id_prefix_scores_half() {
        assert_eq!(score_with_id("alpha", "ab12cd", "ab12"), 0.5);
        assert_eq!(score_with_id("ab12", "ab12cd", "ab12"), 1.0);
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern(" a%b_c\\ "), "%a\\%b\\_c\\\\%");
    }

    #[test]
    fn paths_follow_the_console_routes() {
        assert_eq!(console_path(SearchKind::Node, "x"), "/node/x");
        assert_eq!(console_path(SearchKind::Job, "x"), "/operations?id=x");
    }
}
