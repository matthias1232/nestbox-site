// An agent's sign-in mode: the user's own subscription or the gateway.
import { request } from './client.js';

// Every switchable agent of one sandbox, with what it is doing.
export function list(sandboxId) {
  return request(`sandboxes/${sandboxId}/agents/auth`);
}

// The same answer, named for the activity view.
export function activity(sandboxId) {
  return request(`sandboxes/${sandboxId}/agents/activity`);
}

// Switch one agent; `restart` is idle (default), now or later.
export function set(sandboxId, agent, mode, restart = 'idle') {
  return request(`sandboxes/${sandboxId}/agents/${agent}/auth`, {
    method: 'PUT',
    body: { mode, restart },
  });
}

// Restart one agent without changing its mode.
export function restart(sandboxId, agent, when = 'idle') {
  return request(`sandboxes/${sandboxId}/agents/${agent}/restart`, {
    method: 'POST',
    body: { when },
  });
}
