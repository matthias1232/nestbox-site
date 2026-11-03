// The only place that performs a request.
//
// Three things happen here and nowhere else:
//   1. the CSRF header is attached to every mutating request, because a missing
//      one is a 403 that looks like a random failure;
//   2. the active environment is sent, so the server scopes the query without
//      the client ever constructing a filter;
//   3. a failure becomes a typed error with the server's stable code, so a
//      screen can branch on `code` instead of matching an English message.

const BASE = '/api/v1';

let csrfToken = null;
let environmentId = null;

export class ApiError extends Error {
  constructor(code, message, status, details = null) {
    super(message);
    this.code = code;
    this.status = status;
    // The server's non-secret context, e.g. `{ sandboxes: ["a", "b"] }` on a
    // `provider.in_use`. A screen shows it instead of inventing an explanation,
    // which is why the error carries the whole object rather than a message.
    this.details = details;
  }
}

// Called when the server says the CSRF token is missing. The token lives in
// memory only, so a reload loses it; the session then asks for a fresh sign-in
// instead of showing a raw 403 on every action.
let onCsrfMissing = null;
export function setCsrfMissingHandler(handler) {
  onCsrfMissing = handler;
}

export function setCsrfToken(token) {
  csrfToken = token;
  // A pop-out window cannot ask for its own token without invalidating this one,
  // so it picks the token up from its opener through this property.
  try {
    window.__nestboxCsrf = token;
  } catch {
    /* a window without a document (a worker) has nothing to share */
  }
}

// Adopt the opener's CSRF token in a pop-out window; false when there is none.
export function adoptOpenerCsrf() {
  try {
    const token = window.opener && window.opener.__nestboxCsrf;
    if (token) {
      setCsrfToken(token);
      return true;
    }
  } catch {
    /* an opener from another origin */
  }
  return false;
}

export function setEnvironment(id) {
  environmentId = id;
  // A pop-out window opened the same page and needs the data center too; it
  // cannot ask the server which one the opener used without a second switch.
  try {
    window.__nestboxEnvironment = id;
  } catch {
    /* a window without a document has nothing to share */
  }
}

// Adopt the opener's data center in a pop-out window; false when there is none.
export function adoptOpenerEnvironment() {
  try {
    const id = window.opener && window.opener.__nestboxEnvironment;
    if (id) {
      setEnvironment(id);
      return id;
    }
  } catch {
    /* an opener from another origin */
  }
  return null;
}

// `url` with the active data center appended, for the requests the browser opens
// itself (WebSocket, EventSource, a download link): they send no headers.
export { withEnvironment } from './environmentUrl.js';

export function environmentUrl(url) {
  return withEnvironment(url, environmentId);
}

export function getEnvironment() {
  return environmentId;
}

export async function request(path, options = {}) {
  const method = (options.method || 'GET').toUpperCase();
  const headers = { 'content-type': 'application/json', ...(options.headers || {}) };

  if (method !== 'GET' && method !== 'HEAD' && csrfToken) {
    headers['x-csrf-token'] = csrfToken;
  }
  if (environmentId) {
    headers['x-nestbox-environment'] = environmentId;
  }

  const response = await fetch(`${BASE}/${path}`, {
    ...options,
    method,
    headers,
    credentials: 'same-origin',
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });

  if (response.status === 204) return null;

  const text = await response.text();
  const payload = text ? safeJson(text) : null;

  if (!response.ok) {
    const error = payload?.error || {};
    if (error.code === 'auth.csrf_required' && onCsrfMissing) onCsrfMissing();
    throw new ApiError(
      error.code || 'request.failed',
      error.message || `Request failed with ${response.status}`,
      response.status,
      error.details ?? null,
    );
  }
  return payload;
}

function safeJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// Build a query string from a filter object without leaking undefined values
// into the URL, which would otherwise send the literal string "undefined".
export function query(params) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === '') continue;
    search.set(key, value);
  }
  const text = search.toString();
  return text ? `?${text}` : '';
}
