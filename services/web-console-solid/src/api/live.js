// The live agent situation: one snapshot and one stream.
//
// The snapshot is what makes a cold page show the agents that already exist;
// the stream is what makes it current. Both are opened here so no component
// ever names an endpoint.

import { request } from './client.js';

// The REST fallback for the stream's snapshot frame. It returns the same
// payload, so a caller can use it to refill a gap after a reconnect.
export function snapshot() {
  return request('live/agents/snapshot');
}

// Open the agent stream.
//
// EventSource cannot send headers, so the session cookie authenticates it; the
// stream works from a browser session, not from an API token. Automation reads
// `snapshot()` instead.
//
// Frames carry a `type` field. The server may also name them (`event: request`),
// so both the default message listener and the two named ones are attached; a
// single frame can only fire one of them, and a plain data frame still arrives.
export function stream(handlers) {
  const source = new EventSource(environmentUrl('/api/v1/live/agents'));

  const onFrame = (event) => {
    try {
      handlers.onFrame(JSON.parse(event.data));
    } catch {
      // A malformed frame is dropped rather than breaking the stream: one bad
      // frame must not cost the operator the rest of the incident.
    }
  };

  for (const name of ['message', 'request', 'snapshot']) {
    source.addEventListener(name, onFrame);
  }
  source.onopen = () => handlers.onOpen?.();
  source.onerror = () => handlers.onError?.();

  return () => source.close();
}
