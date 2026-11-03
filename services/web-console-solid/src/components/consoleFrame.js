// Pure logic of the console frame: which modes a sandbox offers, font steps, SSH lines.

// Smallest and largest terminal font in pixels.
export const FONT_MIN = 11;
export const FONT_MAX = 20;
// The font size a terminal starts with.
export const FONT_DEFAULT = 13;

// The modes the split button offers. The desktop is offered for every sandbox
// (the stream itself explains a missing desktop pack), except when the sandbox
// says outright that it has none (`has_desktop === false`). SSH needs the
// right to see the sandbox; a viewer sees shell and desktop only when watching
// is allowed, so SSH is the one mode a viewer never gets.
export function modes(sandbox, role) {
  const out = ['shell'];
  if (!sandbox || sandbox.has_desktop !== false) out.push('desktop');
  if (role !== 'viewer') out.push('ssh');
  return out;
}

// The mode to show: the wanted one when it is offered, else the first offered.
export function pickMode(wanted, offered) {
  return offered.includes(wanted) ? wanted : offered[0];
}

// One font step (dir is +1 or -1), clamped to 11..20.
export function nextFont(size, dir) {
  const current = Number.isFinite(size) ? size : FONT_DEFAULT;
  return Math.min(FONT_MAX, Math.max(FONT_MIN, current + (dir > 0 ? 1 : -1)));
}

// The lines the SSH panel shows from the `connect` answer; null while unknown.
// Returns { command, config } where either may be null.
export function sshLines(connect) {
  if (!connect) return { command: null, config: null };
  const host = connect.host || connect.hostname || connect.address || null;
  const port = (connect.ports || []).find((p) => p.purpose === 'ssh')?.public_port || null;
  const user = connect.ssh_user || 'nestbox';
  const command = connect.ssh_command || (host && port ? `ssh ${user}@${host} -p ${port}` : null);
  const config = connect.ssh_config || (host && port
    ? `Host sandbox\n  HostName ${host}\n  Port ${port}\n  User ${user}\n  IdentityFile ~/.ssh/nestbox_key`
    : null);
  return { command, config };
}
