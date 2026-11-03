// Node summary, drain and quick deploy (contract C8).

import { request } from './client.js';

// One node with load, sandboxes, installs, images and tunnel.
export function summary(id) {
  return request(`nodes/${id}/summary`);
}

// Move or stop every sandbox of a node: `{ to_node, stop_only }`; answers `{ job_id }`.
export function drain(id, body) {
  return request(`nodes/${id}/drain`, { method: 'POST', body });
}

// Provision or join a node, optionally with a model; answers `{ job_id, join_command }`.
export function quick(body) {
  return request('nodes/quick', { method: 'POST', body });
}
