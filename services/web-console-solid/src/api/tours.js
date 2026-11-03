// First-run tours: mark one as seen for the signed-in user.

import { request } from './client.js';

// Remember that this user finished the tour.
export function markSeen(tour) {
  return request(`users/me/tours/${encodeURIComponent(tour)}`, { method: 'PUT' });
}
