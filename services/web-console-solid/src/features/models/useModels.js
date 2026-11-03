// Catalogue + installs + hardware of the Models screens, polled together every 20 s.
import { createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import * as models from '../../api/models.js';

// The verdicts that mean "this node can run it". Mirrors the server's `Fit::fits`; used only to
// count and filter, never to decide (see AGENTS.md).
export const FITS = new Set(['installable', 'cpu_installable', 'already_installed', 'update_available']);

const POLL_MS = 20000;

// One poll for the catalogue classified against `nodeName()`, the installs of every node, and
// the filters. Returns signals plus `refresh(probe)`; call it inside a component.
export function useModels(nodeName) {
  const [items, setItems] = createSignal([]);
  const [hardware, setHardware] = createSignal(null);
  const [budget, setBudget] = createSignal(null);
  const [installs, setInstalls] = createSignal([]);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  const [search, setSearch] = createSignal('');
  const [scope, setScope] = createSignal('all');
  const [family, setFamily] = createSignal('all');
  const [runtime, setRuntime] = createSignal('all');
  const [artefactOnly, setArtefactOnly] = createSignal(false);

  let alive = true;
  let timer = null;

  // Load everything; `probe` forces a fresh hardware measurement.
  async function refresh(probe = false) {
    setBusy(true);
    try {
      const catalog = await models.catalog(probe, {}, nodeName());
      if (!alive) return null;
      setItems(catalog.items || []);
      setHardware(catalog.hardware || null);
      setBudget(catalog.budget || null);
      setError(null);
      try {
        const answer = await models.installs();
        if (alive) setInstalls(answer.items || []);
      } catch {
        if (alive) setInstalls([]);
      }
      return catalog;
    } catch (failure) {
      if (alive) setError(failure.message);
      return null;
    } finally {
      if (alive) setBusy(false);
    }
  }

  onMount(() => {
    timer = setInterval(() => refresh(false), POLL_MS);
  });
  onCleanup(() => {
    alive = false;
    if (timer) clearInterval(timer);
  });

  const families = createMemo(() => [...new Set(items().map((m) => m.family).filter(Boolean))].sort());

  const counts = createMemo(() => {
    const local = items().filter((m) => !m.is_remote);
    const fitting = local.filter((m) => FITS.has(m.fit?.fit));
    return { total: items().length, fits: fitting.length, notFits: local.length - fitting.length };
  });

  // How many nodes hold each model: { [key]: installs[] }.
  const installsByModel = createMemo(() => {
    const map = {};
    for (const row of installs()) (map[row.model_key] ||= []).push(row);
    return map;
  });

  const visible = createMemo(() => {
    const needle = search().trim().toLowerCase();
    const matched = items().filter((m) => {
      if (scope() === 'fits' && !FITS.has(m.fit?.fit)) return false;
      if (scope() === 'not' && (m.is_remote || FITS.has(m.fit?.fit))) return false;
      if (family() !== 'all' && m.family !== family()) return false;
      if (runtime() !== 'all' && m.runtime !== runtime()) return false;
      if (artefactOnly() && !(m.artifact_url || m.has_artifact || m.artifact)) return false;
      return !needle || `${m.name} ${m.key} ${m.publisher || ''} ${m.parameters || ''}`.toLowerCase().includes(needle);
    });
    return matched.sort((a, b) => (FITS.has(a.fit?.fit) ? 0 : 1) - (FITS.has(b.fit?.fit) ? 0 : 1) || a.name.localeCompare(b.name));
  });

  return {
    items, hardware, budget, installs, installsByModel, error, setError, busy, refresh,
    families, counts, visible,
    filters: { search, setSearch, scope, setScope, family, setFamily, runtime, setRuntime, artefactOnly, setArtefactOnly },
  };
}
