// "N destinations were seen, M would have been refused" and the button that turns
// them into rule drafts (the bulk-draft request of ADR 0053).

import { Show, createEffect, createMemo, createSignal } from 'solid-js';

import * as deck from '../../api/deck.js';
import * as policies from '../../api/policies.js';
import { navigate } from '../../routing.js';
import { useUi } from '../../state/ui.jsx';
import { draftsSentence, rulesSentence, stripState, summarizeTraffic, windowFor } from './controlStrip.js';
import { draftSince } from './network.js';

// Props: sandboxId, control (the summary).
export default function RulesFromLogCard(props) {
  const ui = useUi();
  const [summary, setSummary] = createSignal({ seen: 0, refused: 0 });
  const [busy, setBusy] = createSignal(false);
  const [failure, setFailure] = createSignal(null);

  const allowAllOn = createMemo(() => stripState(props.control).allowAll.on);
  const since = createMemo(() => draftSince(props.control));

  // Reload the counts when the start of the period changes (a window opened or ended).
  createEffect(() => {
    const from = since();
    deck
      .traffic(props.sandboxId, { window: windowFor(from), limit: 200 })
      .then((answer) => {
        const items = (answer?.items || []).filter((item) => !item.ts || Date.parse(item.ts) >= Date.parse(from));
        setSummary(summarizeTraffic(items));
        setFailure(null);
      })
      .catch((error) => setFailure(error.message));
  });

  async function create() {
    setBusy(true);
    try {
      const result = await policies.bulkDrafts({ sandbox_id: props.sandboxId, since: since() });
      ui.toast.show({ tone: 'ok', text: draftsSentence(result) });
    } catch (error) {
      ui.toast.show({ tone: 'bad', text: error.message || 'The drafts could not be created.' });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="wb-card">
      <h2>Rules from the log</h2>
      <Show when={!failure()} fallback={<p class="error small">{failure()}</p>}>
        <p>{rulesSentence(summary(), since(), allowAllOn())}</p>
      </Show>
      <button class="secondary tiny" disabled={busy() || summary().seen === 0} onClick={create}>
        {busy() ? 'Creating…' : 'Create rule drafts from these'}
      </button>
      <p class="muted small">
        <a href="/policies?tab=drafts" onClick={(e) => { e.preventDefault(); navigate('/policies?tab=drafts'); }}>
          Review the drafts
        </a>
      </p>
    </section>
  );
}
