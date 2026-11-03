// What the content policy found in this sandbox, for its owner.
//
// The same table the administrator sees, narrowed to this sandbox. Collapsed by
// default: a finding is worth seeing when it happens, not on every visit.
import { Show, createSignal } from 'solid-js';
import PrivacyDlpSection from '../privacy/PrivacyDlpSection.jsx';

export default function PrivacyCard(props) {
  const [open, setOpen] = createSignal(false);
  return (
    <section class="wb-card">
      <div class="wb-card-head">
        <h2>Privacy findings</h2>
        <button class="tiny secondary" onClick={() => setOpen(!open())}>{open() ? 'Hide' : 'Show'}</button>
      </div>
      <p class="muted small">What the check found in this sandbox, masked</p>
      <Show
        when={open()}
        fallback={
          <p class="muted small">
            Findings are stored masked only: the value itself is never stored and nobody can read it.
          </p>
        }
      >
        <PrivacyDlpSection sandboxId={props.sandboxId} />
      </Show>
    </section>
  );
}
