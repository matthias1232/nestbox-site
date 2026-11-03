// Sandbox resource.

import { request, query } from './client.js';

export function list(params = {}) {
  return request(`sandboxes${query(params)}`);
}

export function get(id) {
  return request(`sandboxes/${id}`);
}

export function create(body) {
  return request('sandboxes', { method: 'POST', body });
}

export function act(id, action) {
  return request(`sandboxes/${id}/actions`, { method: 'POST', body: { action } });
}

// Put a sandbox into a stack (parentId = its lead) or take it out (null).
export function setStack(id, parentId) {
  return request(`sandboxes/${id}/stack`, { method: 'PUT', body: { parent_id: parentId } });
}

export function remove(id) {
  return request(`sandboxes/${id}`, { method: 'DELETE' });
}

export function logs(id, tail = 200) {
  return request(`sandboxes/${id}/logs${query({ tail })}`);
}

export function inference(id, limit = 50) {
  return request(`sandboxes/${id}/inference${query({ limit })}`);
}

export function rules(id) {
  return request(`sandboxes/${id}/rules`);
}

// The agents inside a sandbox, and how they are started.
export function agents(id) {
  return request(`sandboxes/${id}/agents`);
}

// Add one agent. The body carries the pack, the instance name, where its home
// lives, whether that home survives a rebuild, and the port it serves.
export function addAgent(id, body) {
  return request(`sandboxes/${id}/agents`, { method: 'POST', body });
}

// Remove one agent. `keepPersistent` is part of the DELETE rather than a second
// call, because keeping or deleting the home is the same decision.
export function removeAgent(id, instance, keepPersistent) {
  return request(
    `sandboxes/${id}/agents/${encodeURIComponent(instance)}${query({ keep_persistent: keepPersistent })}`,
    { method: 'DELETE' },
  );
}

// Run one command in a sandbox. `command` is an argument vector; the console
// wraps a typed line in `bash -lc` so the operator can type what they would
// type in a terminal, and the vector shape makes that choice explicit.
export function exec(id, command, timeoutSeconds = 60) {
  return request(`sandboxes/${id}/exec`, {
    method: 'POST',
    body: { command, timeout_seconds: timeoutSeconds },
  });
}

// The terminal tabs this user keeps in the sandbox, and whether their shells are
// still alive. The list lives on the server so a reload reopens the same
// terminals on any machine, not whatever this browser last stored.
export function terminalSessions(id) {
  return request(`sandboxes/${id}/terminal-sessions`);
}

export function terminalTabs(id) {
  return request(`sandboxes/${id}/terminal-tabs`);
}

// Replace the tab set. A tab that is absent is closed.
export function saveTerminalTabs(id, tabs) {
  return request(`sandboxes/${id}/terminal-tabs`, {
    method: 'PUT',
    body: { tabs: tabs.map((tab) => ({ tab_id: tab.id, title: tab.label })) },
  });
}

// How to reach a sandbox from outside: the node's address and hostname, every
// published port with its dial address, the SSH key state, ready-made commands.
export function connect(id) {
  return request(`sandboxes/${id}/connect`);
}

// The private key, once. A second call answers 410 until the key is rotated.
export function downloadSshKey(id) {
  return request(`sandboxes/${id}/ssh-key/download`);
}

export function rotateSshKey(id) {
  return request(`sandboxes/${id}/ssh-key/rotate`, { method: 'POST' });
}

// The stack this sandbox belongs to, from the operator to the internet, with the
// live state of every piece. `windowSpan` is how far back egress traffic is summed.
export function stackTopology(id, windowSpan = '5m') {
  return request(`sandboxes/${id}/stack-topology${query({ window: windowSpan })}`);
}

// Who else may open the SSH door: operator keys and temporary logins.
export function sshAccess(id) {
  return request(`sandboxes/${id}/ssh-access`);
}

export function addUserKey(id, label, publicKey) {
  return request(`sandboxes/${id}/ssh-keys`, { method: 'POST', body: { label, public_key: publicKey } });
}

export function removeUserKey(id, keyId) {
  return request(`sandboxes/${id}/ssh-keys/${keyId}`, { method: 'DELETE' });
}

// The password is in this answer once and nowhere else.
export function issueCredential(id, minutes) {
  return request(`sandboxes/${id}/ssh-credentials`, { method: 'POST', body: { minutes } });
}

export function revokeCredential(id, credentialId) {
  return request(`sandboxes/${id}/ssh-credentials/${credentialId}`, { method: 'DELETE' });
}
