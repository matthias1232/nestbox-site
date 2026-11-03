// The data center a browser-opened URL must name for itself.
//
// A WebSocket upgrade and an `EventSource` request cannot carry a header, so the
// terminal, the desktop, the live streams and a pop-out window carry the chosen
// data center in `?environment=`. The server applies the same membership check it
// applies to the header (U01). Pure, so a test proves the joins.
export function withEnvironment(url, environmentId) {
  if (!environmentId) return url;
  const separator = url.includes('?') ? '&' : '?';
  return `${url}${separator}environment=${encodeURIComponent(environmentId)}`;
}

// Read the environment a link was opened with; null when it names none.
export function environmentFromUrl(search) {
  return new URLSearchParams(search || '').get('environment');
}
