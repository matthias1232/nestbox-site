// Gateways, supervisors, and topology.

import { request, query } from './client.js';

export function gateways() {
  return request('gateways');
}

export function gateway(id) {
  return request(`gateways/${id}`);
}

export function updateSettings(id, settings, version) {
  return request(`gateways/${id}/settings`, {
    method: 'PUT',
    body: { settings, version },
  });
}

export function sync(id) {
  return request(`gateways/${id}/sync`, { method: 'POST' });
}

export function supervisors() {
  return request('supervisors');
}

// `live` adds the egress edges of a window — including the destinations that
// were blocked — to the static graph. Without it the answer is unchanged.
export function topology(live) {
  return request(`topology${live ? query({ live }) : ''}`);
}

// The settings document of the supervisor that watches one sandbox.
export function supervisorSettings(sandboxId) {
  return request(`sandboxes/${sandboxId}/settings`);
}

// Replace that document. The sandbox receives it when it is running.
export function setSupervisorSettings(sandboxId, settings) {
  return request(`sandboxes/${sandboxId}/settings`, {
    method: 'PUT',
    body: settings,
  });
}

// Queue one whitelisted action for the sandbox's supervisor.
export function supervisorAction(sandboxId, kind, payload = {}) {
  return request(`sandboxes/${sandboxId}/supervisor/actions`, {
    method: 'POST',
    body: { kind, payload },
  });
}
