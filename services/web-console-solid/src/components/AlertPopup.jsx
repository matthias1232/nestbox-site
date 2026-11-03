// One refused destination with its answers: a card, or a table row when `inline` is set.
// Props (contract C15): alert, onAllowNow, onAllowFor(minutes), onDraft, onIgnore, onOpenSandbox;
// plus optional `busy` (disable buttons) and `result` (text shown instead of the buttons).
import { For, Show } from 'solid-js';
import { answers, ageOf } from './alertAnswers.js';

export default function AlertPopup(props) {
  function pick(id) {
    if (id === 'now') props.onAllowNow?.();
    else if (id === '15') props.onAllowFor?.(15);
    else if (id === '60') props.onAllowFor?.(60);
    else if (id === 'draft') props.onDraft?.();
    else if (id === 'ignore') props.onIgnore?.();
    else if (id === 'open') props.onOpenSandbox?.();
  }
  const buttons = () => answers(props.alert).filter((a) => !props.inline || a.id !== 'open');
  const actions = () => (
    <Show when={!props.result} fallback={<span class="badge ok"><i />{props.result}</span>}>
      <div class="alert-actions">
        <For each={buttons()}>
          {(a) => (
            <button
              class={a.id === 'now' ? 'primary tiny' : 'secondary tiny'}
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

  return (
    <Show
      when={props.inline}
      fallback={
        <div class="alert-card" role="alert">
          <div class="alert-head">
            <strong class="mono">{destination()}</strong>
            <span class="badge bad"><i />{props.alert.channel || 'refused'}</span>
          </div>
          <p class="muted small">
            <a href="#" onClick={(e) => { e.preventDefault(); props.onOpenSandbox?.(); }}>{props.alert.sandbox_name || 'sandbox'}</a>
            {' · '}{props.alert.count || 1}× · {ageOf(props.alert.last_at)}
          </p>
          {actions()}
        </div>
      }
    >
      <tr class="alert-row">
        <td class="mono">{destination()}</td>
        <td>
          <a href="#" onClick={(e) => { e.preventDefault(); props.onOpenSandbox?.(); }}>{props.alert.sandbox_name || 'sandbox'}</a>
        </td>
        <td><span class="badge bad"><i />{props.alert.channel || 'refused'}</span></td>
        <td class="mono small">{props.alert.count || 1}× · {ageOf(props.alert.last_at)}</td>
        <td>{actions()}</td>
      </tr>
    </Show>
  );
}
