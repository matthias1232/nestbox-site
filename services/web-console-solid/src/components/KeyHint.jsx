import { For } from 'solid-js';

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '');

// Shortcut keys as <kbd> elements. `keys` like "mod+k" or "g h"; `mod` is Ctrl, or the Command key on macOS.
export default function KeyHint(props) {
  const parts = () =>
    String(props.keys || '')
      .split(/\+|\s+/)
      .filter(Boolean)
      .map((part) => (part === 'mod' ? (isMac ? '⌘' : 'Ctrl') : part.length === 1 ? part.toUpperCase() : part));
  return (
    <span class="key-hint">
      <For each={parts()}>{(part) => <kbd>{part}</kbd>}</For>
    </span>
  );
}
