// The environment-wide traffic list: every sandbox in one window.
//
// The per-sandbox read lives in `deck.js` because it belongs to a sandbox's
// tabs. This one belongs to the Live traffic page, which asks about the whole
// environment at once.

import { request, query } from './client.js';

export function environment(params = {}) {
  return request(`traffic${query(params)}`);
}

// The denied destinations, grouped by what a person can allow.
export function decisions(params = {}) {
  return request(`traffic/decisions${query(params)}`);
}
