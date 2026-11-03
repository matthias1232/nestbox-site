// ShortcutsSheet: a dialog listing the keyboard shortcuts; opened by `?` or the Help menu.
// Mounted once by the Shell. Also registers the global shortcuts it lists.
import { For, Show, createEffect, onCleanup } from 'solid-js';
import '../../styles/help.css';
import KeyHint from '../../components/KeyHint.jsx';
import { useUi, onKey } from '../../state/ui.jsx';
import { SHORTCUTS, registerGlobalShortcuts, trapFocus } from './useHelp.js';

export default function ShortcutsSheet() {
  const ui = useUi();
  registerGlobalShortcuts(ui);
  let dialog;

  createEffect(() => {
    if (!ui.shortcuts.isOpen()) return;
    const stopEscape = onKey('Escape', () => ui.shortcuts.close());
    const stopTrap = dialog ? trapFocus(dialog) : () => {};
    onCleanup(() => {
      stopEscape();
      stopTrap();
    });
  });

  return (
    <Show when={ui.shortcuts.isOpen()}>
      <div class="help-backdrop" onClick={() => ui.shortcuts.close()} />
      <div ref={dialog} class="help-dialog shortcuts-sheet" role="dialog" aria-modal="true" aria-label="Keyboard shortcuts">
        <header>
          <h2>Keyboard shortcuts</h2>
          <button class="ghost" onClick={() => ui.shortcuts.close()} aria-label="Close shortcuts">×</button>
        </header>
        <div class="shortcuts-grid">
          <For each={SHORTCUTS}>
            {(group) => (
              <section>
                <span class="eyebrow">{group.group}</span>
                <dl>
                  <For each={group.items}>
                    {(item) => (
                      <>
                        <dt>{item.label}</dt>
                        <dd><KeyHint keys={item.combo} /></dd>
                      </>
                    )}
                  </For>
                </dl>
              </section>
            )}
          </For>
        </div>
        <p class="muted small">Shortcuts do not fire while you type in a field. Press g, then the second key, within a second.</p>
      </div>
    </Show>
  );
}
