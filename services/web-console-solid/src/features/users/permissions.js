// Pure helpers for the Permissions tab (contract C13).

const ROLE_SENTENCES = {
  admin: 'Admin: everything, including users, providers, keys and global policy.',
  operator: 'Operator: manages their own sandboxes from start to end.',
  member: 'Member: works in their own sandboxes only.',
  viewer: 'Viewer: read-only on their own and shared sandboxes.',
};

// The roles a scope accepts; a sandbox only takes `member`, an organization takes none.
export function rolesFor(scopeType) {
  if (scopeType === 'sandbox') return ['member'];
  if (scopeType === 'organization') return [];
  return ['admin', 'operator', 'member', 'viewer'];
}

// A human label for where a role comes from.
export function viaLabel(via) {
  if (via === 'direct') return 'Direct';
  if (via === 'organization') return 'From organization';
  if (via === 'grant') return 'Sandbox grant';
  return via || 'Unknown';
}

// One sentence saying what a role may do.
export function roleSentence(role) {
  return ROLE_SENTENCES[role] || 'Unknown role.';
}

// Whether a row may be removed: only direct rows, never yourself, never the last admin.
export function canRemove(row, me, rows) {
  if (!row || row.via !== 'direct') return false;
  if (me && row.user_id === me) return false;
  if (row.role === 'admin') {
    const admins = (rows || []).filter((r) => r.role === 'admin');
    if (admins.length <= 1) return false;
  }
  return true;
}

// Whether a role of a row may be changed in place (direct rows only, not your own).
export function canChangeRole(row, me, scopeType) {
  return !!row && row.via === 'direct' && rolesFor(scopeType).length > 1 && !(me && row.user_id === me);
}

// Users that do not have a row yet, for the "Add person" select.
export function addableUsers(users, rows) {
  const have = new Set((rows || []).map((r) => r.user_id));
  return (users || []).filter((u) => !have.has(u.id));
}

// The sign-up link for an invitation token.
export function inviteLink(origin, token) {
  return `${origin}/?invite=${token}`;
}
