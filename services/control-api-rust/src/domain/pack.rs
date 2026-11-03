//! Base operating systems, agent packs, and the tool catalog.
//!
//! All three are catalog data (ADR 0013): a new agent is a row, not a `match`
//! arm. The types here are what the service reads from the tables and what the
//! builder composes a recipe from.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which package manager a base image uses; decides which tool name applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PackageManager {
    Apt,
    Dnf,
    /// Alpine's `apk`: a different package index and a musl base image.
    Apk,
}

impl PackageManager {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Apk => "apk",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "apt" => Self::Apt,
            "dnf" => Self::Dnf,
            "apk" => Self::Apk,
            _ => return None,
        })
    }
}

/// A base operating system a recipe can start from.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Baseline {
    pub key: String,
    pub name: String,
    pub recommended: bool,
    pub base_image: String,
    pub package_manager: PackageManager,
    pub default_tools: Vec<String>,
    pub openshell_ready: bool,
    pub notes: Option<String>,
}

/// How an agent pack gets into an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Script,
    Git,
    Npm,
    Pip,
    Binary,
    /// The pack is its own prebuilt image.
    Image,
    /// A graphical desktop (VNC, XFCE, Chrome); the install is a fixed script
    /// shipped with the control plane, not a reference an operator supplies.
    Desktop,
    None,
}

impl InstallKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Git => "git",
            Self::Npm => "npm",
            Self::Pip => "pip",
            Self::Binary => "binary",
            Self::Image => "image",
            Self::Desktop => "desktop",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "script" => Self::Script,
            "git" => Self::Git,
            "npm" => Self::Npm,
            "pip" => Self::Pip,
            "binary" => Self::Binary,
            "image" => Self::Image,
            "desktop" => Self::Desktop,
            "none" => Self::None,
            _ => return None,
        })
    }
}

/// An agent that can be put into a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentPack {
    pub key: String,
    pub name: String,
    pub vendor: Option<String>,
    pub description: Option<String>,
    /// Baseline keys this pack supports; empty means all.
    pub base_os: Vec<String>,
    pub install_kind: InstallKind,
    pub install_ref: Option<String>,
    /// Paths under the home that must survive a rebuild.
    pub home_paths: Vec<String>,
    pub requires_gpu: bool,
    pub needs_inference: bool,
    pub default_image: String,
    pub default_command: Vec<String>,
    pub ports: Vec<i32>,
    pub env_slots: Vec<String>,
    pub enabled: bool,
}

impl AgentPack {
    /// Whether this pack can be built on the given baseline.
    pub fn supports(&self, baseline_key: &str) -> bool {
        self.base_os.is_empty() || self.base_os.iter().any(|k| k == baseline_key)
    }
}

/// A tool from the catalog, with its name per package manager.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Tool {
    pub key: String,
    pub name: String,
    pub group: String,
    pub description: Option<String>,
    pub apt_name: Option<String>,
    pub rpm_name: Option<String>,
    pub apk_name: Option<String>,
}

impl Tool {
    /// The package name for this manager, if the tool is packaged for it.
    pub fn package_for(&self, manager: PackageManager) -> Option<&str> {
        match manager {
            PackageManager::Apt => self.apt_name.as_deref(),
            PackageManager::Dnf => self.rpm_name.as_deref(),
            // Alpine's names mostly match Debian's, but not all of them, so the
            // column is explicit and a missing entry is "not packaged here".
            PackageManager::Apk => self.apk_name.as_deref(),
        }
    }
}

/// A tool as one baseline can install it.
///
/// The catalog row carries both names because the console shows the whole
/// matrix; `package` is the single name a build for `package_manager` would
/// put in the Dockerfile, and `available` says whether the tool has a name for
/// that manager at all. Without a baseline, both names are shown and no
/// resolution is claimed, which is why `package_manager` is absent rather than
/// guessed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolForBaseline {
    pub key: String,
    pub name: String,
    pub group: String,
    pub description: Option<String>,
    pub apt_name: Option<String>,
    pub rpm_name: Option<String>,
    pub apk_name: Option<String>,
    /// The name for the resolved package manager; absent without a baseline.
    pub package: Option<String>,
    /// The manager `package` was resolved for; absent without a baseline.
    pub package_manager: Option<PackageManager>,
    /// False when the resolved manager has no package name for this tool.
    pub available: bool,
}

impl ToolForBaseline {
    /// The catalog answer when no baseline was asked for: both names, no
    /// resolution, and availability judged by the database constraint that a
    /// tool has at least one name.
    pub fn unresolved(tool: Tool) -> Self {
        let available =
            tool.apt_name.is_some() || tool.rpm_name.is_some() || tool.apk_name.is_some();
        Self {
            key: tool.key,
            name: tool.name,
            group: tool.group,
            description: tool.description,
            apt_name: tool.apt_name,
            rpm_name: tool.rpm_name,
            apk_name: tool.apk_name,
            package: None,
            package_manager: None,
            available,
        }
    }

    /// The catalog answer for one package manager, naming the package a build
    /// would install or marking the tool as not packaged for it.
    pub fn resolved(tool: Tool, manager: PackageManager) -> Self {
        let package = tool.package_for(manager).map(str::to_string);
        let available = package.is_some();
        Self {
            key: tool.key,
            name: tool.name,
            group: tool.group,
            description: tool.description,
            apt_name: tool.apt_name,
            rpm_name: tool.rpm_name,
            apk_name: tool.apk_name,
            package,
            package_manager: Some(manager),
            available,
        }
    }
}

/// What a pack runs with when no recipe overrides it.
///
/// This is the row a sandbox is created from: the image to pull, the command
/// that keeps it alive, the ports it publishes, the environment variable names
/// its provider fills (ADR 0005), and the paths under its home that must
/// survive a rebuild.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackDefaults {
    pub pack_key: String,
    pub image: String,
    pub command: Vec<String>,
    pub ports: Vec<i32>,
    pub env_slots: Vec<String>,
    pub home_paths: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_base_os_list_means_every_baseline() {
        let pack = AgentPack {
            key: "x".into(),
            name: "X".into(),
            vendor: None,
            description: None,
            base_os: vec![],
            install_kind: InstallKind::None,
            install_ref: None,
            home_paths: vec![],
            requires_gpu: false,
            needs_inference: false,
            default_image: "img".into(),
            default_command: vec![],
            ports: vec![],
            env_slots: vec![],
            enabled: true,
        };
        assert!(pack.supports("rocky-9"));
        let mut limited = pack;
        limited.base_os = vec!["ubuntu-24.04".into()];
        assert!(limited.supports("ubuntu-24.04"));
        assert!(!limited.supports("rocky-9"));
    }

    #[test]
    fn a_tool_without_a_name_for_the_manager_is_flagged() {
        let build_tools = Tool {
            key: "build-essential".into(),
            name: "Build tools".into(),
            group: "build".into(),
            description: None,
            apt_name: Some("build-essential".into()),
            rpm_name: None,
            apk_name: Some("build-base".into()),
        };

        let on_debian = ToolForBaseline::resolved(build_tools.clone(), PackageManager::Apt);
        assert_eq!(on_debian.package.as_deref(), Some("build-essential"));
        assert!(on_debian.available);

        let on_rocky = ToolForBaseline::resolved(build_tools.clone(), PackageManager::Dnf);
        assert_eq!(on_rocky.package, None);
        assert!(!on_rocky.available, "dnf has no package for build-essential");

        let unresolved = ToolForBaseline::unresolved(build_tools);
        assert_eq!(unresolved.package, None);
        assert!(unresolved.available, "it is installable on at least one manager");
    }

    #[test]
    fn a_tool_resolves_to_the_managers_name() {
        let tool = Tool {
            key: "vim".into(),
            name: "Vim".into(),
            group: "editor".into(),
            description: None,
            apt_name: Some("vim".into()),
            rpm_name: Some("vim-enhanced".into()),
            apk_name: Some("vim".into()),
        };
        assert_eq!(tool.package_for(PackageManager::Apt), Some("vim"));
        assert_eq!(tool.package_for(PackageManager::Dnf), Some("vim-enhanced"));
    }
}
