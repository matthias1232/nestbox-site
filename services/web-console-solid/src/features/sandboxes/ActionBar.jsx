// The Admin Center action bar: lifecycle, console, allow-all, and shortcuts to the Policies tab
// and the Workbox. It sits in the sticky page header, so it never scrolls away.
//
// The bar owns its busy state and its calls; the parent only learns what changed through
// `onChanged` (re-read the sandbox) and `onControl` (the control summary the quick call answers).
import { For, Show, createSignal } from 'solid-js';
import * as api from '../../api/sandboxes.js';
import * as deck from '../../api/deck.js';
import * as quick from '../../api/quick.js';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import Countdown from '../../components/Countdown.jsx';
import MoveDialog from '../workbox/MoveDialog.jsx';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';

const LIVE = ['running', 'starting', 'provisioning'];
const RESTING = ['stopped', 'failed', 'pending'];
const ALLOW_MINUTES = 15;

// The agent sessions that are working right now, from the control summary.
function busySessions(control) {
  const agents = Array.isArray(control?.agents) ? control.agents : [];
  return agents.flatMap((agent) => (agent.sessions || []).filter((session) => session.busy));
}

// Props: sandbox, control (summary or null), role, onChanged(), onControl(summary), onTab(id), onConsole(mode).
export default function ActionBar(props) {
  const ui = useUi();
  const [busy, setBusy] = createSignal(false);
  const [confirm, setConfirm] = createSignal(null); // 'stop' | 'recreate' | null
  const [moving, setMoving] = createSignal(false);
  const [menu, setMenu] = createSignal(false);

  const id = () => props.sandbox?.id;
  const status = () => props.sandbox?.status || '';
  const live = () => LIVE.includes(status());
  const resting = () => RESTING.includes(status());
  const allowAll = () => Boolean(props.control?.allow_all);
  const lastMode = () => ui?.console.mode(id()) || 'shell';
  const toast = (tone, text) => ui?.toast.show({ tone, text });

  // Run one call with the bar disabled; say what happened in a toast.
  async function run(work, done) {
    if (busy()) return;
    setBusy(true);
    try {
      const answer = await work();
      toast('ok', done);
      props.onChanged?.();
      return answer;
    } catch (failure) {
      toast('bad', failure?.message || 'The action failed.');
    } finally {
      setBusy(false);
    }
  }

  const lifecycle = (action, word) => run(() => api.act(id(), action), `${props.sandbox?.name}: ${word}.`);

  function askStop() {
    if (busySessions(props.control).length > 0) setConfirm('stop');
    else lifecycle('stop', 'stopped');
  }

  function confirmed() {
    const what = confirm();
    setConfirm(null);
    if (what === 'stop') lifecycle('stop', 'stopped');
    if (what === 'recreate') run(() => deck.recreate(id()), 'recreated');
  }

  async function toggleAllowAll() {
    const answer = await run(
      () => (allowAll() ? quick.set(id(), 'allow_all', false) : quick.set(id(), 'allow_all', true, ALLOW_MINUTES)),
      allowAll() ? 'Allow-all was closed.' : `Everything is allowed for ${ALLOW_MINUTES} min.`,
    );
    if (answer && typeof answer === 'object') props.onControl?.(answer);
  }

  function openConsole(mode) {
    setMenu(false);
    props.onConsole?.(mode);
  }

  const busyCount = () => busySessions(props.control).length;

  return (
    <div class="ac-actionbar" role="toolbar" aria-label="Sandbox actions">
      <Show when={resting()}>
        <button class="primary tiny" disabled={busy()} onClick={() => lifecycle('start', 'started')}>Start</button>
      </Show>
      <button class={live() ? 'primary tiny' : 'secondary tiny'} disabled={busy() || !live()} onClick={askStop}>Stop</button>
      <button class="secondary tiny" disabled={busy() || !live()} onClick={() => lifecycle('restart', 'restarted')}>Restart</button>
      <button class="secondary tiny" disabled={busy()} onClick={() => setConfirm('recreate')}>Recreate</button>
      <button class="secondary tiny" disabled={busy()} onClick={() => setMoving(true)}>Move…</button>

      <span class="ac-split">
        <button class="secondary tiny" disabled={!live()} title={lastMode() === 'desktop' ? 'Open the desktop' : 'Open the shell'} onClick={() => openConsole(lastMode())}>
          Console
        </button>
        <button
          class="secondary tiny ac-split-toggle"
          aria-haspopup="menu"
          aria-expanded={menu()}
          aria-label="Choose the console"
          disabled={!live()}
          onClick={() => setMenu(!menu())}
          onKeyDown={(event) => { if (event.key === 'Escape') setMenu(false); }}
        >▾</button>
        <Show when={menu()}>
          <ul class="ac-menu" role="menu" onKeyDown={(event) => { if (event.key === 'Escape') setMenu(false); }}>
            <For each={[['shell', 'Shell'], ['desktop', 'Desktop'], ['ssh', 'SSH command']]}>
              {([mode, label]) => (
                <li role="none"><button role="menuitem" class="ghost tiny" onClick={() => openConsole(mode)}>{label}</button></li>
              )}
            </For>
          </ul>
        </Show>
      </span>

      <button
        class={allowAll() ? 'danger tiny' : 'secondary tiny'}
        disabled={busy() || !live()}
        aria-pressed={allowAll()}
        onClick={toggleAllowAll}
      >
        <Show when={allowAll()} fallback={`Allow all ${ALLOW_MINUTES} min`}>
          Stop allowing{' '}
          <Show when={props.control?.allow_all_until}>
            (<Countdown until={props.control.allow_all_until} onExpire={() => props.onChanged?.()} />)
          </Show>
        </Show>
      </button>

      <button class="ghost tiny" onClick={() => props.onTab?.('policies')}>Policies</button>
      <button class="ghost tiny" onClick={() => navigate(paths.box(id()))}>Workbox</button>

      <ConfirmDialog
        open={confirm() !== null}
        title={confirm() === 'recreate' ? `Recreate ${props.sandbox?.name}?` : `Stop ${props.sandbox?.name}?`}
        onClose={() => setConfirm(null)}
        actions={[
          { label: 'Cancel', tone: 'secondary', onClick: () => setConfirm(null) },
          { label: confirm() === 'recreate' ? `Recreate ${props.sandbox?.name}` : `Stop ${props.sandbox?.name}`, tone: 'danger', onClick: confirmed },
        ]}
      >
        <Show
          when={confirm() === 'recreate'}
          fallback={<p>{busyCount()} agent session{busyCount() === 1 ? ' is' : 's are'} working right now. Stopping ends {busyCount() === 1 ? 'it' : 'them'}.</p>}
        >
          <p>The container is replaced. Everything on a persistent root stays; anything written elsewhere inside the container is lost, and open terminals end.</p>
        </Show>
      </ConfirmDialog>

      <MoveDialog
        open={moving()}
        sandboxId={id()}
        sandboxName={props.sandbox?.name}
        currentNode={props.sandbox?.node_name}
        onClose={() => setMoving(false)}
        onMoved={() => { setMoving(false); props.onChanged?.(); }}
      />
    </div>
  );
}
