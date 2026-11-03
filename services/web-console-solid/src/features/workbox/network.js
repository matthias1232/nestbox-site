// The network vocabulary of one sandbox: the four proxy modes a user may pick,
// the durations an "allow everything" may last, and the sentences the banner
// and the dialogs are built from.
//
// Pure data and pure functions only, so the card that shows them can be read
// without a DOM and the copy that stands next to a dangerous switch can be
// tested (ADR 0052: a mode is switched live, so what the user reads before the
// click is the whole safety net).

// The four user-selectable proxy modes, in the order of least to most open.
// `direct` is missing on purpose: it takes the proxy out of the path and
// rebuilds the sandbox, which is an administrator's choice (contract C3).
export const MODES = [
  {
    value: 'transparent',
    label: 'Open everything',
    help: 'Every TLS connection is opened and recorded, chats included.',
  },
  {
    value: 'hybrid',
    label: 'Only agent chats',
    help: 'Model and agent connections are opened so chats are readable; everything else is tunnelled.',
  },
  {
    value: 'forward_proxy',
    label: 'Tunnel only',
    help: 'Nothing is opened. Host, port and size are recorded. Rules apply.',
  },
  {
    value: 'off_allow_all',
    label: 'Proxy off · allow everything',
    help: 'Nothing is blocked and nothing is opened. Only network connections are recorded.',
    tone: 'bad',
  },
];

// How long "allow everything" lasts; `minutes: null` means until switched off.
export const DURATIONS = [
  { minutes: 15, label: '15 min' },
  { minutes: 60, label: '1 h' },
  { minutes: 240, label: '4 h' },
  { minutes: 480, label: '8 h' },
  { minutes: null, label: 'Unlimited' },
];

// The firewalls the "Pause the rules" dialog can open, all ticked by default:
// a window that covers only half of what a sandbox needs is a support call.
export const CHANNELS = [
  { value: 'openshell', label: 'OpenShell' },
  { value: 'proxy', label: 'Network' },
  { value: 'dns', label: 'DNS' },
  { value: 'mcp', label: 'MCP' },
];

// How a channel is written inside the banner sentence. It is lowercase where it
// starts a list and a word, so "for network, DNS, MCP and OpenShell" reads as one
// sentence.
const CHANNEL_WORD = {
  all: 'everything',
  proxy: 'network',
  dns: 'DNS',
  mcp: 'MCP',
  openshell: 'OpenShell',
  gateway: 'the gateway',
};

const DAY_MS = 24 * 60 * 60 * 1000;

// A list as English: "a", "a and b", "a, b and c".
function joinWords(parts) {
  if (parts.length <= 1) return parts.join('');
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
}

// The label of one mode value, for a sentence after the click.
export function modeLabel(value) {
  const found = MODES.find((item) => item.value === value);
  return found ? found.label : value;
}

// The label of one window channel, for the list of open windows.
export function channelLabel(value) {
  return CHANNEL_WORD[value] || value;
}

// The window channels for the firewalls someone ticked. All four become one
// window for everything, because that is one decision and not four.
export function channelsFor(values) {
  const chosen = new Set(values || []);
  const named = CHANNELS.map((item) => item.value).filter((value) => chosen.has(value));
  if (named.length === CHANNELS.length) return ['all'];
  return named;
}

// The windows that still cover something now. A window with no expiry stays
// open until somebody closes it, so it is always open.
export function openWindows(control) {
  const now = Date.now();
  return (control?.windows || []).filter((entry) => !entry.expires_at || Date.parse(entry.expires_at) > now);
}

// A short English sentence for the loud banner, or null when nothing is
// allowed. Two different sentences, because the two situations are different:
// with the proxy off nothing is even seen, while an evaluation window still
// records what the rules would have blocked.
export function allowAllSentence(control) {
  if (!control) return null;
  if (control.egress?.mode === 'off_allow_all') {
    return 'Proxy is off: everything is allowed and only recorded.';
  }
  const windows = openWindows(control);
  // `allow_all` without a listed window is an older summary of the same fact:
  // say the general sentence rather than claiming nothing is allowed.
  if (!windows.length && !control.allow_all) return null;
  const whole = !windows.length || windows.some((entry) => !entry.channel || entry.channel === 'all');
  const covered = whole
    ? joinWords(['network', 'DNS', 'MCP', 'OpenShell'])
    : joinWords([...new Set(windows.map((entry) => CHANNEL_WORD[entry.channel] || entry.channel))]);
  return `Everything is allowed (evaluation mode) for ${covered}; every decision is recorded.`;
}

// When "allow everything" ends by itself; null when it lasts until somebody
// turns it off. The latest end wins, because the banner must not promise an end
// that another window outlives.
export function allowAllUntil(control) {
  if (!control) return null;
  if (control.allow_all_until) return control.allow_all_until;
  if (control.egress?.mode === 'off_allow_all') return control.egress.revert_at || null;
  const windows = openWindows(control);
  if (!windows.length) return null;
  if (windows.some((entry) => !entry.expires_at)) return null;
  return windows.map((entry) => entry.expires_at).sort().pop();
}

// What "Suggest rules" asks the API for: the start of the oldest open window,
// else the last 24 hours. A window row carries no start time today, so a
// summary without one falls back to the day.
export function draftSince(control, now = new Date()) {
  const starts = openWindows(control)
    .map((entry) => Date.parse(entry.opened_at || ''))
    .filter((time) => Number.isFinite(time));
  const since = starts.length ? Math.min(...starts) : now.getTime() - DAY_MS;
  return new Date(since).toISOString();
}
