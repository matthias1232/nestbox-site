// A modal question with named answers.
//
// The caller supplies the buttons, because "turn everything off" and "delete this
// rule" need different words, and the component must not guess a verb.

import { createEffect, For, onCleanup, Show } from 'solid-js';

import '../styles/controls.css';

export default function ConfirmDialog(props) {
  createEffect(() => {
    if (!props.open) return;
    const onKey = (event) => {
      if (event.key === 'Escape') props.onClose?.();
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  return (
    <Show when={props.open}>
      <div class="nb-dialog-backdrop" onClick={() => props.onClose?.()} />
      <div class="nb-dialog" role="dialog" aria-modal="true">
        <h3>{props.title}</h3>
        <div class="nb-dialog-body">{props.children}</div>
        <div class="nb-dialog-actions">
          <For each={props.actions || []}>
            {(action) => (
              <button type="button" class={action.tone || 'secondary'} onClick={() => action.onClick?.()}>
                {action.label}
              </button>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}
