//! Who is connected to a sandbox right now: terminals, SSH and desktop viewers.
//!
//! Presence is read from `sandbox_sessions`, the table that already records
//! who opened which terminal (ADR 0023). Sharing a session means two rows with
//! the same tab; a desktop viewer is a row of kind `desktop` (ADR 0055).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One open session of one person (or one SSH client) in one sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PresenceEntry {
    /// The `sandbox_sessions` row.
    pub session_id: Uuid,
    /// The user, when known (SSH logins have none).
    pub user_id: Option<Uuid>,
    /// The user's display name, when known.
    pub user_name: Option<String>,
    /// `terminal`, `ssh` or `desktop`.
    pub kind: String,
    /// The terminal tab (tmux session `nestbox-<tab>`), when any.
    pub tab_id: Option<String>,
    /// True for a desktop viewer that cannot type or click.
    pub view_only: bool,
    /// When the session opened.
    pub opened_at: DateTime<Utc>,
}
