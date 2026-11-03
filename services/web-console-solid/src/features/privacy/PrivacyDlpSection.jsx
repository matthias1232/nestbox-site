// The DLP section of the Content privacy page, and of a sandbox.
//
// One table for the findings, with the colours and the columns an operator asks
// for: what happened, to which detector, in which sandbox, from which person,
// through which model to which address — and the masked value that was replaced.
// Everything is sortable, filterable, searchable, groupable and exportable,
// because a list of findings is only useful if a reader can narrow it.
import { For, Show, createMemo, createSignal } from 'solid-js';
import * as privacy from '../../api/privacy.js';
import DataTable from '../../components/DataTable.jsx';
import DlpBadge from '../../components/DlpBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import DlpFindingDrawer from './DlpFindingDrawer.jsx';
import DlpExceptionsTable from './DlpExceptionsTable.jsx';

const FRAMES = ['1h', '24h', '7d', '30d'];
const VIEWS = [
  { id: 'events', label: 'Findings' },
  { id: 'changes', label: 'Changes' },
  { id: 'exceptions', label: 'Exceptions' },
];
const STATES = [
  { value: 'blocked', label: 'Blocked' },
  { value: 'replaced', label: 'Replaced' },
  { value: 'pseudonymized', label: 'Pseudonymized' },
  { value: 'allowed', label: 'Recorded' },
  { value: 'unavailable', label: 'Not checked' },
  { value: 'too_large', label: 'Too large' },
];

const ACCOUNT = { account: 'Subscription', key: 'API-Key', unknown: '—' };
const DIRECTION = { inbound: '→ model', outbound: '← agent' };

function row(item) {
  const incident = item.incident || item;
  return {
    id: incident.id,
    ts: incident.created_at,
    sandbox: item.sandbox_name || '',
    owner: item.owner_name || '',
    agent: item.agent || '',
    account: ACCOUNT[item.account_kind] || '',
    direction: DIRECTION[incident.direction] || '',
    detector: incident.detector,
    state: incident.action,
    requested: incident.requested_action || '',
    from: incident.masked_value || '',
    to: incident.replacement || '',
    provider: item.provider || '',
    model: item.served_model || incident.model || '',
    host: incident.destination || '',
    ip: item.upstream_ip || '',
    ipWould: item.upstream_ip_would || '',
    count: incident.occurrences || 1,
    lastSeen: incident.last_seen_at || incident.created_at,
    raw: item,
  };
}

const columns = [
  { key: 'ts', label: 'When', sortable: true, filter: 'none', value: (r) => r.ts,
    render: (r) => <span class="mono small">{String(r.ts).replace('T', ' ').slice(0, 19)}</span> },
  { key: 'sandbox', label: 'Sandbox', sortable: true, groupable: true, filter: 'select', render: (r) => r.sandbox },
  { key: 'owner', label: 'User', sortable: true, groupable: true, filter: 'select', render: (r) => r.owner },
  { key: 'agent', label: 'Agent', sortable: true, filter: 'select', render: (r) => r.agent },
  { key: 'account', label: 'Account', sortable: true, filter: 'select', render: (r) => r.account },
  { key: 'direction', label: 'Direction', sortable: true, filter: 'select', render: (r) => r.direction },
  { key: 'detector', label: 'Detector', sortable: true, groupable: true, filter: 'select', render: (r) => r.detector },
  {
    key: 'state',
    label: 'What happened',
    sortable: true,
    groupable: true,
    filter: 'select',
    value: (r) => r.state,
    render: (r) => (
      <span>
        <DlpBadge state={r.state} />
        <Show when={r.requested && r.requested !== r.state}>
          <span class="muted small"> asked for: {r.requested}</span>
        </Show>
      </span>
    ),
  },
  {
    key: 'from',
    label: 'From → to',
    sortable: false,
    render: (r) => (
      <span class="mono small">
        {r.from || '—'}
        <Show when={r.to}><span class="muted"> → </span>{r.to}</Show>
      </span>
    ),
  },
  { key: 'provider', label: 'Provider', sortable: true, groupable: true, filter: 'select', render: (r) => r.provider },
  { key: 'model', label: 'Model', sortable: true, groupable: true, filter: 'select', render: (r) => r.model },
  { key: 'host', label: 'Destination', sortable: true, render: (r) => r.host },
  {
    key: 'ip',
    label: 'IP',
    sortable: false,
    render: (r) => (
      <span class="mono small">
        {r.ip || (r.ipWould ? <span class="muted">would: {r.ipWould}</span> : '—')}
      </span>
    ),
  },
  { key: 'count', label: '×', sortable: true, value: (r) => r.count, render: (r) => (r.count > 1 ? `×${r.count}` : '') },
];

export default function PrivacyDlpSection(props) {
  const [frame, setFrame] = createSignal('24h');
  const [view, setView] = createSignal('events');
  const [query, setQuery] = createSignal('');
  const [items, setItems] = createSignal([]);
  const [report, setReport] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [selected, setSelected] = createSignal(null);

  const load = async () => {
    try {
      const [log, overview] = await Promise.all([
        privacy.log({
          timeframe: frame(),
          sandbox_id: props.sandboxId,
          q: query() || undefined,
          limit: 300,
        }),
        privacy.overview({
          timeframe: frame(),
          sandbox_id: props.sandboxId,
          group_by: 'action',
        }),
      ]);
      setItems(log.items || []);
      setReport(overview);
      setError(null);
    } catch (failure) {
      setError(failure.message || 'The findings could not be loaded.');
    }
  };
  const poll = usePolling(load, { everyMs: 10000, onError: (e) => setError(e.message) });
  const rows = createMemo(() => items().map(row));
  const totals = () => report()?.totals;

  return (
    <section class="tab-panel">
      <div class="controls">
        <button class={poll.paused() ? 'secondary tiny' : 'primary tiny'} onClick={poll.toggle}>
          {poll.paused() ? 'Paused' : 'Live'}
        </button>
        <span class="muted small">Window</span>
        <select value={frame()} onChange={(event) => { setFrame(event.currentTarget.value); poll.refresh(); }}>
          <For each={FRAMES}>{(value) => <option value={value}>{value}</option>}</For>
        </select>
        <input
          class="input"
          type="search"
          placeholder="Search (detector, sandbox, model, host, user)…"
          value={query()}
          onInput={(event) => { setQuery(event.currentTarget.value); }}
        />
      </div>

      <Show when={error()}><p class="error">{error()}</p></Show>

      <div class="controls" style="gap:1.5rem;flex-wrap:wrap">
        <For each={STATES}>
          {(state) => (
            <span>
              <DlpBadge state={state.value} />{' '}
              <b>
                {totals()
                  ? (state.value === 'blocked' ? totals().blocked
                    : state.value === 'replaced' ? totals().replaced
                    : state.value === 'pseudonymized' ? totals().pseudonymized
                    : state.value === 'allowed' ? totals().allowed
                    : 0)
                  : '—'}
              </b>
            </span>
          )}
        </For>
        <span class="muted small">{totals() ? `${totals().incidents} rows, ${totals().seen} findings` : ''}</span>
      </div>

      <div class="controls">
        <For each={VIEWS}>
          {(item) => (
            <button class={view() === item.id ? 'primary tiny' : 'secondary tiny'} onClick={() => setView(item.id)}>
              {item.label}
            </button>
          )}
        </For>
      </div>

      <Show when={view() === 'events'}>
        <DataTable
          rows={rows()}
          columns={columns}
          rowKey={(r) => r.id}
          defaultSort={{ key: 'ts', dir: 'desc' }}
          storageKey={`dlp-${props.sandboxId || 'env'}`}
          onRowClick={(r) => setSelected(r.raw)}
          emptyText="No finding in this window. That is the good case; it is not proof that nothing would be found."
        />
      </Show>
      <Show when={view() === 'changes'}>
        <ChangesTable sandboxId={props.sandboxId} />
      </Show>
      <Show when={view() === 'exceptions'}>
        <DlpExceptionsTable sandboxId={props.sandboxId} />
      </Show>

      <DlpFindingDrawer
        incident={selected()}
        open={Boolean(selected())}
        onClose={() => setSelected(null)}
        onChanged={() => { setSelected(null); load(); }}
      />

      <p class="muted small">
        Only the masked excerpt and the placeholder are stored. The value itself is not in the
        database, and not even an administrator can read it.
      </p>
    </section>
  );
}

/// The policy history: who changed what, from which value to which.
function ChangesTable(props) {
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  const poll = usePolling(
    async () => {
      const answer = await privacy.changes({ sandbox_id: props.sandboxId, limit: 100 });
      setItems(answer.items || []);
      setError(null);
    },
    { everyMs: 30000, onError: (e) => setError(e.message) },
  );
  const rows = createMemo(() =>
    items().map((item) => ({
      id: item.id,
      ts: item.created_at,
      who: item.actor_label,
      kind: item.kind,
      scope: item.subject_type === 'environment' ? 'Environment default' : 'Sandbox',
      before: (item.detail?.before?.mode) || '—',
      after: (item.detail?.after?.mode) || item.detail?.mode || '—',
      raw: item,
    })),
  );
  return (
    <div>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <DataTable
        rows={rows()}
        columns={[
          { key: 'ts', label: 'When', sortable: true, render: (r) => <span class="mono small">{String(r.ts).replace('T', ' ').slice(0, 19)}</span> },
          { key: 'who', label: 'Who', sortable: true, filter: 'text' },
          { key: 'kind', label: 'What', sortable: true, filter: 'select' },
          { key: 'scope', label: 'Scope', sortable: true, groupable: true, filter: 'select' },
          { key: 'before', label: 'before', sortable: false, render: (r) => <span class="small" style="color:var(--dlp-blocked)">{r.before}</span> },
          { key: 'after', label: 'after', sortable: false, render: (r) => <span class="small" style="color:var(--dlp-clean)">{r.after}</span> },
        ]}
        rowKey={(r) => r.id}
        defaultSort={{ key: 'ts', dir: 'desc' }}
        emptyText="No change in this window."
      />
      <button class="secondary tiny" onClick={poll.refresh}>Refresh</button>
    </div>
  );
}
