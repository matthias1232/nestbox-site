// The appliance's own containers: state, live use, limits, and the two writes.
//
// Kept apart from `cluster.js` on purpose. A gateway and a supervisor belong to
// the sandbox story (who enforces policy); these rows are the platform the
// console itself runs on, and only an admin changes them.

import { request } from './client.js';

/** Every platform container with live use and the limits it runs under. */
export function components() {
  return request('system/components');
}

/**
 * The appliance's readiness answer: `{ status, disk_free_bytes }`.
 *
 * `disk_free_bytes` is `null` when the host did not answer in time, so a caller
 * that wants to warn about a filling disk must treat `null` as "unknown", not
 * as "no space". The route is public, which is why the home page can read it
 * without a signed-in session.
 */
export function ready() {
  return request('readyz');
}

/** One component's last log lines. */
export function logs(service, tail = 200) {
  return request(`system/components/${encodeURIComponent(service)}/logs?tail=${tail}`);
}

/**
 * The same read, under the name the topology and log screens use.
 *
 * Two names for one route is deliberate: `logs` reads as "the component's log",
 * `componentLogs` as "the log of a card I selected". Both call the one route, so
 * the screens cannot drift onto two different sources.
 */
export function componentLogs(service, tail = 200) {
  return logs(service, tail);
}

/**
 * Change CPU, memory or process limits. Docker applies them live; the answer
 * does not claim a restart that did not happen.
 */
export function setLimits(service, body) {
  return request(`system/components/${encodeURIComponent(service)}/limits`, {
    method: 'PUT',
    body,
  });
}

/** Restart one container. The server refuses the ones that would cut the request. */
export function restart(service) {
  return request(`system/components/${encodeURIComponent(service)}/restart`, { method: 'POST' });
}
