// The inventory tree (contract C3).

import { request, query } from './client.js';

// The tree for one view: `infrastructure`, `workloads` or `models`.
export function tree(view = 'infrastructure') {
  return request(`inventory/tree${query({ view })}`);
}
