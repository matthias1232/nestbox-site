// One refused destination with its answers: a card, or a table row when `inline` is set.
// Props (contract C15): alert, onAllowNow, onAllowFor(minutes), onDraft, onIgnore, onOpenSandbox;
// plus optional `busy` (disable buttons), `result` (text shown instead of the buttons) and
// `onHide` (the card's × button, which hides this pop-up and leaves the alert open);
//
// The card is the refusal the operator must answer (V11): an eyebrow "REFUSED · 3 min ago" in
// --bad, one line "<sandbox> → <host>:<port> (<channel>, N×)", and four answers in the order
// Allow 15 min (the one primary) · Allow now · Draft rule · Ignore. The table row keeps the
// full answer list, because there is room for it.
import { For, Show } from 'solid-js';
import { answers, ageOf } from './alertAnswers.js';
import '../styles/primitives.css';

// The card's four answers, in display order; the first one is the primary button.
const CARD_ANSWERS = [
  { id: '15', label: 'Allow 15 min' },
  { id: 'now', label: 'Allow now' },
  { id: 'draft', label: 'Draft rule' },
  { id: 'ignore', label: 'Ignore' },
];

export default function AlertPopup(props) {
  function pick(id) {
    if (id === 'now') props.onAllowNow?.();
    else if (id === '15') props.onAllowFor?.(15);
    else if (id === '60') props.onAllowFor?.(60);
    else if (id === 'draft') props.onDraft?.();
    else if (id === 'ignore') props.onIgnore?.();
    else if (id === 'open') props.onOpenSandbox?.();
  }
  const buttons = () => (props.inline ? answers(props.alert).filter((a) => a.id !== 'open') : CARD_ANSWERS);
  const primaryId = () => (props.inline ? 'now' : '15');
  const actions = () => (
    <Show when={!props.result} fallback={<span class="badge ok"><i />{props.result}</span>}>
      <div class="alert-actions">
        <For each={buttons()}>
          {(a) => (
            <button
              class={a.id === primaryId() ? 'primary tiny' : 'secondary tiny'}
              disabled={props.busy}
              onClick={() => pick(a.id)}
            >
              {a.label}
            </button>
          )}
        </For>
      </div>
    </Show>
  );
  const destination = () => `${props.alert.host}${props.alert.port ? `:${props.alert.port}` : ''}`;
  const age = () => ageOf(props.alert.last_at);
  const openSandbox = (event) => {
    event.preventDefault();
    props.onOpenSandbox?.();
  };

  return (
    <Show
      when={props.inline}
      fallback={
        <div class="alert-card" role="alert">
          <Show when={props.onHide}>
            <button
              class="alert-hide ghost tiny"
              aria-label="Hide this pop-up"
              title="Hide this pop-up"
              onClick={() => props.onHide?.()}
            >
              <span aria-hidden="true">×</span>
            </button>
          </Show>
          <p class="eyebrow alert-eyebrow">REFUSED{age() ? ` · ${age()}` : ''}</p>
          <p class="alert-line">
            <a href="#" onClick={openSandbox}>{props.alert.sandbox_name || 'sandbox'}</a>
            <span aria-hidden="true">→</span>
            <span class="mono">{destination()}</span>
            <span class="muted">
              (<span class="mono">{props.alert.channel || 'refused'}</span>, {props.alert.count || 1}×)
            </span>
          </p>
          {actions()}
        </div>
      }
    >
      <tr class="alert-row">
        <td class="mono">{destination()}</td>
        <td>
          <a href="#" onClick={openSandbox}>{props.alert.sandbox_name || 'sandbox'}</a>
        </td>
        <td><span class="badge bad"><i />{props.alert.channel || 'refused'}</span></td>
        <td class="mono small">{props.alert.count || 1}× · {ageOf(props.alert.last_at)}</td>
        <td>{actions()}</td>
      </tr>
    </Show>
  );
}
