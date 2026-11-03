//! HTTP: parsing, validation, delegation, and responses.
//!
//! Nothing here contains a rule or a query. A handler reads the request, asks a
//! service, and formats the answer. If a handler grows an `if` that decides
//! something about the domain, that `if` belongs in `services/`.

pub mod access;
pub mod alerts;
pub mod bulk;
pub mod accounting;
pub mod chat_live;
pub mod chats;
pub mod agent_auth;
pub mod auth;
pub mod cluster;
pub mod control;
pub mod cudax;
pub mod events;
pub mod hosting;
pub mod image_templates;
pub mod images;
pub mod inventory;
pub mod inventory_tree;
pub mod jobs;
pub mod mcp;
pub mod media;
pub mod metrics;
pub mod model_grants;
pub mod model_clusters;
pub mod model_readiness;
pub mod move_preflight;
pub mod mounts;
pub mod models;
pub mod node_connect;
pub mod node_join;
pub mod nodes;
pub mod node_ops;
pub mod oidc;
pub mod packs;
pub mod backups;
pub mod persistent_roots;
pub mod permissions;
pub mod policies;
pub mod policy_preview;
pub mod presence;
pub mod privacy;
pub mod privacy_exceptions;
pub mod privacy_refusal;
pub mod principal;
pub mod providers;
pub mod proxies;
pub mod quick_allow;
pub mod quick;
pub mod secrets;
pub mod recipes;
pub mod rule_bulk;
pub mod sandbox_agents;
pub mod sandboxes;
pub mod sandboxes_settings;
pub mod sandboxes_terminal;
pub mod sandboxes_vnc;
pub mod system;
pub mod telemetry;
pub mod tenancy;
pub mod stack;
pub mod search;
pub mod topology;
pub mod tours;
pub mod traffic;
pub mod users;

use axum::Router;
use axum::routing::{any, delete, get, patch, post, put};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::security::headers;
use crate::state::AppState;

/// Build the versioned router.
///
/// The `/api/v1/` prefix is frozen (ADR 0015): a breaking change gets a new
/// prefix rather than silently altering this one.
/// The proxy's content check, with a body limit of its own.
///
/// Merged after the global `RequestBodyLimitLayer` so the larger limit applies to
/// this path and to nothing else. The handler still refuses content over 8 MiB
/// with a clear message; the 9 MiB here is the transport's ceiling, a little above
/// the check's own.
fn internal_privacy_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v1/internal/privacy/inspect",
            post(accounting::inspect),
        )
        // A refusal without a finding is still a decision about a person's
        // request, and the chat has to be able to say so.
        .route(
            "/api/v1/internal/privacy/refusal",
            post(privacy_refusal::refusal),
        )
        // Live reports from the broker and the accounting proxy. Nothing stored.
        .route("/api/v1/internal/chats/live", post(chat_live::accept))
        .layer(axum::extract::DefaultBodyLimit::max(9 * 1024 * 1024))
        .with_state(state)
}

pub fn router(state: AppState) -> Router {
    let mut router = Router::new()
        // System and documentation
        .route("/api/v1/healthz", get(system::healthz))
        .route("/api/v1/readyz", get(system::readyz))
        .route("/api/v1/runtime", get(system::runtime_info))
        .route("/api/v1/openapi.json", get(system::openapi))
        .route("/api/v1/docs", get(system::docs))
        .route("/api/schema/v1", get(system::openapi))
        // Authentication
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/auth/csrf", post(auth::csrf))
        // Identity providers and invitations
        .route("/api/v1/auth/options", get(oidc::options))
        .route("/api/v1/auth/oidc/{provider}/start", get(oidc::start))
        .route("/api/v1/auth/oidc/{provider}/callback", get(oidc::callback))
        .route("/api/v1/auth/invitations/{token}", get(tenancy::preview_invitation))
        // Customers, data centers, sites, grants, invitations
        .route("/api/v1/organizations", get(tenancy::organizations).post(tenancy::create_customer))
        .route("/api/v1/sites", get(tenancy::list_sites).post(tenancy::create_site))
        .route("/api/v1/sites/{id}", delete(tenancy::delete_site))
        .route("/api/v1/sandboxes/{id}/grants", get(tenancy::list_grants))
        .route(
            "/api/v1/sandboxes/{id}/grants/{user_id}",
            put(tenancy::grant).delete(tenancy::revoke),
        )
        .route("/api/v1/invitations", get(tenancy::list_invitations).post(tenancy::invite))
        .route("/api/v1/invitations/{id}", delete(tenancy::revoke_invitation))
        // Environments
        .route("/api/v1/users", get(users::list).post(users::create))
        .route("/api/v1/users/{id}", axum::routing::put(users::update).delete(users::remove))
        .route("/api/v1/users/{id}/password", post(users::set_password))
        .route("/api/v1/environments", get(system::environments).post(tenancy::create_data_center))
        .route("/api/v1/environments/current", get(system::current_environment))
        // Sandboxes
        .route(
            "/api/v1/sandboxes",
            get(sandboxes::list).post(sandboxes::create),
        )
        .route(
            "/api/v1/sandboxes/{id}",
            get(sandboxes::get).delete(sandboxes::delete),
        )
        .route("/api/v1/sandboxes/{id}/actions", post(sandboxes::action))
        .route("/api/v1/sandboxes/{id}/logs", get(sandboxes::logs))
        .route("/api/v1/sandboxes/{id}/exec", post(sandboxes::exec))
        .route("/api/v1/sandboxes/{id}/inference", get(telemetry::sandbox_inference))
        .route("/api/v1/sandboxes/{id}/rules", get(policies::sandbox_rules))
        .route("/api/v1/sandboxes/{id}/allow", post(policies::allow_destination))
        // Which sandboxes belong together.
        .route("/api/v1/sandboxes/{id}/stack", put(sandboxes::put_stack))
        .route("/api/v1/sandboxes/{id}/resources", put(sandboxes::put_resources))
        .route("/api/v1/host/load", get(sandboxes::host_load))
        .route("/api/v1/sandboxes/{id}/agent-config", post(sandboxes::reconfigure))
        // One sandbox's controls: agents' sign-in modes, egress mode, sessions.
        .route("/api/v1/sandboxes/control-overview", get(control::overview))
        .route("/api/v1/sandboxes/{id}/control", get(control::summary))
        .route("/api/v1/sandboxes/{id}/agents/auth", get(agent_auth::list))
        .route("/api/v1/sandboxes/{id}/agents/activity", get(agent_auth::activity))
        .route("/api/v1/sandboxes/{id}/agents/{agent}/auth", put(agent_auth::set))
        .route("/api/v1/sandboxes/{id}/agents/{agent}/restart", post(agent_auth::restart))
        .route("/api/v1/sandboxes/{id}/presence", get(presence::list))
        .route("/api/v1/sandboxes/{id}/models", get(model_grants::sandbox_models))
        .route("/api/v1/sandboxes/{id}/move-preflight", get(move_preflight::preflight))
        .route("/api/v1/sandboxes/{id}/recreate", post(sandboxes::recreate))
        // Admin Center: resource history, processes, inventory
        .route(
            "/api/v1/internal/metrics/ingest",
            post(metrics::ingest),
        )
        .route("/api/v1/sandboxes/{id}/metrics", get(metrics::metrics))
        .route("/api/v1/sandboxes/login-sessions", get(access::login_overview))
        .route("/api/v1/sandboxes/{id}/processes", get(metrics::processes))
        .route("/api/v1/sandboxes/{id}/live/{kind}", get(metrics::live_view))
        .route("/api/v1/sandboxes/{id}/processes/{pid}/action", post(metrics::process_action))
        .route("/api/v1/sandboxes/{id}/supervisor", get(cluster::sandbox_supervisor))
        .route("/api/v1/sandboxes/{id}/inventory", get(inventory::get))
        .route(
            "/api/v1/sandboxes/{id}/inventory/refresh",
            post(inventory::refresh),
        )
        // Admin Center: terminal, SSH, ports, sessions
        .route("/api/v1/sandboxes/{id}/desktop", get(sandboxes_vnc::desktop))
        .route(
            "/api/v1/sandboxes/{id}/terminal-sessions",
            get(sandboxes_terminal::sessions),
        )
        .route(
            "/api/v1/sandboxes/{id}/terminal",
            get(sandboxes_terminal::terminal),
        )
        .route(
            "/api/v1/sandboxes/{id}/terminal-tabs",
            get(sandboxes_terminal::tabs).put(sandboxes_terminal::save_tabs),
        )
        // Closing a tab for everybody: the tmux session ends, every viewer leaves.
        .route(
            "/api/v1/sandboxes/{id}/terminal-tabs/{tab}",
            delete(presence::close_tab),
        )
        .route(
            "/api/v1/sandboxes/{id}/settings",
            get(sandboxes_settings::get).put(sandboxes_settings::update),
        )
        .route(
            "/api/v1/sandboxes/{id}/supervisor/actions",
            post(sandboxes_settings::action),
        )
        .route("/api/v1/sandboxes/{id}/connect", get(access::connect))
        .route("/api/v1/sandboxes/{id}/ssh-access", get(access::ssh_access))
        .route("/api/v1/sandboxes/{id}/ssh-keys", post(access::add_user_key))
        .route("/api/v1/sandboxes/{id}/ssh-keys/{key}", delete(access::remove_user_key))
        .route("/api/v1/sandboxes/{id}/ssh-credentials", post(access::issue_credential))
        .route("/api/v1/sandboxes/{id}/ssh-credentials/{credential}", delete(access::revoke_credential))
        .route("/api/v1/sandboxes/{id}/stack-topology", get(stack::stack_topology))
        .route("/api/v1/system/components", get(stack::components))
        .route("/api/v1/system/components/{service}/logs", get(stack::component_logs))
        .route(
            "/api/v1/system/components/{service}/limits",
            axum::routing::put(stack::set_component_limits),
        )
        .route(
            "/api/v1/system/components/{service}/restart",
            post(stack::restart_component),
        )
        .route("/api/v1/nodes/{id}/endpoint", put(access::put_node_endpoint))
        .route("/api/v1/sandboxes/{id}/ssh-key", get(access::ssh_key))
        .route(
            "/api/v1/sandboxes/{id}/ssh-key/download",
            get(access::download_ssh_key),
        )
        .route(
            "/api/v1/sandboxes/{id}/ssh-key/rotate",
            post(access::rotate_ssh_key),
        )
        .route(
            "/api/v1/sandboxes/{id}/ports",
            get(access::ports).post(access::add_port),
        )
        .route(
            "/api/v1/sandboxes/{id}/ports/{forward_id}",
            delete(access::remove_port),
        )
        .route("/api/v1/sandboxes/{id}/sessions", get(access::sessions))
        // Admin Center: traffic, map, effective policies, MCP calls
        .route("/api/v1/traffic", get(traffic::environment))
        .route("/api/v1/traffic/decisions", get(traffic::decisions))
        .route("/api/v1/sandboxes/{id}/traffic", get(traffic::traffic))
        .route(
            "/api/v1/sandboxes/{id}/traffic/{event_id}/body",
            get(traffic::body),
        )
        .route("/api/v1/sandboxes/{id}/map", get(traffic::map))
        .route(
            "/api/v1/sandboxes/{id}/policies/effective",
            get(traffic::effective_policies),
        )
        .route(
            "/api/v1/sandboxes/{id}/mcp/calls",
            get(traffic::mcp_calls),
        )
        // MCP registry and sandbox attachments
        .route(
            "/api/v1/mcp/servers",
            get(mcp::list_servers).post(mcp::create_server),
        )
        .route(
            "/api/v1/mcp/servers/{key}",
            get(mcp::get_server)
                .put(mcp::update_server)
                .delete(mcp::delete_server),
        )
        .route(
            "/api/v1/sandboxes/{id}/mcp/servers",
            get(mcp::sandbox_servers).post(mcp::attach_server),
        )
        .route(
            "/api/v1/sandboxes/{id}/mcp/servers/{key}",
            delete(mcp::detach_server),
        )
        .route(
            "/api/v1/sandboxes/{id}/mcp/servers/{key}/discover",
            post(mcp::discover),
        )
        // Accounting: organisation overview, rows, retention, purge.
        .route("/api/v1/accounting/overview", get(accounting::overview))
        .route("/api/v1/accounting/requests", get(accounting::requests))
        .route("/api/v1/accounting/usage", get(accounting::usage))
        .route("/api/v1/accounting/prices", get(accounting::prices).put(accounting::put_price))
        .route(
            "/api/v1/accounting/settings",
            get(accounting::get_settings).put(accounting::put_settings),
        )
        .route("/api/v1/accounting/purge", post(accounting::purge))
        // Internal: the accounting proxy is the only writer of these rows.
        .route("/api/v1/internal/accounting/record", post(accounting::ingest))
        // The accounting proxy asks which models this sandbox may use and where they live.
        .route(
            "/api/v1/internal/accounting/models/{sandbox}",
            get(model_grants::internal_models),
        )
        // Content privacy (DLP): policy, a filter test, incidents, purge.
        .route(
            "/api/v1/privacy/settings",
            get(privacy::get_settings).put(privacy::put_settings),
        )
        .route("/api/v1/privacy/settings/{sandbox_id}", axum::routing::delete(privacy::clear_settings))
        .route("/api/v1/privacy/inspect", post(privacy::inspect))
        .route("/api/v1/privacy/incidents", get(privacy::get_incidents))
        .route("/api/v1/privacy/overview", get(privacy::dlp_overview))
        .route("/api/v1/privacy/log", get(privacy::dlp_log))
        .route("/api/v1/privacy/changes", get(privacy::dlp_changes))
        // Chats: the turn list, its groups, one turn, the live stream, and the
        // audit of an administrator watching it.
        .route("/api/v1/chats/turns", get(chats::turns))
        .route("/api/v1/chats/groups", get(chats::groups))
        .route("/api/v1/chats/turns/{id}", get(chats::turn))
        .route("/api/v1/chats/stream", get(events::chats_stream))
        .route("/api/v1/live/watch", post(chats::live_watch))
        // DLP exceptions: allow a finding on purpose.
        .route(
            "/api/v1/privacy/exceptions",
            get(privacy_exceptions::list).post(privacy_exceptions::create),
        )
        .route(
            "/api/v1/privacy/exceptions/{id}",
            axum::routing::delete(privacy_exceptions::delete),
        )
        // A rule with an end, created from the live view.
        .route(
            "/api/v1/sandboxes/{id}/quick-allow",
            get(quick_allow::list).post(quick_allow::create),
        )
        .route("/api/v1/privacy/purge", post(privacy::purge))
        // Media rendering: the environment default, a sandbox's own choice,
        // the renders and their assets.
        .route(
            "/api/v1/media/settings",
            get(media::get_settings).put(media::put_settings),
        )
        .route("/api/v1/media/users", get(media::user_grants))
        .route("/api/v1/media/users/{id}", put(media::set_user_grant))
        .route(
            "/api/v1/sandboxes/{id}/media",
            get(media::sandbox_settings).put(media::put_sandbox_settings),
        )
        .route("/api/v1/sandboxes/{id}/media/renders", get(media::renders))
        .route("/api/v1/media/purge", post(media::purge))
        .route("/api/v1/media/renders/{id}", delete(media::delete_render))
        .route("/api/v1/media/renders/{id}/asset/{name}", get(media::asset))
        // Internal: node-token endpoints for the proxy and the runner.
        //
        // The policy check is registered twice and accepts any method, for one
        // reason: Envoy's ext_authz HTTP filter forwards the original method and
        // appends the original path to its `path_prefix`. So the proxy calls
        // `/internal/policy/check/<the path it is asking about>` with `GET` when
        // the agent used `GET`. The handler reads headers, not a body, and
        // ignores the suffix; without both routes every request would be a 404
        // and therefore (fail closed) a deny.
        .route("/api/v1/internal/metrics/batch", post(metrics::ingest_node))
        // The media MCP server is the caller here, not a browser: it holds the
        // appliance's media token and names the sandbox it renders for.
        .route("/api/v1/internal/media/render", post(media::bridge_render))
        .route("/api/v1/internal/media/info", get(media::bridge_info))
        .route("/api/v1/internal/media/renders/{id}/asset", get(media::bridge_asset))
        .route("/api/v1/internal/ssh-sessions", post(access::ssh_session_event))
        .route(
            "/api/v1/internal/policy/check",
            any(policies::internal_check),
        )
        .route(
            "/api/v1/internal/policy/check/{*suffix}",
            any(policies::internal_check),
        )
        // Envoy appends the original path to the prefix; for a request to `/`
        // that is a trailing slash, which the wildcard route does not match.
        // Without this route the proxy's very first plain-HTTP request would be
        // a 404 — a deny that never asked a question.
        .route(
            "/api/v1/internal/policy/check/",
            any(policies::internal_check),
        )
        .route(
            "/api/v1/internal/telemetry/batch",
            post(telemetry::internal_batch),
        )
        // Policies
        .route("/api/v1/policies", get(policies::list).post(policies::create))
        .route("/api/v1/policies/global", get(policies::global))
        .route(
            "/api/v1/policies/{id}",
            get(policies::get).put(policies::update).delete(policies::delete),
        )
        .route("/api/v1/policies/{id}/diff", get(policies::diff))
        .route("/api/v1/policies/{id}/versions", get(policies::versions))
        .route("/api/v1/policies/{id}/clone", post(policies::clone_policy))
        .route("/api/v1/policies/{id}/assign", post(policies::assign_many))
        .route("/api/v1/network-rules/copy", post(policies::copy_rules))
        // The preset library, the editor's schema, and a sandbox's policy
        .route(
            "/api/v1/policy-templates",
            get(policies::list_templates).post(policies::create_template),
        )
        .route(
            "/api/v1/policy-templates/{id}/instantiate",
            post(policies::instantiate_template),
        )
        .route("/api/v1/policy-schema", get(policies::schema))
        .route(
            "/api/v1/sandboxes/{id}/policy",
            get(policies::get_sandbox_policy)
                .put(policies::put_sandbox_policy)
                .delete(policies::delete_sandbox_policy),
        )
        // Network rules
        .route(
            "/api/v1/network-rules",
            get(policies::list_rules).post(policies::create_rule),
        )
        .route("/api/v1/network-rules/{id}", delete(policies::delete_rule))
        // DNS rules: which names the proxy may resolve.
        .route(
            "/api/v1/dns-rules",
            get(policies::list_dns_rules).post(policies::create_dns_rule),
        )
        .route("/api/v1/dns-rules/{id}", delete(policies::delete_dns_rule))
        // How deeply one sandbox is observed on the way out.
        .route(
            "/api/v1/sandboxes/{id}/egress-mode",
            get(policies::get_egress_mode).put(policies::put_egress_mode),
        )
        // Rule drafts and temporary windows
        .route(
            "/api/v1/rule-drafts",
            get(policies::list_drafts).post(policies::draft_from_event),
        )
        // Everything a period of observation denied or only recorded, as drafts.
        .route("/api/v1/rule-drafts/bulk", post(rule_bulk::create))
        .route("/api/v1/rule-drafts/{id}/approve", post(policies::approve_draft))
        .route("/api/v1/rule-drafts/{id}/reject", post(policies::reject_draft))
        .route(
            "/api/v1/traffic-windows",
            get(policies::windows).post(policies::open_window),
        )
        .route(
            "/api/v1/traffic-windows/{id}",
            delete(policies::close_window),
        )
        // Telemetry
        .route("/api/v1/telemetry/events", get(telemetry::history))
        .route("/api/v1/telemetry/facets", get(telemetry::facets))
        .route("/api/v1/telemetry/stream", get(events::telemetry_stream))
        .route("/api/v1/telemetry/ingest", post(telemetry::ingest))
        .route("/api/v1/telemetry/seed-demo", post(telemetry::seed_demo))
        .route("/api/v1/events/stream", get(events::state_stream))
        // Cluster
        .route("/api/v1/gateways", get(cluster::list_gateways))
        .route("/api/v1/gateways/{id}", get(cluster::get_gateway))
        .route(
            "/api/v1/gateways/{id}/settings",
            put(cluster::update_settings),
        )
        .route("/api/v1/gateways/{id}/sync", post(cluster::sync))
        .route(
            "/api/v1/supervisors",
            get(cluster::supervisors).post(cluster::create_supervisor),
        )
        .route(
            "/api/v1/supervisors/{id}",
            patch(cluster::bind_supervisor),
        )
        .route(
            "/api/v1/internal/supervisors/{id}/report",
            post(cluster::report_supervisor),
        )
        .route("/api/v1/topology", get(topology::topology))
        // Providers
        .route(
            "/api/v1/providers",
            get(providers::list).post(providers::create),
        )
        .route(
            "/api/v1/providers/{id}",
            get(providers::get)
                .put(providers::update)
                .delete(providers::delete),
        )
        // Registered before `{id}` so a static path is never read as an id.
        .route("/api/v1/providers/probe-all", post(providers::probe_all))
        .route("/api/v1/providers/{id}/probe", post(providers::probe))
        // Exit proxies: the catalogue, and which entry a sandbox or a provider
        // leaves through. The exit is a separate question from the egress mode
        // (how much is observed), which is why it is its own resource.
        .route("/api/v1/proxies", get(proxies::list).post(proxies::create))
        .route(
            "/api/v1/proxies/{id}",
            get(proxies::get).put(proxies::update).delete(proxies::delete),
        )
        .route(
            "/api/v1/sandboxes/{id}/proxy",
            get(proxies::get_sandbox_proxy).put(proxies::put_sandbox_proxy),
        )
        .route(
            "/api/v1/providers/{id}/proxy",
            put(proxies::put_provider_proxy),
        )
        .route("/api/v1/secrets", get(secrets::list).post(secrets::put))
        .route("/api/v1/secrets/{id}", axum::routing::delete(secrets::delete))
        // Models
        .route("/api/v1/models", get(models::catalog))
        .route("/api/v1/models/installed", get(models::installed))
        .route("/api/v1/models/hardware", get(models::hardware))
        .route("/api/v1/models/facets", get(models::facets))
        .route("/api/v1/models/requirements", get(models::requirements))
        .route("/api/v1/models/installs", get(models::installs))
        .route("/api/v1/models/sync", post(models::sync))
        // CUDA-X libraries
        .route("/api/v1/cuda-x/components", get(cudax::components))
        .route("/api/v1/cuda-x/installs", get(cudax::installs))
        .route("/api/v1/cuda-x/components/{key}/assign", post(cudax::assign))
        .route("/api/v1/cuda-x/components/{key}/install", post(cudax::install))
        .route("/api/v1/cuda-x/components/{key}/remove", post(cudax::remove))
        .route("/api/v1/models/{key}/install", post(models::install))
        .route("/api/v1/models/{key}/uninstall", post(models::uninstall))
        .route("/api/v1/models/{key}/install", patch(models::update_install))
        .route("/api/v1/models/{key}/logs", get(models::logs))
        .route("/api/v1/models/{key}/metrics", get(models::metrics))
        // Load the model now and answer one tiny request, so the next real one is fast.
        .route("/api/v1/models/{key}/warm", post(model_readiness::warm))
        // Which models a sandbox may use, granted to the environment, a user or it.
        .route(
            "/api/v1/model-grants",
            get(model_grants::list).post(model_grants::create),
        )
        .route("/api/v1/model-grants/{id}", delete(model_grants::remove))
        // Nodes
        .route("/api/v1/nodes", get(nodes::list))
        .route("/api/v1/sandboxes/{id}/move", post(node_join::move_sandbox))
        .route("/api/v1/sandboxes/{id}/recover", post(node_join::recover_sandbox))
        .route("/api/v1/sandbox-moves", get(node_join::moves))
        .route("/api/v1/persistent-roots/{id}/snapshots", get(node_join::root_snapshots))
        .route("/api/v1/snapshots/prune", post(node_join::prune_snapshots))
        .route("/api/v1/sandbox-moves/{id}", get(node_join::get_move))
        .route("/api/v1/registry/images", get(images::registry))
        .route("/api/v1/images/matrix", get(images::matrix))
        .route("/api/v1/image-assignments", post(images::assign))
        .route("/api/v1/image-assignments/{id}", delete(images::unassign))
        .route("/api/v1/node-join-tokens", post(node_join::create_token))
        .route("/api/v1/join/{token}/script", get(node_join::script))
        .route("/api/v1/join/{token}", post(node_join::join))
        .route("/api/v1/node/compose", get(node_join::compose))
        .route("/api/v1/join/{token}/cloud-init", get(hosting::cloud_init))
        .route("/api/v1/host-accounts", get(hosting::accounts).post(hosting::add_account))
        .route("/api/v1/host-accounts/{id}", delete(hosting::remove_account))
        .route("/api/v1/host-accounts/{id}/types", get(hosting::types))
        .route("/api/v1/hosts", get(hosting::hosts).post(hosting::burst))
        .route("/api/v1/hosts/{id}", delete(hosting::destroy))
        .route("/api/v1/nodes/{id}", get(nodes::get).delete(node_join::remove))
        .route("/api/v1/nodes/{id}/sandboxes", get(nodes::sandboxes))
        .route("/api/v1/nodes/{id}/display", put(nodes::update_display))
        .route("/api/v1/nodes/{id}/probe", post(nodes::probe))
        // A machine joined by address, its tunnel state, and how it reaches the internet.
        .route("/api/v1/nodes/connect", post(node_connect::connect))
        .route("/api/v1/nodes/tunnels", get(node_connect::tunnels))
        .route("/api/v1/nodes/{id}/egress-lock", put(node_connect::set_egress_lock))
        .route("/api/v1/node/egress-lock", get(node_connect::node_egress_lock))
        // Builder catalogs
        .route("/api/v1/baselines", get(packs::baselines))
        .route("/api/v1/agent-packs", get(packs::agent_packs))
        .route("/api/v1/tool-catalog", get(packs::tool_catalog))
        // Recipes
        .route("/api/v1/recipes", get(recipes::list).post(recipes::create))
        .route(
            "/api/v1/recipes/{id}",
            get(recipes::get).patch(recipes::update).delete(recipes::delete),
        )
        .route("/api/v1/recipes/{id}/build", post(recipes::build))
        .route(
            "/api/v1/recipes/{id}/apply-template",
            post(recipes::apply_template),
        )
        .route("/api/v1/packages", get(packs::catalogue))
        .route(
            "/api/v1/image-templates",
            get(image_templates::list).post(image_templates::create),
        )
        .route(
            "/api/v1/image-templates/default",
            get(image_templates::default_template),
        )
        .route(
            "/api/v1/image-templates/{id}",
            get(image_templates::get)
                .patch(image_templates::update)
                .delete(image_templates::delete),
        )
        .route("/api/v1/recipes/{id}/instantiate", post(recipes::instantiate))
        // Persistent roots
        .route(
            "/api/v1/persistent-roots",
            get(persistent_roots::list).post(persistent_roots::create),
        )
        .route(
            "/api/v1/persistent-roots/{id}",
            get(persistent_roots::get).delete(persistent_roots::delete),
        )
        .route("/api/v1/persistent-roots/{id}/mount", post(persistent_roots::mount))
        .route("/api/v1/persistent-roots/{id}/dismount", post(persistent_roots::dismount))
        .route("/api/v1/persistent-roots/{id}/usage", get(persistent_roots::usage))
        .route("/api/v1/persistent-roots/{id}/backups", get(backups::list_for_root).post(backups::create))
        .route("/api/v1/backups", get(backups::list_all))
        .route("/api/v1/backups/{id}/restore", post(backups::restore))
        .route("/api/v1/backups/{id}", axum::routing::delete(backups::delete))
        // Sandbox agents
        .route(
            "/api/v1/sandboxes/{id}/agents",
            get(sandbox_agents::list).post(sandbox_agents::add),
        )
        .route(
            "/api/v1/sandboxes/{id}/agents/{instance}",
            delete(sandbox_agents::remove),
        )
        // Jobs and schedules
        .route("/api/v1/jobs", get(jobs::list))
        .route("/api/v1/jobs/{id}", get(jobs::get))
        .route("/api/v1/jobs/{id}/logs", get(jobs::logs))
        .route("/api/v1/schedules", get(jobs::schedules))
        .route("/api/v1/schedules/{id}/run", post(jobs::run_schedule))
        // Control Center (Wave 0 stubs; packages B01-B12 fill the services behind them).
        .route("/api/v1/inventory/tree", get(inventory_tree::tree))
        .route("/api/v1/search", get(search::search))
        .route("/api/v1/sandboxes/bulk", post(bulk::sandboxes))
        .route("/api/v1/models/{key}/install-bulk", post(models::install_bulk))
        .route(
            "/api/v1/model-clusters",
            get(model_clusters::list).post(model_clusters::create),
        )
        .route(
            "/api/v1/model-clusters/{id}",
            get(model_clusters::get)
                .put(model_clusters::update)
                .delete(model_clusters::delete),
        )
        .route("/api/v1/model-clusters/{id}/members", post(model_clusters::add_member))
        .route(
            "/api/v1/model-clusters/{id}/members/{install_id}",
            delete(model_clusters::remove_member),
        )
        .route("/api/v1/internal/accounting/clusters/{id}", get(model_clusters::internal_view))
        .route("/api/v1/nodes/{id}/summary", get(node_ops::summary))
        .route("/api/v1/nodes/{id}/drain", post(node_ops::drain))
        .route("/api/v1/nodes/quick", post(node_ops::quick))
        .route("/api/v1/alerts", get(alerts::list))
        .route("/api/v1/sandboxes/{id}/mounts", get(mounts::get).put(mounts::put))
        .route("/api/v1/policies/preview", post(policy_preview::preview))
        .route("/api/v1/policies/{id}/compiled", get(policy_preview::compiled))
        .route("/api/v1/sandboxes/{id}/quick", put(quick::set))
        .route(
            "/api/v1/permissions",
            get(permissions::list).put(permissions::put).delete(permissions::delete),
        )
        .route("/api/v1/users/me/tours/{tour}", put(tours::mark_seen))
        // Audit
        .route("/api/v1/audit-events", get(system::audit_events))
        // A CONNECT's request target is `host:port`, not a path, so no route can
        // match it. The proxy asks about a tunnel with exactly that request, and
        // this is the one place it can be answered; every other unmatched
        // request is the plain 404 it always was.
        .fallback(policies::connect_or_not_found)
        .with_state(state.clone());

    // Security headers on every response, including errors and streams. A CSP
    // that appears only on success is not a CSP.
    for layer in headers::security_headers() {
        router = router.layer(layer);
    }
    if state.config.session_cookie_secure {
        router = router.layer(headers::hsts_header());
    }

    // A body limit is the cheapest defence against a request that tries to
    // exhaust memory before any handler sees it.
    router = router.layer(RequestBodyLimitLayer::new(state.config.body_limit_bytes));

    // One exception, and only after the global limit is in place: the proxy's
    // content check takes a whole model request, and a real one carries a system
    // prompt and the tool definitions with it. With the 1 MiB default every real
    // agent request was refused before the check could answer (finding A1).
    router = router.merge(internal_privacy_router(state.clone()));

    router.layer(TraceLayer::new_for_http())
}
