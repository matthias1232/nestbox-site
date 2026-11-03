// One-click controls for a sandbox (contract C12).

import { request } from './client.js';

// `what`: allow_all | egress_mode | dns | internet | mcp | recording. Answers the control summary.
export function set(sandboxId, what, value, minutes = null) {
  return request(`sandboxes/${sandboxId}/quick`, { method: 'PUT', body: { what, value, minutes } });
}
