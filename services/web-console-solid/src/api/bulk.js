// Bulk actions on sandboxes as one job (contract C5).

import { request } from './client.js';

// Apply `action` to every id; the answer is `{ job_id, total }`.
export function sandboxes(ids, action, params = {}) {
  return request('sandboxes/bulk', { method: 'POST', body: { ids, action, params } });
}
