// The monitor layout of a desktop sandbox: which monitors exist, how large each
// one is, where it sits, and whether they are separate screens or one stretched
// screen (U10a).
//
// `get` reads what the sandbox's screen is now; `set` replaces it. A change can
// lose a race with an agent or another operator working on the same desktop, so
// `set` carries the `applied_at` the caller read: the server answers `409
// display.changed` with the layout that is really there instead of overwriting a
// change somebody else just made.

import { request } from './client.js';

export function get(sandboxId) {
  return request(`sandboxes/${sandboxId}/display`);
}

// One layout. A call without a stamp means "replace whatever is there".
export function set(sandboxId, layout, expectedAppliedAt) {
  return request(`sandboxes/${sandboxId}/display`, {
    method: 'PUT',
    body: displayBody(layout, expectedAppliedAt),
  });
}

// The body of a write, without a stamp key when there is none.
//
// Pure, so the shape the server expects is tested without a request: an empty
// string is no stamp, not a stamp of "".
export function displayBody(layout, expectedAppliedAt) {
  const body = { layout };
  if (expectedAppliedAt) body.expected_applied_at = expectedAppliedAt;
  return body;
}
