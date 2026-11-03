// Chats: model calls as turns, their groups, one turn, and the live stream.

import { request, query, environmentUrl } from './client.js';

// The turn list. Filters: sandbox_id, session_id, model, provider, dlp,
// account_kind, owner, q, since, until, before (next page), limit.
export const turns = (params = {}) => request(`chats/turns${query(params)}`);

// The same rows grouped. `by` is session, model, provider, day, sandbox or owner.
export const groups = (params = {}) => request(`chats/groups${query(params)}`);

// One turn with the findings of its request.
export const turn = (id) => request(`chats/turns/${id}`);

// Record that a live view was opened. The audit entry is what makes watching a
// visible act rather than a silent one.
export const liveWatch = (what) => request('live/watch', { method: 'POST', body: { what } });

// Turns starting, answers growing, turns stored. One connection for the page.
export function stream(handlers = {}) {
  const source = new EventSource(environmentUrl('/api/v1/chats/stream'));
  const read = (name, handler) => {
    if (!handler) return;
    source.addEventListener(name, (event) => {
      try {
        handler(JSON.parse(event.data));
      } catch {
        // A malformed frame is dropped rather than breaking the stream.
      }
    });
  };
  read('chat_started', handlers.onChatStarted);
  read('chat_delta', handlers.onChatDelta);
  read('chat_done', handlers.onChatDone);
  source.onerror = () => handlers.onError?.();
  return () => source.close();
}
