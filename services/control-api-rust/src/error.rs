//! Errors and the published error envelope.
//!
//! Every failure a client sees has the same shape (ADR 0015):
//!
//! ```json
//! {"error": {"code": "sandbox.not_found", "message": "...", "details": {}, "request_id": "..."}}
//! ```
//!
//! Two rules protect more than they cost:
//!
//! * `code` is stable and untranslated. The UI translates the code; it never
//!   translates the message.
//! * Internal detail never reaches the client. A database error, a path, or a
//!   stack trace in a response is a finding, so that detail goes to the log
//!   under the same `request_id`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// A stable, client-branchable error code.
///
/// Enumerated as constants rather than free-form strings so a typo cannot
/// create a code no client handles and no test covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(transparent)]
pub struct ErrorCode(pub &'static str);

impl ErrorCode {
    pub const UNAUTHENTICATED: Self = Self("auth.unauthenticated");
    pub const FORBIDDEN: Self = Self("auth.forbidden");
    pub const CSRF_REQUIRED: Self = Self("auth.csrf_required");
    pub const SESSION_EXPIRED: Self = Self("auth.session_expired");

    pub const SANDBOX_NOT_FOUND: Self = Self("sandbox.not_found");
    pub const SANDBOX_NAME_TAKEN: Self = Self("sandbox.name_taken");
    pub const SANDBOX_INVALID_STATE: Self = Self("sandbox.invalid_state");
    pub const SANDBOX_VERSION_CONFLICT: Self = Self("sandbox.version_conflict");
    /// A terminal was refused because the sandbox or the user already holds the
    /// maximum. The console branches on it to say which limit was hit.
    pub const SANDBOX_TERMINAL_LIMIT: Self = Self("sandbox.terminal_limit");
    /// A WebSocket handshake arrived from an origin that is not allowed. A
    /// WebSocket is not covered by the same-origin policy, so this check is the
    /// only thing standing between a logged-in operator and a page they visit.
    pub const SANDBOX_TERMINAL_ORIGIN: Self = Self("sandbox.terminal_origin_refused");
    /// A desktop-only call reached a sandbox whose image has no desktop pack
    /// (no `nestbox-display`, no X server). The caller's next step is another
    /// sandbox, not a retry.
    pub const SANDBOX_NO_DESKTOP: Self = Self("sandbox.no_desktop");
    /// The display layout was changed by somebody else since the caller read it;
    /// `details` carries the layout that is on the screen now.
    pub const DISPLAY_CHANGED: Self = Self("display.changed");

    pub const POLICY_NOT_FOUND: Self = Self("policy.not_found");
    pub const POLICY_INVALID: Self = Self("policy.invalid");
    pub const POLICY_NEEDS_RECREATE: Self = Self("policy.requires_recreate");
    pub const POLICY_GLOBAL_ACTIVE: Self = Self("policy.global_active");

    pub const GATEWAY_NOT_FOUND: Self = Self("gateway.not_found");
    pub const GATEWAY_NOT_READY: Self = Self("gateway.not_ready");

    pub const MODEL_NOT_FOUND: Self = Self("model.not_found");
    pub const MODEL_NOT_INSTALLABLE: Self = Self("model.not_installable");
    pub const MODEL_ALREADY_INSTALLED: Self = Self("model.already_installed");
    pub const MODEL_INSTALL_NOT_FOUND: Self = Self("model.install_not_found");
    /// The catalogue row has no artefact this platform can fetch and serve.
    pub const MODEL_NOT_SERVABLE: Self = Self("model.not_servable");
    /// The node said no while fetching or starting the server.
    pub const MODEL_SERVE_FAILED: Self = Self("model.serve_failed");
    /// The install is between the reservation and the answer; wait or let it fail.
    pub const MODEL_INSTALL_IN_PROGRESS: Self = Self("model.install_in_progress");

    /// A named site is not part of the caller's data center.
    pub const SITE_NOT_FOUND: Self = Self("site.not_found");
    pub const NODE_NOT_FOUND: Self = Self("node.not_found");
    /// A node of that name is already registered on this installation. Its own code, so a
    /// client does not read it as a sandbox name clash.
    pub const NODE_NAME_TAKEN: Self = Self("node.name_taken");
    pub const CUDAX_COMPONENT_NOT_FOUND: Self = Self("cudax.component_not_found");
    pub const CUDAX_INSTALL_NOT_FOUND: Self = Self("cudax.install_not_found");
    pub const COMPONENT_NOT_FOUND: Self = Self("component.not_found");
    pub const NODE_RUNTIME_UNAVAILABLE: Self = Self("node.runtime_unavailable");
    pub const NODE_INSUFFICIENT_RESOURCES: Self = Self("node.insufficient_resources");

    pub const RECIPE_NOT_FOUND: Self = Self("recipe.not_found");
    pub const RECIPE_NOT_BUILT: Self = Self("recipe.not_built");
    pub const RECIPE_INVALID: Self = Self("recipe.invalid");
    pub const BASELINE_NOT_FOUND: Self = Self("baseline.not_found");
    pub const PACK_NOT_FOUND: Self = Self("pack.not_found");
    pub const PACK_INCOMPATIBLE: Self = Self("pack.incompatible");

    pub const ROOT_NOT_FOUND: Self = Self("root.not_found");
    pub const ROOT_IN_USE: Self = Self("root.in_use");
    pub const MOUNT_CONFLICT: Self = Self("mount.conflict");
    pub const MOUNT_NOT_FOUND: Self = Self("mount.not_found");
    pub const AGENT_NOT_FOUND: Self = Self("agent.not_found");
    pub const AGENT_EXISTS: Self = Self("agent.exists");

    pub const JOB_NOT_FOUND: Self = Self("job.not_found");
    pub const SCHEDULE_NOT_FOUND: Self = Self("schedule.not_found");

    pub const RULE_NOT_FOUND: Self = Self("rule.not_found");
    pub const DRAFT_NOT_FOUND: Self = Self("draft.not_found");
    pub const DRAFT_NOT_PENDING: Self = Self("draft.not_pending");

    pub const SECRET_UNRESOLVABLE: Self = Self("secret.unresolvable");
    pub const PROVIDER_NOT_FOUND: Self = Self("provider.not_found");
    pub const PROVIDER_CREDENTIAL_MISSING: Self = Self("provider.credential_missing");
    pub const PROVIDER_IN_USE: Self = Self("provider.in_use");
    pub const PROVIDER_PENDING: Self = Self("provider.pending");

    // Access: the SSH key and the published ports (ADR 0024).
    pub const SSH_KEY_NOT_FOUND: Self = Self("key.not_found");
    /// The private key was already downloaded once. Rotation is the way back.
    pub const SSH_KEY_ALREADY_DOWNLOADED: Self = Self("key.already_downloaded");
    pub const SSH_KEY_UNREADABLE: Self = Self("key.unreadable");
    pub const PORT_POOL_EXHAUSTED: Self = Self("port.pool_exhausted");
    pub const FORWARD_NOT_FOUND: Self = Self("forward.not_found");

    pub const VALIDATION_FAILED: Self = Self("request.validation_failed");
    pub const PAYLOAD_TOO_LARGE: Self = Self("request.payload_too_large");
    pub const RATE_LIMITED: Self = Self("request.rate_limited");
    pub const NOT_FOUND: Self = Self("request.not_found");
    pub const CONFLICT: Self = Self("request.conflict");
    pub const INTERNAL: Self = Self("server.internal");
    /// The route is registered and documented but its package has not landed.
    pub const NOT_IMPLEMENTED: Self = Self("server.not_implemented");
}

/// The error a handler returns.
///
/// Carries a code, a developer-facing message, and optional non-secret detail.
/// `request_id` is attached by the layer that has it, so every error in a
/// response shares the id in the response headers.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ErrorCode,
    /// Developer-facing English. The UI translates the code, not this string.
    pub message: String,
    /// Non-secret additional context, safe to return.
    pub details: serde_json::Value,
    /// Internal cause. Logged, never serialised.
    pub internal: Option<String>,
}

impl ApiError {
    /// Build an error from its parts.
    pub fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: serde_json::Value::Null,
            internal: None,
        }
    }

    /// Attach context the client may see. Must never contain a secret.
    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    /// Attach the internal cause. It is logged, not returned.
    pub fn with_internal(mut self, internal: impl Into<String>) -> Self {
        self.internal = Some(internal.into());
        self
    }

    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::UNAUTHENTICATED, message)
    }

    pub fn forbidden(permission: &str) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::FORBIDDEN,
            format!("This action requires the '{permission}' permission."),
        )
        .with_details(serde_json::json!({ "required_permission": permission }))
    }

    /// A request refused by a rule rather than by a missing role.
    ///
    /// Separate from `forbidden(permission)` because the message names the
    /// rule ("not attached", "not on the allow-list") and a client that sees
    /// `required_permission` would look for a grant that does not exist.
    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN, message)
    }

    pub fn not_found(code: ErrorCode, what: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, what)
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::VALIDATION_FAILED,
            message,
        )
    }

    pub fn conflict(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    /// A request that is well-formed but exceeds a documented limit.
    ///
    /// Separate from `conflict` because the client's next step differs: a 429
    /// says "close something and retry", a 409 says "the state is wrong".
    pub fn too_many(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, code, message)
    }

    /// A request that is well-formed but cannot be satisfied with the given
    /// code: a node without the resources, an incompatible pack.
    pub fn unprocessable(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    /// The resource existed and no longer can be handed out: the one-time key
    /// download after the first call.
    pub fn gone(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(StatusCode::GONE, code, message)
    }

    /// A dependency the request needs is not available.
    pub fn unavailable(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
    }

    /// A documented route whose implementation is a pending work package.
    ///
    /// Registered up front so the contract test "every route is documented and
    /// every documented route exists" holds while packages land in parallel.
    pub fn not_implemented(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            ErrorCode::NOT_IMPLEMENTED,
            format!("{what} is not implemented yet."),
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::INTERNAL,
            message,
        )
    }
}

/// The wire shape. One struct, so every error path produces the same envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub details: serde_json::Value,
    /// Filled in by `IntoResponse` from the request extension when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // The internal cause is logged here and nowhere else, which is what
        // makes "the detail is in the log under the request id" true.
        if let Some(internal) = &self.internal {
            tracing::error!(
                code = self.code.0,
                status = self.status.as_u16(),
                internal = %crate::security::redaction::redact(internal),
                "request failed"
            );
        }

        let body = ErrorEnvelope {
            error: ErrorBody {
                code: self.code.0,
                message: self.message,
                details: self.details,
                request_id: None,
            },
        };

        (self.status, Json(body)).into_response()
    }
}

/// Store and service failures, mapped once.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(sqlx::Error),
    #[error("row not found")]
    NotFound,
    /// A write raced another write. Becomes a 409, not a 500: it is a
    /// legitimate outcome the client can retry.
    #[error("version conflict")]
    VersionConflict,
    /// A unique constraint fired. The caller decides the code.
    #[error("unique violation: {0}")]
    UniqueViolation(String),
    /// RLS refused the write. Should be unreachable from the application, which
    /// is why it is logged as an error rather than mapped to a friendly message.
    #[error("tenant scope violation")]
    TenantScopeViolation,
    /// A row holds a value the domain cannot represent. A schema and an
    /// application that disagree is a bug, so it is a 500 rather than a guess.
    #[error("unexpected data: {0}")]
    UnexpectedData(String),
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::NotFound => Self::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NOT_FOUND,
                "The requested resource does not exist.",
            ),
            StoreError::VersionConflict => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::CONFLICT,
                "The resource changed since it was read. Reload and try again.",
            ),
            StoreError::UniqueViolation(_) => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::CONFLICT,
                "A resource with that name already exists.",
            ),
            StoreError::TenantScopeViolation => Self::new(
                StatusCode::FORBIDDEN,
                ErrorCode::FORBIDDEN,
                "The resource belongs to another environment.",
            ),
            StoreError::UnexpectedData(detail) => Self::internal(
                "The request could not be completed.",
            )
            .with_internal(detail),
            StoreError::Database(err) => {
                Self::internal("The request could not be completed.").with_internal(err.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_namespaced_and_stable() {
        for code in [
            ErrorCode::SANDBOX_NOT_FOUND,
            ErrorCode::FORBIDDEN,
            ErrorCode::INTERNAL,
        ] {
            assert!(code.0.contains('.'), "{:?} not namespaced", code);
        }
    }

    #[test]
    fn a_validation_error_is_422_not_400() {
        // 400 means malformed; 422 means well-formed but unacceptable. The
        // distinction is what lets a client decide whether retrying helps.
        assert_eq!(
            ApiError::validation("bad").status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn a_version_conflict_is_409_so_a_client_can_retry() {
        let error: ApiError = StoreError::VersionConflict.into();
        assert_eq!(error.status, StatusCode::CONFLICT);
    }

    #[test]
    fn a_database_error_does_not_leak_its_message_to_the_client() {
        let error: ApiError = StoreError::Database(sqlx::Error::RowNotFound).into();
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!error.message.contains("RowNotFound"));
        assert!(error.internal.is_some());
    }

    #[test]
    fn a_forbidden_error_names_the_missing_permission() {
        let error = ApiError::forbidden("sandbox.exec");
        assert_eq!(error.status, StatusCode::FORBIDDEN);
        assert!(error.details.to_string().contains("sandbox.exec"));
    }
}
