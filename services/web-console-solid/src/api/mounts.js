// The effective mounts of a sandbox (contract C10).

import { request } from './client.js';

// What the sandbox has mounted and could mount.
export function get(sandboxId) {
  return request(`sandboxes/${sandboxId}/mounts`);
}

// Make the mount set equal `body`: `{ roots: [{ root_id, mount_path, read_only }], apply: 'now' | 'later' }`.
export function put(sandboxId, body) {
  return request(`sandboxes/${sandboxId}/mounts`, { method: 'PUT', body });
}
