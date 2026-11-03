// How a chat turn reads on screen: times, token counts, models, tool calls and
// the one-line summary at the top of a thread.
//
// Pure, so a test can pin the wording without a browser and a screenshot cannot
// disagree with the export.

const PAD = (value) => String(value).padStart(2, '0');

/** A timestamp as `YYYY-MM-DD HH:MM` in the reader's time zone, or an em dash. */
export function formatTime(ts) {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '—';
  return `${date.getFullYear()}-${PAD(date.getMonth() + 1)}-${PAD(date.getDate())} ${PAD(date.getHours())}:${PAD(date.getMinutes())}`;
}

/** The clock only, for a bubble's footer. */
export function formatClock(ts) {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '—';
  return `${PAD(date.getHours())}:${PAD(date.getMinutes())}`;
}

/** The day only, the key the server groups by. */
export function formatDay(ts) {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '—';
  return `${date.getFullYear()}-${PAD(date.getMonth() + 1)}-${PAD(date.getDate())}`;
}

/** A count with dots as thousands separators: `12.480`. */
export function formatTokens(value) {
  const number = Math.round(Number(value) || 0);
  return String(number).replace(/\B(?=(\d{3})+(?!\d))/g, '.');
}

/** `1.204 ein → 318 aus · 96 im Cache`, the line under an answer. */
export function tokenLine(turn) {
  if (!turn) return '';
  const flow = `${formatTokens(turn.input_tokens)} ein → ${formatTokens(turn.output_tokens)} aus`;
  const cache = [];
  if (Number(turn.cache_read) > 0) cache.push(`${formatTokens(turn.cache_read)} im Cache`);
  if (Number(turn.cache_write) > 0) cache.push(`${formatTokens(turn.cache_write)} geschrieben`);
  return cache.length ? `${flow} · ${cache.join(' · ')}` : flow;
}

/** The model a turn was served by, falling back through the router's answer. */
export function modelLabel(turn) {
  return (
    turn?.served_model ||
    turn?.requested_model ||
    turn?.router_model ||
    turn?.model ||
    '—'
  );
}

/** The provider a turn went to, or the route's last hop. */
export function providerLabel(turn) {
  return turn?.provider || turn?.router_provider || '—';
}

/**
 * The tool calls of a turn as `{ name, input }`.
 *
 * A stored turn carries the provider's own JSON, which differs per provider; a
 * live turn carries nothing. Both become one shape here, so the card and the
 * export agree.
 */
export function normaliseToolCalls(value) {
  const list = Array.isArray(value) ? value : value?.calls;
  if (!Array.isArray(list)) return [];
  return list.map((item) => {
    if (typeof item === 'string') return { name: item, input: null };
    const name = item?.name || item?.function?.name || item?.tool || 'tool';
    const input = item?.input ?? item?.arguments ?? item?.function?.arguments ?? null;
    return { name: String(name), input: input ?? null };
  });
}

/** One tool call as one monospace line. */
export function toolText(call) {
  const input = call?.input;
  if (input === null || input === undefined) return String(call?.name || 'tool');
  const body = typeof input === 'string' ? input : JSON.stringify(input);
  return `${call.name} ${body}`;
}

function unique(values) {
  const seen = new Set();
  for (const value of values) {
    if (value === null || value === undefined || value === '') continue;
    seen.add(String(value));
  }
  return [...seen];
}

/**
 * The head of a thread: who, where, with which account and which models.
 *
 * A thread is a range of turns, not a session object, so everything the header
 * says is derived from the turns it shows. Nothing here is invented.
 */
export function threadHeader(turns = []) {
  const stamps = turns.map((turn) => Date.parse(turn?.ts)).filter((at) => Number.isFinite(at));
  return {
    count: turns.length,
    sandboxes: unique(turns.map((turn) => turn?.sandbox_name || turn?.sandbox_id)),
    users: unique(turns.map((turn) => turn?.owner_name || turn?.owner_user_id)),
    agents: unique(turns.map((turn) => turn?.agent)),
    accounts: unique(turns.map((turn) => turn?.account_kind)),
    models: unique(turns.map(modelLabel)),
    providers: unique(turns.map((turn) => turn?.provider)),
    first: stamps.length ? new Date(Math.min(...stamps)).toISOString() : null,
    last: stamps.length ? new Date(Math.max(...stamps)).toISOString() : null,
  };
}

/**
 * The content-check counters of one group, in the order they are shown.
 *
 * Zero stays visible on purpose: "nothing was blocked here" is an answer the
 * reader is looking for, and a hidden zero looks like a missing column.
 */
export function groupCounters(group) {
  return [
    { state: 'blocked', label: 'blockiert', value: Number(group?.blocked) || 0 },
    { state: 'replaced', label: 'ersetzt', value: Number(group?.replaced) || 0 },
    { state: 'pseudonymized', label: 'pseudonymisiert', value: Number(group?.pseudonymized) || 0 },
  ];
}
