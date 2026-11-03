// The one call the workbox needs, and the summary the sandbox list shows.
import { request } from './client.js';

// Everything the workbox shows for one sandbox: modes, windows, agents, people, models.
export function summary(sandboxId) {
  return request(`sandboxes/${sandboxId}/control`);
}

// The cheap per-sandbox summary for every visible sandbox (the admin list).
export function overview() {
  return request('sandboxes/control-overview');
}
