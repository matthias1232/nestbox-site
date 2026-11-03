// Content privacy: the DLP policy, a single test of the filter, the incidents it
// records, and the admin purge.

import { request, query } from './client.js';

// The environment default (no sandbox id) or one sandbox's own settings.
// The answer carries both the stored settings and the detector catalog.
export const settings = (sandboxId) =>
  request(`privacy/settings${query({ sandbox_id: sandboxId })}`);

// Replaces the whole settings row, so the body always carries every field.
export const saveSettings = (body) => request('privacy/settings', { method: 'PUT', body });

// One test of the filter with the policy in force. It answers what would
// happen; the console never sends the content anywhere else.
export const inspect = (body) => request('privacy/inspect', { method: 'POST', body });

export const incidents = (sandboxId, limit = 50) =>
  request(`privacy/incidents${query({ sandbox_id: sandboxId, limit })}`);

// Admin: remove incident rows older than a day count.
export const purge = (days) => request('privacy/purge', { method: 'POST', body: { days } });

// The DLP dashboard: incidents over a window, grouped by detector, action,
// severity, sandbox, day or destination. An admin sees the environment; anyone
// else sees the sandboxes they own.
export const overview = ({ timeframe = '24h', group_by = 'detector', sandbox_id } = {}) =>
  request(`privacy/overview${query({ timeframe, group_by, sandbox_id })}`);

// The DLP log: the newest incidents with their masked excerpts, filterable.
export const log = ({ timeframe = '24h', sandbox_id, action, detector, q, limit = 100 } = {}) =>
  request(`privacy/log${query({ timeframe, sandbox_id, action, detector, q, limit })}`);

// Exceptions: a finding allowed on purpose, by value or by detector, for one
// sandbox or the environment, until a moment or forever.
export const exceptions = (sandboxId) =>
  request(`privacy/exceptions${query({ sandbox_id: sandboxId })}`);
export const createException = (body) =>
  request('privacy/exceptions', { method: 'POST', body });
export const deleteException = (id) =>
  request(`privacy/exceptions/${id}`, { method: 'DELETE' });

// Who changed this sandbox's policy (or the default it inherits), when, and from
// what to what. Read from the audit trail; no value a detector matched is in it.
export const changes = ({ sandbox_id, limit = 50 } = {}) =>
  request(`privacy/changes${query({ sandbox_id, limit })}`);
