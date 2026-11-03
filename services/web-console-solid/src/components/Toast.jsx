import { For, Show } from 'solid-js';
import '../styles/primitives.css';
import { useUi } from '../state/ui.jsx';

// The bottom-right toast stack. It reads `ui.toast.queue()`; the Undo button runs `undo()` then dismisses.
export default function Toast() {
  const ui = useUi();
  return (
    <div class="toast-stack" role="status" aria-live="polite">
      <For each={ui.toast.queue()}>
        {(toast) => (
          <div class="toast" data-tone={toast.tone}>
            <span class="toast-text">{toast.text}</span>
            <Show when={toast.undo}>
              <button
                type="button"
                class="ghost tiny"
                onClick={() => {
                  toast.undo();
                  ui.toast.dismiss(toast.id);
                }}
              >
                Undo
              </button>
            </Show>
            <button type="button" class="ghost tiny" aria-label="Dismiss" onClick={() => ui.toast.dismiss(toast.id)}>
              ×
            </button>
          </div>
        )}
      </For>
    </div>
  );
}
