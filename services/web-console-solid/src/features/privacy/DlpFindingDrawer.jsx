// One finding in full, and the way to allow it on purpose.
//
// The drawer is where a blocked request becomes a decision the user can revisit:
// what fired, what happened, where the request was going — and a button that says
// "allow this value here for an hour", so the next attempt goes through.
import { For, Show, createSignal } from 'solid-js';
import * as privacy from '../../api/privacy.js';
import DlpBadge from '../../components/DlpBadge.jsx';
import RouteChain from '../../components/RouteChain.jsx';

const DURATIONS = [
  { label: '15 min', minutes: 15 },
  { label: '1 h', minutes: 60 },
  { label: '24 h', minutes: 1440 },
  { label: '7 days', minutes: 10080 },
  { label: 'Always', minutes: null },
];
const ACTIONS = [
  { value: 'allow', label: 'allow' },
  { value: 'replace', label: 'replace instead of block' },
  { value: 'pseudonymize', label: 'pseudonymize' },
];

function field(label, value) {
  return (
    <Show when={value}>
      <div class="small"><span class="muted">{label}: </span><span class="mono">{value}</span></div>
    </Show>
  );
}

export default function DlpFindingDrawer(props) {
  const incident = () => props.incident?.incident || props.incident;
  const wrapper = () => props.incident || {};
  const [action, setAction] = createSignal('allow');
  const [minutes, setMinutes] = createSignal(60);
  const [sandboxOnly, setSandboxOnly] = createSignal(true);
  const [scope, setScope] = createSignal('value');
  const [reason, setReason] = createSignal('');
  const [message, setMessage] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  async function allow() {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await privacy.createException({
        incident_id: incident().id,
        scope: scope(),
        sandbox_only: sandboxOnly(),
        action: action(),
        minutes: minutes(),
        reason: reason(),
      });
      setMessage('Exception created. It applies from the next request.');
      props.onChanged?.();
    } catch (failure) {
      setError(failure.message || 'The exception could not be created.');
    } finally {
      setBusy(false);
    }
  }

  const route = () => {
    const item = wrapper();
    return [
      { kind: 'sandbox', label: item.sandbox_name || 'Sandbox', state: 'passed' },
      { kind: 'egress', label: 'Egress', detail: item.channel || '', state: 'passed' },
      {
        kind: 'dlp',
        label: `DLP: ${incident()?.action || ''}`,
        detail: incident()?.requested_action ? `asked for: ${incident().requested_action}` : '',
        state: incident()?.action === 'blocked' ? 'blocked' : 'passed',
      },
      {
        kind: 'upstream',
        label: item.upstream_ip ? `${incident()?.destination} (${item.upstream_ip})` : (incident()?.destination || ''),
        state: incident()?.action === 'blocked' ? 'would' : 'passed',
      },
      {
        kind: 'model',
        label: item.served_model || incident()?.model || '',
        state: incident()?.action === 'blocked' ? 'would' : 'passed',
      },
    ];
  };

  return (
    <Show when={props.open}>
      <div class="card" style="position:fixed;inset:0;z-index:40;overflow:auto;padding:1rem;background:color-mix(in srgb, currentColor 4%, var(--background))">
        <div class="controls">
          <DlpBadge state={incident()?.action} />
          <b class="mono">{incident()?.detector}</b>
          <span class="muted small">{String(incident()?.created_at || '').replace('T', ' ').slice(0, 19)}</span>
          <Show when={(incident()?.occurrences || 1) > 1}>
            <span class="muted small">×{incident().occurrences}</span>
          </Show>
          <button class="tiny secondary" onClick={props.onClose}>Close</button>
        </div>

        <div class="small">
          {field('Sandbox', wrapper().sandbox_name)}
          {field('User', wrapper().owner_name)}
          {field('Agent', wrapper().agent)}
          {field('Account', wrapper().account_kind)}
          {field('Direction', incident()?.direction)}
          {field('Source', incident()?.source)}
          {field('Request', `${incident()?.method || ''} ${incident()?.path || ''}`)}
          {field('Provider', wrapper().provider)}
          {field('Model', wrapper().served_model || incident()?.model)}
          {field('Destination', incident()?.destination)}
          {field('IP', wrapper().upstream_ip || (wrapper().upstream_ip_would ? `would: ${wrapper().upstream_ip_would}` : ''))}
          {field('masked value', incident()?.masked_value)}
          {field('Substitute', incident()?.replacement)}
          {field('Fingerprint', (incident()?.value_fingerprint || '').slice(0, 16))}
          {field('Exception', incident()?.exception_id)}
        </div>

        <RouteChain route={route()} />

        <Show when={incident()?.value_fingerprint}>
          <div class="card">
            <b>Allow this request</b>
            <div class="controls">
              <select value={scope()} onChange={(e) => setScope(e.currentTarget.value)}>
                <option value="value">this value only</option>
                <option value="detector">any value from this detector</option>
              </select>
              <select value={action()} onChange={(e) => setAction(e.currentTarget.value)}>
                <For each={ACTIONS}>{(entry) => <option value={entry.value}>{entry.label}</option>}</For>
              </select>
              <select value={String(minutes())} onChange={(e) => setMinutes(e.currentTarget.value === 'null' ? null : Number(e.currentTarget.value))}>
                <For each={DURATIONS}>{(entry) => <option value={String(entry.minutes)}>{entry.label}</option>}</For>
              </select>
              <label class="small">
                <input type="checkbox" checked={sandboxOnly()} onChange={(e) => setSandboxOnly(e.currentTarget.checked)} /> this sandbox only
              </label>
              <input class="input" placeholder="Reason (optional)" value={reason()} onInput={(e) => setReason(e.currentTarget.value)} />
              <button class="primary tiny" disabled={busy()} onClick={allow}>Create exception</button>
            </div>
            <Show when={message()}><p class="notice small">{message()}</p></Show>
            <Show when={error()}><p class="error small">{error()}</p></Show>
          </div>
        </Show>
      </div>
    </Show>
  );
}
