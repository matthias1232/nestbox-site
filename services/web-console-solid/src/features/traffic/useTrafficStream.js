// Live telemetry state: the last window from history, then the shared stream on top.
//
// Two sources, one list. Opening the page reads the window that already
// happened and then appends what arrives, so an idle sandbox does not look like
// a broken page and a reload shows the minutes before it. Rows are normalised to
// one shape because the stream carries less than the stored row: a live event
// knows the destination and the decision, while the stored row also knows the
// method, the path and the MCP tool.
//
// The stream is the app's one telemetry connection (`state/live.jsx`): this hook
// asks for it while the page is mounted and hands it back when the page leaves.
//
// The buffer is capped: a live console that grows without bound is a memory leak
// that shows up an hour into an incident, which is the worst possible moment.

import { createSignal, onCleanup } from 'solid-js';
import { useLive } from '../../state/live.jsx';
import * as trafficApi from '../../api/traffic.js';
import * as sandboxesApi from '../../api/sandboxes.js';

/** Rows kept in view. The window is the real bound; this is the render cap. */
const MAX_ROWS = 500;

/** Windows the page offers. `24h` is the server's cap for the row list. */
export const WINDOWS = ['5m', '15m', '1h', '6h', '24h'];

/** A decision filter, as the API spells it. */
export const DECISIONS = [
  { id: '', label: 'All' },
  { id: 'blocked', label: 'Blocked' },
  { id: 'allowed', label: 'Allowed' },
];

/** The channel chips. `mcp` is a protocol, not an event kind. */
export const KINDS = [
  { id: '', label: 'Everything' },
  { id: 'network', label: 'Network' },
  { id: 'dns', label: 'DNS' },
  { id: 'mcp', label: 'MCP' },
];

/**
 * One row, whatever produced it.
 *
 * `source` is the only field that differs on purpose: `live` rows come from the
 * stream and are coarser than `stored` rows, which the table says rather than
 * hiding behind an empty cell that looks like a bug.
 */
function stored(item, names) {
  return {
    id: item.id,
    ts: item.ts,
    sandboxId: item.sandbox_id,
    sandbox: item.sandbox_name || names[item.sandbox_id] || 'unknown sandbox',
    kind: item.protocol === 'dns' ? 'dns' : item.app_protocol === 'mcp' ? 'mcp' : 'network',
    decision: item.decision,
    host: item.host,
    port: item.port,
    protocol: item.protocol,
    method: item.method,
    path: item.path,
    status: item.status,
    tool: item.mcp_tool,
    channel: item.channel,
    reason: item.reason_text || item.reason,
    binary: item.binary || item.agent_hint,
    country: item.country,
    source: 'stored',
  };
}

function liveRow(event, names) {
  const isDns = event.kind === 'dns';
  return {
    id: event.id,
    ts: event.ts,
    sandboxId: event.sandbox_id,
    sandbox: names[event.sandbox_id] || 'unknown sandbox',
    kind: isDns ? 'dns' : event.app_protocol === 'mcp' ? 'mcp' : 'network',
    decision: event.decision,
    host: isDns ? event.hostname : event.destination || event.host,
    port: isDns ? null : event.port,
    protocol: isDns ? 'dns' : event.protocol || 'tcp',
    method: null,
    path: null,
    status: null,
    tool: null,
    // A DNS event carries no channel on the wire; its kind is the channel.
    channel: isDns ? 'dns' : event.channel,
    reason: event.reason || (event.rule_name ? `rule ${event.rule_name}` : null),
    binary: event.binary,
    country: null,
    source: 'live',
  };
}

/** Whether a row passes the chips and the search box. */
function matches(row, { kind, decision, host, sandbox }) {
  if (kind && row.kind !== kind) return false;
  if (decision === 'blocked' && row.decision !== 'deny') return false;
  if (decision === 'allowed' && row.decision === 'deny') return false;
  if (sandbox && row.sandboxId !== sandbox) return false;
  if (host) {
    const needle = host.toLowerCase();
    const haystack = `${row.host} ${row.sandbox} ${row.tool || ''} ${row.reason || ''}`.toLowerCase();
    if (!haystack.includes(needle)) return false;
  }
  return true;
}

export function useTrafficStream() {
  const live = useLive();
  const [rows, setRows] = createSignal([]);
  const [names, setNames] = createSignal({});
  const [windowSpan, setWindowSpan] = createSignal('15m');
  const [kind, setKind] = createSignal('');
  const [decision, setDecision] = createSignal('');
  const [host, setHost] = createSignal('');
  const [sandbox, setSandbox] = createSignal('');
  const [summary, setSummary] = createSignal(null);
  const [paused, setPaused] = createSignal(false);
  const [pending, setPending] = createSignal([]);
  const [connected, setConnected] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal(null);

  let release = null;
  let stops = [];
  /** Ids already in view, so a repeated frame or a re-read cannot duplicate. */
  let seen = new Set();

  function setAll(next) {
    seen = new Set(next.map((row) => row.id));
    setRows(next.slice(0, MAX_ROWS));
  }

  function append(row) {
    if (seen.has(row.id)) return;
    seen.add(row.id);
    setRows((current) => [row, ...current].slice(0, MAX_ROWS));
  }

  /** Read the window that already happened, and the names for the stream. */
  async function load() {
    setLoading(true);
    setError(null);
    try {
      const [page, list] = await Promise.all([
        trafficApi.environment({
          window: windowSpan(),
          filter: decision() || 'all',
          host: host(),
          sandbox: sandbox(),
          limit: 200,
        }),
        sandboxesApi.list({ limit: 200 }),
      ]);
      const map = {};
      for (const entry of list.items || []) {
        const sandboxRow = entry.sandbox || entry;
        if (sandboxRow.id) map[sandboxRow.id] = sandboxRow.name;
      }
      setNames(map);
      setSummary(page.summary || null);
      setAll((page.items || []).map((item) => stored(item, map)));
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoading(false);
    }
  }

  /**
   * Subscribe to the app's telemetry stream. Safe to call twice; the second call
   * is a no-op. The stream itself is the one in `state/live.jsx`, opened while
   * this page (or any other screen) holds a request on it.
   */
  function start() {
    if (release) return;
    release = live.wantTelemetry();
    const handle = (event) => {
      const row = liveRow(event, names());
      // While paused the rows are held aside, so resuming shows what arrived.
      if (paused()) {
        setPending((current) => [row, ...current].slice(0, MAX_ROWS));
        return;
      }
      append(row);
    };
    stops = [live.on('network', handle), live.on('dns', handle), live.on('inference', handle)];
    setConnected(true);
  }

  function stop() {
    for (const halt of stops) halt();
    stops = [];
    release?.();
    release = null;
    setConnected(false);
  }

  function togglePause() {
    if (paused()) {
      // Flush what arrived while paused, oldest first so the order is preserved.
      const buffered = [...pending()].reverse();
      buffered.forEach(append);
      setPending([]);
      setPaused(false);
    } else {
      setPaused(true);
    }
  }

  function clear() {
    setAll([]);
    setPending([]);
  }

  /** Apply a facet that the server filters on, then re-read the window. */
  function applyDecision(next) {
    setDecision(next);
    load();
  }

  function applySandbox(next) {
    setSandbox(next);
    load();
  }

  const visible = () =>
    rows().filter((row) =>
      matches(row, { kind: kind(), decision: decision(), host: host(), sandbox: sandbox() }),
    );

  const counts = () => {
    const result = { network: 0, dns: 0, mcp: 0, blocked: 0 };
    for (const row of rows()) {
      if (result[row.kind] !== undefined) result[row.kind] += 1;
      if (row.decision === 'deny') result.blocked += 1;
    }
    return result;
  };

  onCleanup(stop);

  return {
    rows, visible, counts, summary, names,
    windowSpan, setWindowSpan,
    kind, setKind,
    decision, applyDecision,
    host, setHost,
    sandbox, applySandbox,
    paused, pending, connected, loading, error,
    start, stop, load, togglePause, clear,
    setError,
  };
}
