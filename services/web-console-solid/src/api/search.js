// Global search over sandboxes, nodes, models and more (contract C4).

import { request, query } from './client.js';

// `params`: { q, kind, environment, status, limit }; `kind` is a comma list.
export function search(params) {
  return request(`search${query(params)}`);
}
