// What gets started inside the sandbox, and by which way in: typed in a
// console terminal, run over SSH, or started by an agent or script already
// inside. A sample taken while this tab is open, not a recording.
import { For, Show, createSignal } from 'solid-js';
import * as deck from '../../api/deck.js';
import { usePolling } from '../../components/Poll.js';

const WAYS = ['all', 'terminal', 'ssh', 'inside'];

export default function CommandsTab(props) {
  const [data, setData] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [way, setWay] = createSignal('all');
  const [text, setText] = createSignal('');
  const poll = usePolling(() => deck.liveView(props.sandbox.id, 'commands'), {
    everyMs: 2000,
    onData: (d) => { setData(d); setError(null); },
    onError: (e) => setError(e.message),
  });
  const rows = () => [...(data()?.items || [])].reverse().filter((row) => {
    if (way() !== 'all' && row.via !== way()) return false;
    const needle = text().trim().toLowerCase();
    return !needle || row.command.toLowerCase().includes(needle);
  });

  return (
    <section class="tab-panel">
      <div class="controls">
        <button class={poll.paused() ? 'secondary tiny' : 'primary tiny'} onClick={poll.toggle}>{poll.paused() ? 'Paused' : 'Live'}</button>
        <span class="muted small">way in</span>
        <select value={way()} onChange={(e) => setWay(e.currentTarget.value)}>
          <For each={WAYS}>{(item) => <option value={item}>{item}</option>}</For>
        </select>
        <input placeholder="search commands" value={text()} onInput={(e) => setText(e.currentTarget.value)} />
        <span class="muted small">{data() ? `${rows().length} commands since ${String(data().watching_since).slice(11, 19)}` : ''}</span>
      </div>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <div class="table-scroll">
        <table class="dense">
          <thead><tr><th>Time</th><th>Way in</th><th>User</th><th>PID</th><th>Command</th></tr></thead>
          <tbody>
            <For each={rows()} fallback={<tr><td colspan="5" class="muted small">Nothing has started since you opened this tab. Run something in the sandbox and it appears here.</td></tr>}>
              {(row) => (
                <tr>
                  <td class="mono small">{String(row.ts).slice(11, 19)}</td>
                  <td><span class={`badge ${row.via === 'inside' ? 'muted' : 'warn'}`}><i />{row.via}</span></td>
                  <td>{row.user}</td>
                  <td class="mono">{row.pid}</td>
                  <td class="mono small" style="white-space:pre-wrap;word-break:break-all">{row.command}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <p class="muted small">{data()?.note}</p>
    </section>
  );
}
