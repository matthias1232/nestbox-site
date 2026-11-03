// Who is connected to a sandbox, and the terminal tabs everybody shares.
import { request, query } from './client.js';

// Everybody connected now: terminals, SSH logins, desktop viewers.
export function presence(sandboxId) {
  return request(`sandboxes/${sandboxId}/presence`);
}

// Every user's terminal tabs of this sandbox, with who is in which tab.
export function tabsAll(sandboxId) {
  return request(`sandboxes/${sandboxId}/terminal-tabs${query({ all: 1 })}`);
}

// Close one tab for everyone: the tmux session ends and every viewer leaves.
export function closeTabForAll(sandboxId, tab) {
  return request(`sandboxes/${sandboxId}/terminal-tabs/${encodeURIComponent(tab)}`, {
    method: 'DELETE',
  });
}
