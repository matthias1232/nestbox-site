// Opens server-sent event streams. The one place besides telemetry.js, live.js and chats.js
// that names an EventSource, so the shared live store never touches the browser API itself.

// Open `path` and call `onEvent(name, parsed)` for each of `names`. Returns a function that
// closes the stream. `hooks.onOpen` and `hooks.onError` report the connection state.
export function openStream(path, names, onEvent, hooks = {}) {
  const source = new EventSource(path);
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
