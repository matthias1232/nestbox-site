// Moving a sandbox and its folders to another node (ADR 0049).

import { request } from './client.js';

export const list = () => request('sandbox-moves');
export const get = (id) => request(`sandbox-moves/${id}`);
export const start = (sandboxId, node) =>
  request(`sandboxes/${sandboxId}/move`, { method: 'POST', body: { node } });

/** After the node is gone: restore from the newest scheduled snapshots on another node. */
export const recover = (sandboxId, node) =>
  request(`sandboxes/${sandboxId}/recover`, { method: 'POST', body: { node } });

/** Whether a move can work, said before anything stops. */
export const preflight = (sandboxId, node) =>
  request(`sandboxes/${sandboxId}/move-preflight?node=${encodeURIComponent(node)}`);
