// Refused destinations nobody answered yet (contract C9).

import { request, query } from './client.js';

// `params`: { window: '15m', sandbox }.
export function list(params = {}) {
  return request(`alerts${query(params)}`);
}
