// The inventory tree for the sidebar and the home page (contract C3).
//
// Wave 0 stub: package F01 loads `api/inventory.js`, keeps it fresh from the live stream and
// fills `root`. F05, F06 and F17 code against this shape.
import { createSignal } from 'solid-js';

const [view, setView] = createSignal('infrastructure');

// `root()` is the tree root or null; `error()` a message or null; `refresh()` reloads.
export function useInventory() {
  return { root: () => null, error: () => null, refresh() {}, view, setView };
}
