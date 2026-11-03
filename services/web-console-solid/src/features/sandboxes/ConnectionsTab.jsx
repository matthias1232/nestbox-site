// Every socket in the sandbox and the process that owns it, refreshed live.
//
// This answers "who is talking to whom": which process holds which connection,
// which of them leave for the internet, which are SSH, and which are name
// lookups. It is a sample taken every couple of seconds, not a trace, and the
// note under the table says so.
import { For, Show, createSignal } from 'solid-js';
import * as deck from '../../api/deck.js';
import { usePolling } from '../../components/Poll.js';

const SCOPES = ['all', 'internet', 'internal', 'loopback'];

export default function ConnectionsTab(props) {
  const [data, setData] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [scope, setScope] = createSignal('internet');
  const [kind, setKind] = createSignal('all');
  const [text, setText] = createSignal('');
  const poll = usePolling(() => deck.liveView(props.sandbox.id, 'connections'), {
    everyMs: 2500,
    onData: (d) => { setData(d); setError(null); },
    onError: (e) => setError(e.message),
  });

  const rows = () => (data()?.items || []).filter((row) => {
    if (scope() !== 'all' && row.scope !== scope() && row.direction !== 'listen') return false;
    if (scope() !== 'all' && row.direction === 'listen' && kind() !== 'listen') return false;
    if (kind() === 'listen' && row.direction !== 'listen') return false;
    if (kind() === 'ssh' && !row.ssh) return false;
    if (kind() === 'dns' && !row.dns) return false;
    if (kind() === 'outbound' && row.direction !== 'outbound') return false;
    const needle = text().trim().toLowerCase();
    if (needle && !`${row.process} ${row.command} ${row.remote} ${row.local}`.toLowerCase().includes(needle)) return false;
    return true;
  });
  const counts = () => data()?.counts || {};

  return (
    <section class="tab-panel">
      <div class="controls">
        <button class={poll.paused() ? 'secondary tiny' : 'primary tiny'} onClick={poll.toggle}>{poll.paused() ? 'Paused' : 'Live'}</button>
        <button class="secondary tiny" onClick={poll.refresh}>Refresh</button>
        <span class="muted small">where</span>
        <select value={scope()} onChange={(e) => setScope(e.currentTarget.value)}>
          <For each={SCOPES}>{(item) => <option value={item}>{item}</option>}</For>
        </select>
        <span class="muted small">kind</span>
        <select value={kind()} onChange={(e) => setKind(e.currentTarget.value)}>
          <option value="all">all</option>
          <option value="outbound">outgoing</option>
          <option value="listen">listening</option>
          <option value="ssh">SSH</option>
          <option value="dns">name lookups</option>
        </select>
        <input placeholder="process or address" value={text()} onInput={(e) => setText(e.currentTarget.value)} />
        <span class="badge ok"><i />{counts().internet ?? 0} to the internet</span>
        <span class="badge muted"><i />{counts().ssh ?? 0} SSH</span>
        <span class="badge muted"><i />{counts().dns ?? 0} DNS</span>
        <span class="badge muted"><i />{counts().listening ?? 0} listening</span>
      </div>
      <Show when={error()}><p class="error">{error()} (a stopped sandbox has no connections)</p></Show>
      <div class="table-scroll">
        <table class="dense">
          <thead><tr><th>Process</th><th>PID</th><th>Proto</th><th>Local</th><th>Remote</th><th>Where</th><th>State</th><th>Kind</th></tr></thead>
          <tbody>
            <For each={rows()} fallback={<tr><td colspan="8" class="muted small">No connection matches this filter.</td></tr>}>
              {(row) => (
                <tr>
                  <td title={row.command}><span class="mono">{row.process || '–'}</span> <span class="muted small">{(row.command || '').slice(0, 60)}</span></td>
                  <td class="mono">{row.pid ?? '–'}</td>
                  <td>{row.proto}</td>
                  <td class="mono small">{row.local}</td>
                  <td class="mono small">{row.remote || '–'}</td>
                  <td><span class={`badge ${row.scope === 'internet' ? 'warn' : 'muted'}`}><i />{row.scope}</span></td>
                  <td>{row.state}</td>
                  <td>{row.ssh ? 'SSH ' : ''}{row.dns ? 'DNS ' : ''}{row.direction}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <p class="muted small">{data() ? `${data().taken_at} · ${data().note}` : ''}</p>
    </section>
  );
}
