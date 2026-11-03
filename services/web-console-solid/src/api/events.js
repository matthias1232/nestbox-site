import { environmentUrl } from './client.js';

// Opens server-sent event streams for the shared live store (`state/live.jsx`).
//
// This is the only place the shared streams are opened, so the store never touches the
// browser API itself, and a test (`streams.test.js`) can prove that no page or feature
// names an `EventSource`. `chats.js` and `live.js` still open their own streams; those
// belong to their own screens and move here in a later package.

// Open `path` and call `onEvent(name, parsed)` for each of `names`. Returns a function that
// closes the stream. `hooks.onOpen` and `hooks.onError` report the connection state.
export function openStream(path, names, onEvent, hooks = {}) {
  const source = new EventSource(environmentUrl(path));
  for (const name of names) {
    source.addEventListener(name, (event) => {
      try {
        onEvent(name, JSON.parse(event.data));
      } catch {
        // A malformed frame is dropped rather than breaking the stream.
      }
    });
  }
  source.onopen = () => hooks.onOpen?.();
  source.onerror = () => hooks.onError?.();
  return () => source.close();
}
