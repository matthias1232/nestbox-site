// The evaluation windows of one sandbox: what is allowed for a while, how long it
// still lasts, and the button that ends it. The proxy mode and the "allow
// everything" choices live in the control strip; this card keeps the list and the
// one hint that needs a recreate. Texts come from `network.js`.

import { For, Show, createSignal } from 'solid-js';

import Countdown from '../../components/Countdown.jsx';
import ConfirmDialog from '../../components/ConfirmDialog.jsx';
import * as deck from '../../api/deck.js';
import * as policies from '../../api/policies.js';
import { channelLabel, modeLabel, openWindows } from './network.js';

// The windows card: the mode in one line, the open windows, the recreate hint.
export default function NetworkCard(props) {
  const sandboxId = () => props.sandboxId;
  const control = () => props.control || {};
  const egress = () => control().egress || {};
  const windows = () => openWindows(control());

  const [failure, setFailure] = createSignal(null);
  const [message, setMessage] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [recreateOpen, setRecreateOpen] = createSignal(false);

  // One place for the three states a call has: it failed, it worked, it runs.
  async function run(action, done) {
    setFailure(null);
    setMessage(null);
    setBusy(true);
    try {
      await action();
      if (done) setMessage(done);
      props.onChanged?.();
    } catch (error) {
      setFailure(error.message + (error.code ? ` (${error.code})` : ''));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="wb-card">
      <h2>Network windows</h2>
      <p class="muted small">
        Proxy mode: <strong>{modeLabel(egress().mode || 'forward_proxy')}</strong>. Change it in the strip above.
      </p>

      <Show when={failure()}>
        <p class="error small">{failure()}</p>
      </Show>
      <Show when={message()}>
        <p class="notice small">{message()}</p>
      </Show>

      <Show when={egress().needs_recreate}>
        <div class="warn-note">
          <p>This sandbox was made before live switching; recreate it once so TLS can be opened.</p>
          <button class="secondary tiny" disabled={busy()} onClick={() => setRecreateOpen(true)}>Recreate once</button>
        </div>
      </Show>

      <Show when={windows().length} fallback={<p class="muted small">No window is open; the rules apply.</p>}>
        <ul class="plain">
          <For each={windows()}>
            {(entry) => (
              <li>
                <span class="badge info">
                  <i />
                  {entry.mode === 'bypass' ? 'allowed quietly' : 'allowed and recorded'} · {channelLabel(entry.channel)}
                </span>{' '}
                <Show when={entry.expires_at} fallback={<span class="muted small">until you turn it off</span>}>
                  <Countdown prefix="ends in " until={entry.expires_at} onExpire={() => props.onChanged?.()} />
                </Show>
                {' '}
                <button
                  class="tiny ghost"
                  disabled={busy()}
                  onClick={() => run(() => policies.closeWindow(entry.id), 'The window is closed.')}
                >
                  End
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>

      <ConfirmDialog
        open={recreateOpen()}
        title="Recreate this sandbox once?"
        onClose={() => setRecreateOpen(false)}
        actions={[
          {
            label: 'Recreate',
            tone: 'danger',
            onClick: () => {
              setRecreateOpen(false);
              run(() => deck.recreate(sandboxId()), 'The sandbox was recreated; TLS can be opened now.');
            },
          },
          { label: 'Cancel', tone: 'secondary', onClick: () => setRecreateOpen(false) },
        ]}
      >
        <p>
          The container is made again with the CA mounted. Persistent folders and logins stay;
          anything else inside the container is lost, and open terminals end.
        </p>
      </ConfirmDialog>
    </section>
  );
}
