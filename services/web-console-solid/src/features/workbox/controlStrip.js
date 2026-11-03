// The pure logic of the one-click control strip, the live log and the "rules from
// the log" card of the Workbox. No JSX, no state, no API calls, so the copy that
// stands next to a switch and the way an event becomes a log line can be tested.

import { allowAllUntil, openWindows } from './network.js';

// Allow-everything choices, in display order; `minutes: null` means until switched off.
export const ALLOW_ALL_CHOICES = [
  { value: 'off', label: 'Off', minutes: null },
  { value: '15', label: '15 min', minutes: 15 },
  { value: '60', label: '1 h', minutes: 60 },
  { value: 'forever', label: 'Until I stop', minutes: null },
];

// Proxy modes of the strip, in the order of the package text.
export const MODE_CHOICES = [
  { value: 'forward_proxy', label: 'Forward' },
  { value: 'transparent', label: 'Transparent' },
  { value: 'hybrid', label: 'Hybrid' },
  { value: 'off_allow_all', label: 'Off', tone: 'bad' },
];

// What the recorder keeps; the sentence says what each choice stores.
export const RECORDING_CHOICES = [
  { value: 'full', label: 'Full', help: 'store flows, bodies and chats' },
  { value: 'flows', label: 'Flows', help: 'store flows only' },
  { value: 'off', label: 'Off', help: 'store nothing (live view still works)' },
];

// How long a channel switch lasts.
export const FOR_CHOICES = [
  { minutes: 15, label: '15 min' },
  { minutes: 60, label: '1 h' },
  { minutes: 1440, label: '24 h' },
];

// The three channel switches; `window` is the traffic-window channel behind each.
export const CHANNEL_SWITCHES = [
  { key: 'internet', label: 'Internet', window: 'proxy' },
  { key: 'dns', label: 'DNS', window: 'dns' },
  { key: 'mcp', label: 'MCP', window: 'mcp' },
];

// Whether an ISO time lies in the future (null and unparsable are not).
function future(iso, now) {
  const time = Date.parse(iso || '');
  return Number.isFinite(time) && time > now;
}

// The state of one channel: open when a window covers it (or everything), with the
// latest end among the covering windows, or null when one has no end.
function channelState(windows, channel) {
  const covering = windows.filter(
    (entry) => !entry.channel || entry.channel === 'all' || entry.channel === channel,
  );
  if (!covering.length) return { on: false, until: null };
  if (covering.some((entry) => !entry.expires_at)) return { on: true, until: null };
  return { on: true, until: covering.map((entry) => entry.expires_at).sort().pop() };
}

// What the strip shows for one control summary (contract C12).
export function stripState(summary, now = Date.now()) {
  const control = summary || {};
  const windows = openWindows(control);
  const wholeWindows = windows.filter((entry) => !entry.channel || entry.channel === 'all');
  const proxyOff = control.egress?.mode === 'off_allow_all';
  const on = Boolean(control.allow_all) || wholeWindows.length > 0 || proxyOff;
  let recording = control.recording;
  if (!['full', 'flows', 'off'].includes(recording)) {
    if (future(control.telemetry_mute_until, now)) recording = 'off';
    else if (future(control.telemetry_mute_bodies_until, now)) recording = 'flows';
    else recording = 'full';
  }
  return {
    allowAll: { on, until: on ? allowAllUntil(control) : null },
    mode: control.egress?.mode || 'forward_proxy',
    recording,
    channels: {
      dns: channelState(windows, 'dns'),
      internet: channelState(windows, 'proxy'),
      mcp: channelState(windows, 'mcp'),
    },
  };
}

// The words next to a control: the label of a choice, or the recording sentence.
export function labelFor(group, value) {
  const lists = {
    allowAll: ALLOW_ALL_CHOICES,
    mode: MODE_CHOICES,
    recording: RECORDING_CHOICES,
  };
  const found = (lists[group] || []).find((item) => item.value === value);
  if (!found) return String(value ?? '');
  if (group === 'recording') return `${found.label} — ${found.help}`;
  return found.label;
}

// The quick-control call for an allow-everything click. `choice` is one of the
// ALLOW_ALL_CHOICES values; without one, a running allow-all is ended and a quiet
// sandbox gets 15 minutes. Answers `{ what, value, minutes }` for `quick.set`.
export function nextAllowAllAction(state, choice = null) {
  const picked = choice || (state?.allowAll?.on ? 'off' : '15');
  const found = ALLOW_ALL_CHOICES.find((item) => item.value === picked);
  if (!found || found.value === 'off') return { what: 'allow_all', value: false, minutes: null };
  return { what: 'allow_all', value: true, minutes: found.minutes };
}

// The choice that should look pressed: the one the user made last while it still
// holds, an unbounded allow-all as "Until I stop", else nothing.
export function allowAllChoice(state, lastChoice = null) {
  if (!state?.allowAll?.on) return 'off';
  if (lastChoice && lastChoice !== 'off') return lastChoice;
  return state.allowAll.until == null ? 'forever' : null;
}

// Whether the caller may use the switches: operators and admins always, anyone else
// only on their own box. An unknown owner does not lock the strip (the server decides).
export function mayControl(role, ownerId, userId) {
  if (role === 'operator' || role === 'admin') return true;
  if (!ownerId || !userId) return true;
  return ownerId === userId;
}

// The quick-control call for a recording choice: the mute lasts a day.
export function recordingAction(value) {
  return { what: 'recording', value, minutes: value === 'full' ? null : 1440 };
}

// The quick-control call for a channel switch.
export function channelAction(key, on, minutes) {
  return { what: key, value: Boolean(on), minutes: on ? minutes : null };
}

// ---- live log ---------------------------------------------------------------

// Filter chips of the live log.
export const LOG_FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'refused', label: 'Refused' },
  { id: 'dns', label: 'DNS' },
  { id: 'inference', label: 'Inference' },
];

// How many log lines the card keeps; older ones fall off the end.
export const LOG_KEEP = 300;

// One telemetry event as a log line, or null when it belongs to another sandbox.
// `kind` is the stream name (`network`, `dns`, `inference`).
export function logLine(kind, event, sandboxId) {
  if (!event || typeof event !== 'object') return null;
  const data = event.event && typeof event.event === 'object' ? event.event : event;
  if (data.sandbox_id !== sandboxId) return null;
  const decision = data.decision || 'allow';
  let text;
  if (kind === 'dns') text = data.hostname || '';
  else if (kind === 'inference') {
    text = [data.model_key || data.endpoint || 'model request', data.status_code].filter(Boolean).join(' · ');
  } else {
    const host = data.sni || data.destination || '';
    text = data.port ? `${host}:${data.port}` : host;
  }
  return {
    key: data.id || `${kind}-${data.ts}-${text}`,
    ts: data.ts || new Date().toISOString(),
    kind,
    decision,
    refused: decision === 'deny',
    text,
    binary: data.binary || null,
    protocol: data.protocol || data.app_protocol || null,
  };
}

// New lines on top, bounded, a repeated key replaced rather than doubled.
export function addLine(lines, line) {
  if (!line) return lines;
  return [line, ...lines.filter((item) => item.key !== line.key)].slice(0, LOG_KEEP);
}

// The lines one chip shows.
export function filterLines(lines, filter) {
  if (filter === 'refused') return lines.filter((line) => line.refused);
  if (filter === 'dns') return lines.filter((line) => line.kind === 'dns');
  if (filter === 'inference') return lines.filter((line) => line.kind === 'inference');
  return lines;
}

// The red lane: alerts of this sandbox, one per id (the latest wins), newest first,
// without the ones the user ignored.
export function laneAlerts(alerts, sandboxId, isIgnored = () => false) {
  const byId = new Map();
  for (const alert of alerts || []) {
    if (!alert || alert.sandbox_id !== sandboxId || isIgnored(alert.id)) continue;
    byId.set(alert.id, alert);
  }
  return [...byId.values()].sort(
    (left, right) => Date.parse(right.last_at || '') - Date.parse(left.last_at || ''),
  );
}

// ---- rules from the log -----------------------------------------------------

// The distinct destinations in a traffic answer and how many would have been refused.
export function summarizeTraffic(items) {
  const seen = new Set();
  const refused = new Set();
  for (const item of Array.isArray(items) ? items : []) {
    const name = item.host || item.sni || item.destination || item.hostname;
    if (!name) continue;
    const key = `${name}:${item.port ?? ''}`;
    seen.add(key);
    if (item.decision === 'deny' || item.would_deny) refused.add(key);
  }
  return { seen: seen.size, refused: refused.size };
}

// The window name the traffic API accepts for a start time.
export function windowFor(sinceIso, now = Date.now()) {
  const age = now - Date.parse(sinceIso || '');
  if (!Number.isFinite(age) || age > 60 * 60 * 1000) return '24h';
  return age > 15 * 60 * 1000 ? '1h' : '15m';
}

// The sentence of the card.
export function rulesSentence(summary, since, allowAllOn) {
  const time = new Date(since).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const lead = allowAllOn ? `Since allow-all started at ${time}` : 'In the last 24 hours';
  const noun = summary.seen === 1 ? 'destination was' : 'destinations were';
  return `${lead}, ${summary.seen} ${noun} seen, ${summary.refused} would have been refused.`;
}

// The toast text after the bulk request.
export function draftsSentence(result) {
  const created = result?.created ?? (result?.drafts || []).length;
  return `${created} ${created === 1 ? 'draft' : 'drafts'} created, review them`;
}

// ---- models card ------------------------------------------------------------

// The grants a user holds whose model this sandbox does not list yet, one per model
// name, so the "Add model" select offers each model once.
export function notYetGranted(grants, current) {
  const have = new Set((current || []).map((row) => row.model_name));
  const seen = new Set();
  const out = [];
  for (const grant of grants || []) {
    if (!grant?.model_name || have.has(grant.model_name) || seen.has(grant.model_name)) continue;
    seen.add(grant.model_name);
    out.push(grant);
  }
  return out;
}
