//! Ist-Zustand einer Sandbox, wie ihn der Runtime-Adapter meldet.
//!
//! Diese Typen sind bewusst getrennt vom Soll-Zustand (`Sandbox`): der
//! Controller speichert Wünsche, der Adapter liefert Fakten.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Netzwerkschnittstelle einer Sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct NetworkInterface {
    pub name: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub gateway: String,
}

/// Ressourcenlimits, die der Runtime tatsächlich gesetzt hat.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct RuntimeResources {
    #[serde(default)]
    pub memory_limit: u64,
    #[serde(default)]
    pub nano_cpus: u64,
    #[serde(default)]
    pub pids_limit: i64,
}

/// Live-Fakten zu einer Sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct RuntimeFacts {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub container_name: String,
    pub status: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub pack_key: String,
    #[serde(default)]
    pub tool_profile: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub network: Vec<NetworkInterface>,
    #[serde(default)]
    pub uptime_seconds: u64,
    #[serde(default)]
    pub health: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub resources: RuntimeResources,
    /// The OpenShell supervisor of this sandbox (`running`, `degraded`, `stopped`,
    /// `missing`); empty for a runtime that has none.
    #[serde(default)]
    pub supervisor_status: String,
    /// The supervisor's image, which carries its version.
    #[serde(default)]
    pub supervisor_image: String,
}

/// Wunsch zum Erzeugen einer Sandbox – die Sprache des Adapters.
#[derive(Debug, Clone)]
pub struct SandboxSpec {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub pack_key: String,
    pub tool_profile: String,
    pub memory_limit: String,
    pub cpu_limit: f64,
    pub pids_limit: i64,
    /// Bind mounts. Sources come from the persistent-root table, never from a
    /// request; the runner validates them against its persistent base again.
    pub mounts: Vec<crate::domain::persist::MountSpec>,
    /// GPUs to hand to the container. Zero on a node without `container_gpu`.
    pub gpu_count: u32,
    /// How deeply this sandbox's egress is observed (`direct`,
    /// `forward_proxy`, `transparent`, `hybrid`). The adapter needs it before
    /// the container exists: `direct` puts the container on a network with a
    /// route out, and the two TLS-reading modes need the node CA mounted.
    pub egress_mode: String,
    /// The node the sandbox is placed on. `None` means the local node. The dispatcher needs it
    /// at creation, before the sandbox row is committed and a lookup by name could find it.
    pub node_id: Option<uuid::Uuid>,
    /// Soft memory floor (`512m`); `None` leaves the engine's default.
    pub memory_reservation: Option<String>,
    /// Relative CPU weight for the scheduler; `None` leaves the engine's default.
    pub cpu_shares: Option<i64>,
}

/// A script run over SSH on a machine that is not a node yet.
///
/// The credentials are used once, for this call, and are never stored (ADR 0057).
/// There is deliberately no `Debug`, so a password cannot end up in a log line.
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteSshRun {
    /// Host or IP.
    pub host: String,
    /// SSH port.
    pub port: u16,
    /// SSH user.
    pub user: String,
    /// Password, when no key is given.
    pub password: Option<String>,
    /// Private key text, when no password is given.
    pub private_key: Option<String>,
    /// The shell script, sent on stdin to `sh -s`.
    pub script: String,
    /// Environment for the script.
    pub env: std::collections::BTreeMap<String, String>,
    /// Overall timeout in seconds.
    pub timeout_seconds: u32,
}

/// What the runner found on its own host, as raw text the control plane parses.
///
/// Raw rather than parsed on purpose: the parsers are tested Rust, and a
/// second implementation in the runner would be a second opinion.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct HardwareReport {
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub cpuinfo: String,
    #[serde(default)]
    pub meminfo: String,
    /// `nvidia-smi --query-gpu=index,name,memory.total,memory.free --format=csv,noheader,nounits`
    #[serde(default)]
    pub nvidia_smi_csv: Option<String>,
    /// The plain `nvidia-smi` header, for the CUDA version.
    #[serde(default)]
    pub nvidia_smi_header: Option<String>,
    /// Whether the Docker daemon has an NVIDIA runtime registered.
    #[serde(default)]
    pub container_gpu: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// A live package-catalogue question against one baseline image.
///
/// The image and the manager come from the baseline the operator chose; the
/// runner starts that image once and asks its native package manager. Nothing
/// is cached in the database, because a catalogue is a fact about a remote
/// index, not control state.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CatalogueRequest {
    /// The baseline's base image, e.g. `ubuntu:24.04`.
    pub image: String,
    /// `apt`, `dnf`, or `apk`.
    pub package_manager: String,
    /// A package-name fragment, validated before it reaches a shell.
    pub query: String,
}

/// One package a distribution offers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct PackageHit {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

/// What the distribution answered.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CatalogueAnswer {
    pub items: Vec<PackageHit>,
    /// False when no index could be queried; `detail` then says why.
    pub available: bool,
    pub detail: String,
}

/// A request to check that an image really contains the packages a recipe named.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageVerification {
    /// The built tag under `nestbox-recipe/`.
    pub tag: String,
    pub package_manager: String,
    pub packages: Vec<String>,
}

/// Which of the expected packages the image's package database lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct VerificationAnswer {
    pub present: Vec<String>,
    pub missing: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// A request to build an image from a Dockerfile.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BuildRequest {
    /// `nestbox-recipe/<slug>:<hash>`
    pub tag: String,
    pub dockerfile: String,
    /// Extra files by relative name, e.g. the entrypoint script.
    #[serde(default)]
    pub files: Vec<(String, String)>,
}

/// What a build produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct BuildResult {
    pub tag: String,
    #[serde(default)]
    pub image_id: Option<String>,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub success: bool,
}

/// What a command run inside a sandbox produced.
///
/// `truncated` is a separate flag rather than a marker inside the text, so a
/// caller can tell "the output ended" from "the output was cut" without parsing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ExecResult {
    pub exit_code: Option<i64>,
    pub output: String,
    #[serde(default)]
    pub truncated: bool,
}

/// One model artefact to fetch onto a node.
///
/// A single file, named by repository and file name, because that is what a
/// GGUF model is: one complete file whose name the platform would otherwise
/// have to guess from a snapshot listing.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelDownload {
    /// The catalog key; also the directory name under the model base.
    pub model_key: String,
    /// `org/repo` on Hugging Face.
    pub repo: String,
    /// The file inside the repository.
    pub file: String,
    /// Refuse an artefact larger than this, so a mistyped repository cannot
    /// fill the node's disk before anyone notices.
    pub max_bytes: i64,
    /// The node to download on; `None` means the local runner.
    #[serde(default)]
    pub node_id: Option<uuid::Uuid>,
}

/// Where a download left the artefact, and whether it had to fetch it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ModelDownloadResult {
    /// The path on the node, inside the model base.
    pub path: String,
    pub bytes: i64,
    /// True when the file was already there and was not fetched again.
    pub cached: bool,
}

/// A model server container to start for an install.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelServe {
    pub model_key: String,
    /// `repo@sha256:…`; the runtime refuses anything else.
    pub image: String,
    /// The argument vector. `{model}` has already been replaced by the path
    /// inside the container; the runtime still refuses an absolute path that is
    /// not the model it mounted.
    pub command: Vec<String>,
    pub memory_limit_bytes: i64,
    pub cpu_cores: f64,
    pub port: u16,
    /// The node to serve on; `None` means the local runner.
    #[serde(default)]
    pub node_id: Option<uuid::Uuid>,
}

/// A started model server.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ModelServeResult {
    pub name: String,
    #[serde(default)]
    pub container_id: Option<String>,
    pub status: String,
    /// Address `ip:port` the server is published on, on a joined node.
    #[serde(default)]
    pub published: Option<String>,
}

/// Convert an operator-facing memory limit into the form Docker accepts.
///
/// Operators write `2Gi` because that is what Kubernetes and every cloud console
/// uses. Docker accepts only `b`, `k`, `m`, `g` suffixes, and rejects `2Gi` with
/// "should specify the units" — a message that does not mention the real
/// problem. Normalising here means the console can keep the familiar notation
/// and the runtime gets something it understands.
///
/// Returns bytes as `<n>b`, which is unambiguous and cannot be misread as
/// decimal megabytes.
pub fn normalize_memory_limit(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("A memory limit is required.".to_string());
    }

    let lowered = trimmed.to_ascii_lowercase();
    let (number, unit) = split_quantity(&lowered);

    let multiplier = match memory_multiplier(unit) {
        Some(multiplier) => multiplier,
        None => {
            return Err(format!(
                "Unknown memory unit '{unit}'. Use b, k, m, g, or ki, mi, gi."
            ))
        }
    };

    let quantity: u64 = number
        .parse()
        .map_err(|_| format!("'{number}' is not a whole number of memory units."))?;
    if quantity == 0 {
        return Err("A memory limit must be greater than zero.".to_string());
    }

    let bytes = quantity
        .checked_mul(multiplier)
        .ok_or_else(|| "The memory limit is too large.".to_string())?;

    // 1 TiB is far beyond what this appliance runs; refusing it catches a typo
    // like `2t` where `2g` was meant, before it reaches the runtime.
    if bytes > 1_099_511_627_776 {
        return Err("A memory limit above 1 TiB is not accepted.".to_string());
    }

    Ok(format!("{bytes}b"))
}

/// The number of bytes one unit stands for.
///
/// One table for reading a limit and for answering "how big is that", so a
/// sandbox's request and the budget it is checked against cannot disagree about
/// what `2Gi` means.
fn memory_multiplier(unit: &str) -> Option<u64> {
    Some(match unit {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "ki" | "kib" => 1_024,
        "m" | "mb" => 1_000_000,
        "mi" | "mib" => 1_048_576,
        "g" | "gb" => 1_000_000_000,
        "gi" | "gib" => 1_073_741_824,
        "t" | "tb" => 1_000_000_000_000,
        "ti" | "tib" => 1_099_511_627_776,
        _ => return None,
    })
}

/// The size a memory limit names, in bytes.
///
/// The same parsing as `normalize_memory_limit`, because a request has to be
/// compared against a node's budget before it is accepted, and comparing a
/// string with a byte count is how "32Gi fits in 7.8 GiB" gets written.
pub fn memory_bytes(value: &str) -> Result<i64, String> {
    let lowered = value.trim().to_ascii_lowercase();
    if lowered.is_empty() {
        return Err("A memory limit is required.".to_string());
    }
    let (number, unit) = split_quantity(&lowered);
    let multiplier = memory_multiplier(unit)
        .ok_or_else(|| format!("Unknown memory unit '{unit}'. Use b, k, m, g, or ki, mi, gi."))?;
    let quantity: i64 = number
        .parse()
        .map_err(|_| format!("'{number}' is not a whole number of memory units."))?;
    quantity
        .checked_mul(multiplier as i64)
        .ok_or_else(|| "The memory limit is too large.".to_string())
}

/// Split `2gi` into (`2`, `gi`). A trailing non-digit run is the unit.
fn split_quantity(value: &str) -> (&str, &str) {
    let split = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    (&value[..split], &value[split..])
}

/// One forward the runner is asked to hold open for a sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ForwardRequest {
    /// The port on the node.
    pub port: i32,
    /// The port inside the sandbox. For SSH it is only a label: the runner
    /// answers SSH itself and opens a shell.
    pub target_port: i32,
    /// `ssh`, `agent` or `custom`.
    pub purpose: String,
}

/// One appliance container, as the runner describes it.
///
/// These are the components around the sandboxes (control API, gateway, proxy,
/// runner, database, console), not the sandboxes themselves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ComponentState {
    pub service: String,
    pub description: String,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    /// `running`, `exited`, `restarting`, `missing`, ...
    pub state: String,
    /// The container's own health check, when it has one.
    #[serde(default)]
    pub health: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub restarts: i64,
    #[serde(default)]
    pub ports: serde_json::Value,
    /// The container's CPU share of all host cores, as Docker reports it.
    #[serde(default)]
    pub cpu_pct: Option<f32>,
    /// Current memory use and the limit it runs under.
    #[serde(default)]
    pub mem_bytes: Option<i64>,
    #[serde(default)]
    pub mem_limit_bytes: Option<i64>,
    #[serde(default)]
    pub pids: Option<i64>,
    /// The limits the container was started with, as Docker holds them.
    #[serde(default)]
    pub limits: serde_json::Value,
}

/// A change to a platform container's CPU, memory or process limits.
///
/// Narrow on purpose: no device requests, no privileged mode, no mounts. This
/// tunes components that already exist; it is not a way to build a container
/// through the console.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ComponentLimits {
    #[serde(default)]
    pub memory_limit_bytes: Option<i64>,
    #[serde(default)]
    pub nano_cpus: Option<i64>,
    #[serde(default)]
    pub cpu_shares: Option<i64>,
    #[serde(default)]
    pub pids_limit: Option<i64>,
}

impl ComponentLimits {
    /// Whether anything would change. An empty change is a 422 rather than a
    /// success that did nothing.
    pub fn is_empty(&self) -> bool {
        self.memory_limit_bytes.is_none()
            && self.nano_cpus.is_none()
            && self.cpu_shares.is_none()
            && self.pids_limit.is_none()
    }
}

/// A temporary SSH login as the runner checks it: never the password itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SshCredential {
    pub username: String,
    pub salt: String,
    /// `sha256(salt || password)`, hex (`domain::ssh_access::hash_password`).
    pub hash: String,
    /// Unix seconds. The runner refuses a login after it, whatever the clock says here.
    pub expires_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubernetes_units_become_bytes() {
        assert_eq!(normalize_memory_limit("2Gi").as_deref(), Ok("2147483648b"));
        assert_eq!(normalize_memory_limit("512Mi").as_deref(), Ok("536870912b"));
        assert_eq!(normalize_memory_limit("1g").as_deref(), Ok("1000000000b"));
    }

    #[test]
    fn a_limit_has_a_byte_count() {
        assert_eq!(memory_bytes("2Gi"), Ok(2_147_483_648));
        assert_eq!(memory_bytes("512Mi"), Ok(536_870_912));
        assert_eq!(memory_bytes("1g"), Ok(1_000_000_000));
        assert!(memory_bytes("2Xi").is_err());
        assert!(memory_bytes("").is_err());
        // The two readings agree, which is the point of one table.
        let normalized = normalize_memory_limit("3Gi").expect("valid");
        assert_eq!(normalized, format!("{}b", memory_bytes("3Gi").expect("valid")));
    }

    #[test]
    fn a_bare_number_is_bytes() {
        assert_eq!(normalize_memory_limit("1048576").as_deref(), Ok("1048576b"));
    }

    #[test]
    fn nonsense_is_named() {
        assert!(normalize_memory_limit("").is_err());
        assert!(normalize_memory_limit("lots").is_err());
        assert!(normalize_memory_limit("0g").is_err());
        assert!(normalize_memory_limit("2zz").is_err());
    }

    #[test]
    fn an_absurd_value_is_refused_rather_than_sent_to_the_runtime() {
        assert!(normalize_memory_limit("2000g").is_err());
    }
}

/// One tagged image on a node, as its runner reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct NodeImageFact {
    pub image: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub size: i64,
}

/// What a snapshot of a persistent root produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SnapshotFact {
    pub snapshot: String,
    #[serde(default)]
    pub bytes_added: i64,
    #[serde(default)]
    pub bytes_total: i64,
    #[serde(default)]
    pub files: i64,
    #[serde(default)]
    pub seconds: f64,
}
