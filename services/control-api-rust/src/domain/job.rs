//! Jobs and schedules.
//!
//! A job is one-shot work with a target set, progress, and partial failures. A
//! schedule is a recurring definition that creates jobs. They are separate
//! because "retry this bulk action" and "run the sync every 20 minutes" are
//! different questions, and one object cannot answer both cleanly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Where a job is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    /// Finished with at least one failed target. Distinguished from `Failed`
    /// because the operator needs to see which targets survived.
    Partial,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "partial" => Self::Partial,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// Whether the job has stopped moving.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Failed | Self::Cancelled
        )
    }
}

/// A job as the API returns it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub kind: String,
    pub status: JobStatus,
    pub total: i32,
    pub completed: i32,
    pub failed: i32,
    pub payload: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub schedule_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Job {
    /// Percentage complete, for a progress bar.
    ///
    /// A zero-target job reports 100 rather than dividing by zero: it is
    /// trivially complete, not undefined.
    pub fn percent(&self) -> i32 {
        if self.total <= 0 {
            return 100;
        }
        let done = self.completed + self.failed;
        ((done as f64 / self.total as f64) * 100.0).round() as i32
    }

    /// Whether the job can be retried.
    ///
    /// Retry means "create a new job for the failed targets", not "resume this
    /// record": keeping the original as history is what makes the failure
    /// auditable.
    pub fn is_retryable(&self) -> bool {
        self.failed > 0 && self.status.is_terminal()
    }
}

/// One target of a job, so partial failure is attributable.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobTarget {
    pub job_id: Uuid,
    pub subject_id: Uuid,
    pub state: TargetState,
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

impl TargetState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }
}

/// A recurring definition that creates jobs.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Schedule {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub kind: String,
    /// Five-field cron, evaluated in the environment's timezone.
    pub cron: String,
    pub payload: serde_json::Value,
    pub enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The job kinds the system creates. A closed set so a typo cannot create an
/// orphaned job nobody handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    SandboxBulkAction,
    SandboxCreate,
    SandboxDelete,
    PolicyRollout,
    GatewayConfigSync,
    ModelInstall,
    ModelUpdate,
    ModelUninstall,
    CertificateRenew,
    ProviderRefresh,
    TelemetryPartitionMaintenance,
    MetricsPartitionMaintenance,
    MetricsRollup,
    EgressPartitionMaintenance,
    PolicyReconciliation,
    CertificateExpiryCheck,
    AddressPoolReconcile,
    ModelUpdateCheck,
    SessionCleanup,
    CatalogSync,
    RecipeBuild,
    /// One model on many nodes (contract C6).
    ModelBulkInstall,
    /// Move or stop every sandbox of a node (contract C8).
    NodeDrain,
    /// Provision or join a node, optionally with a model (contract C8).
    NodeQuickDeploy,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SandboxBulkAction => "sandbox_bulk_action",
            Self::SandboxCreate => "sandbox_create",
            Self::SandboxDelete => "sandbox_delete",
            Self::PolicyRollout => "policy_rollout",
            Self::GatewayConfigSync => "gateway_config_sync",
            Self::ModelInstall => "model_install",
            Self::ModelUpdate => "model_update",
            Self::ModelUninstall => "model_uninstall",
            Self::CertificateRenew => "certificate_renew",
            Self::ProviderRefresh => "provider_refresh",
            Self::TelemetryPartitionMaintenance => "telemetry_partition_maintenance",
            Self::MetricsPartitionMaintenance => "metrics_partition_maintenance",
            Self::MetricsRollup => "metrics_rollup",
            Self::EgressPartitionMaintenance => "egress_partition_maintenance",
            Self::PolicyReconciliation => "policy_reconciliation",
            Self::CertificateExpiryCheck => "certificate_expiry_check",
            Self::AddressPoolReconcile => "address_pool_reconcile",
            Self::ModelUpdateCheck => "model_update_check",
            Self::SessionCleanup => "session_cleanup",
            Self::CatalogSync => "catalog_sync",
            Self::RecipeBuild => "recipe_build",
            Self::ModelBulkInstall => "model_bulk_install",
            Self::NodeDrain => "node_drain",
            Self::NodeQuickDeploy => "node_quick_deploy",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(status: JobStatus, total: i32, completed: i32, failed: i32) -> Job {
        Job {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            kind: JobKind::SandboxBulkAction.as_str().into(),
            status,
            total,
            completed,
            failed,
            payload: serde_json::json!({}),
            result: None,
            error: None,
            schedule_id: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn the_control_center_kinds_have_their_contract_names() {
        assert_eq!(JobKind::ModelBulkInstall.as_str(), "model_bulk_install");
        assert_eq!(JobKind::NodeDrain.as_str(), "node_drain");
        assert_eq!(JobKind::NodeQuickDeploy.as_str(), "node_quick_deploy");
    }

    #[test]
    fn percent_is_a_share_of_all_targets_including_failures() {
        assert_eq!(job(JobStatus::Running, 10, 5, 0).percent(), 50);
        assert_eq!(job(JobStatus::Running, 10, 5, 5).percent(), 100);
    }

    #[test]
    fn a_zero_target_job_is_complete_not_undefined() {
        assert_eq!(job(JobStatus::Succeeded, 0, 0, 0).percent(), 100);
    }

    #[test]
    fn a_partial_failure_is_retryable_and_says_why() {
        let j = job(JobStatus::Partial, 10, 8, 2);
        assert!(j.status.is_terminal());
        assert!(j.is_retryable());
        assert!(!job(JobStatus::Succeeded, 10, 10, 0).is_retryable());
    }
}
