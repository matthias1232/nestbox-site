//! The tenant tree: organisations, data centers, sites, invitations (ADR 0044, 0045).
//!
//! Pure rules only. Names, slugs and roles are checked here so the service and the tests
//! share one definition of "valid".

use crate::domain::auth::Role;

/// How an organisation relates to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationKind {
    /// The provider itself. Its administrators reach every tenant below.
    Platform,
    /// A reseller. Its administrators reach the customers created under it.
    Partner,
    /// An end customer. Reaches only its own data centers.
    Customer,
}

impl OrganizationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Partner => "partner",
            Self::Customer => "customer",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform" => Some(Self::Platform),
            "partner" => Some(Self::Partner),
            "customer" => Some(Self::Customer),
            _ => None,
        }
    }

    /// Whether an organisation of this kind may have customers of its own.
    pub fn may_have_children(self) -> bool {
        matches!(self, Self::Platform | Self::Partner)
    }
}

/// Where a site's nodes live.
pub const SITE_KINDS: [&str; 3] = ["home", "cloud", "edge"];

/// Turn a display name into a slug: lower-case letters, digits and single dashes.
///
/// Returns `None` when nothing usable is left (a name of only symbols), because an empty
/// slug is not an address.
pub fn slugify(name: &str) -> Option<String> {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            dash = true;
        }
    }
    out.truncate(63);
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() { None } else { Some(out) }
}

/// Whether `slug` is acceptable as stored (the database enforces the same shape).
pub fn valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes[0] != b'-'
        && bytes[bytes.len() - 1] != b'-'
}

/// The role an invitation may carry. An invitation to one sandbox is always a member's:
/// anything higher would hand over more than the sandbox.
pub fn invitation_role(role: &str, for_sandbox: bool) -> Option<Role> {
    let role = Role::parse(role)?;
    if for_sandbox { Some(Role::Member) } else { Some(role) }
}

/// A sandbox grant is for a person who can work: a viewer cannot.
pub fn grantable(role: Role) -> bool {
    role >= Role::Member
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_a_slug() {
        assert_eq!(slugify("Acme GmbH & Co."), Some("acme-gmbh-co".into()));
        assert_eq!(slugify("  --Home  Lab--"), Some("home-lab".into()));
        assert_eq!(slugify("!!!"), None);
    }

    #[test]
    fn a_slug_is_checked_the_way_the_database_checks_it() {
        assert!(valid_slug("acme-1"));
        assert!(!valid_slug("-acme"));
        assert!(!valid_slug("Acme"));
        assert!(!valid_slug(""));
    }

    #[test]
    fn only_a_provider_may_have_customers() {
        assert!(OrganizationKind::Partner.may_have_children());
        assert!(!OrganizationKind::Customer.may_have_children());
    }

    #[test]
    fn an_invitation_to_a_sandbox_is_always_a_member() {
        assert_eq!(invitation_role("admin", true), Some(Role::Member));
        assert_eq!(invitation_role("operator", false), Some(Role::Operator));
        assert_eq!(invitation_role("root", false), None);
    }

    #[test]
    fn a_viewer_cannot_be_given_a_sandbox_to_work_in() {
        assert!(!grantable(Role::Viewer));
        assert!(grantable(Role::Member));
    }
}
