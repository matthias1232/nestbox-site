// The loud banner for a sandbox where everything is allowed.
//
// It exists because "the proxy is off" and "a window is open" both remove the
// rules silently, and a user who forgot about it must see it on every screen of
// the box. It also carries the two ways out: turn the recorded traffic into rule
// drafts, or end it now.

import { Show, createSignal } from 'solid-js';

import Banner from '../../components/Banner.jsx';
import Countdown from '../../components/Countdown.jsx';
import * as policies from '../../api/policies.js';
import { allowAllSentence, allowAllUntil, draftSince, openWindows } from './network.js';

// The banner that says everything is allowed, with the two ways out of it.
export default function AllowAllBanner(props) {
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal(null);
  const [failure, setFailure] = createSignal(null);

  const sentence = () => allowAllSentence(props.control);
  const until = () => allowAllUntil(props.control);

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

  // Every destination that was denied or only observed becomes one allow draft,
  // so the answer to "allow everything" is a rule proposal and not a habit.
  function suggest() {
    run(async () => {
      const result = await policies.bulkDrafts({
        sandbox_id: props.sandboxId,
        since: draftSince(props.control),
      });
      const created = result?.created ?? (result?.drafts || []).length;
      setMessage(`${created} rule drafts created — review them under Policies.`);
    });
  }

  // Close what allows everything: the windows of this sandbox, and the proxy-off
  // mode, which goes back to the readable middle rather than to an extreme.
  function endNow() {
    const windows = openWindows(props.control);
    const offMode = props.control?.egress?.mode === 'off_allow_all';
    run(async () => {
      for (const entry of windows) await policies.closeWindow(entry.id);
      if (offMode) await policies.setEgressMode(props.sandboxId, 'hybrid', null);
    }, offMode || windows.length ? 'Everything is allowed no more.' : null);
  }

  return (
    <Show when={sentence()}>
      <Banner
        tone="bad"
        actions={
          <>
            <button class="tiny secondary" disabled={busy()} onClick={suggest}>Suggest rules</button>
            <button class="tiny danger" disabled={busy()} onClick={endNow}>End now</button>
          </>
        }
      >
        <div>
          {'⚠ '}
          {sentence()}
          <Show when={until()} fallback={<span class="muted small"> · until switched off</span>}>
            <Countdown prefix=" · ends in " until={until()} onExpire={() => props.onChanged?.()} />
          </Show>
        </div>
        <Show when={message()}>
          <p class="small">{message()}</p>
        </Show>
        <Show when={failure()}>
          <p class="error small">{failure()}</p>
        </Show>
      </Banner>
    </Show>
  );
}
