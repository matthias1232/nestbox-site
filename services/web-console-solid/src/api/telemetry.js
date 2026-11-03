// Telemetry history.
//
// The live stream is not here on purpose: the app holds exactly one telemetry
// connection, in `state/live.jsx`, and every screen subscribes through
// `useLive()` (pattern J6). An `EventSource` opened from a page would be a second
// connection against the browser's six-per-host budget.

import { request, query } from './client.js';

export function history(params = {}) {
  return request(`telemetry/events${query(params)}`);
}

export function facets(minutes = 60) {
  return request(`telemetry/facets${query({ minutes })}`);
}

export function seedDemo() {
  return request('telemetry/seed-demo', { method: 'POST' });
}
