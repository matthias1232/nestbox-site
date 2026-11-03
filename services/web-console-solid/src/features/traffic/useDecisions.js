// The decisions view: what is being stopped, and the three ways to open it.
//
// A denial is not a request; it is a destination that was refused, grouped, with
// a count. The operator decides about the destination, and there are exactly
// three answers, so the panel offers exactly three:
//
// * **once** — a rule draft, reviewed and approved (ADR 0011);
// * **for a time** — a traffic window that expires on its own;
// * **permanently** — a policy rule, which is the same draft with no expiry.
//
// Hiding a row is a session-local view choice, never a policy change: a hidden
// target keeps being denied and stays reachable through "show hidden".

import { createSignal } from 'solid-js';
import * as trafficApi from '../../api/traffic.js';
import * as policies from '../../api/policies.js';

/** The channel a denied destination belongs to, from its protocol. */
export function channelFor(protocol) {
  const word = String(protocol || '').toLowerCase();
  if (word === 'dns') return 'dns';
  if (word === 'mcp' || word === 'jsonrpc') return 'mcp';
  return 'proxy';
}

/** A stable key for one target, so hiding survives a refresh. */
export function keyOf(item) {
  return `${item.sandbox_id || 'none'}|${item.host}|${item.port}|${item.protocol}`;
}

export function useDecisions() {
  const [items, setItems] = createSignal([]);
  const [windowSpan, setWindowSpan] = createSignal('1h');
  const [hidden, setHidden] = createSignal(new Set());
  const [showHidden, setShowHidden] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const answer = await trafficApi.decisions({ window: windowSpan() });
      setItems(answer.items || []);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoading(false);
    }
  }

  function hide(item) {
    setHidden((current) => new Set([...current, keyOf(item)]));
  }

  function unhide(item) {
    setHidden((current) => {
      const next = new Set(current);
      next.delete(keyOf(item));
      return next;
    });
  }

  const visible = () =>
    items().filter((item) => showHidden() || !hidden().has(keyOf(item)));
  const hiddenCount = () => items().filter((item) => hidden().has(keyOf(item))).length;

  /** Allow a destination for a bounded time, on its own firewall. */
  async function allowFor(item, minutes) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await policies.openWindow(
        item.sandbox_id,
        minutes,
        `Allow ${item.host} from the decisions view`,
        'bypass',
        channelFor(item.protocol),
      );
      setNotice(`${item.host} is open for ${minutes} minutes on ${channelFor(item.protocol)}.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  /** Draft a rule: the permanent answer, which is reviewed before it applies. */
  async function draft(item) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await policies.draftFromEvent(
        { host: item.host, port: item.port, protocol: item.protocol },
        item.sandbox_id || null,
      );
      setNotice(
        `A rule draft for ${item.host} is waiting under Policies → Pending drafts. It is not enforced yet.`,
      );
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return {
    items, visible, hiddenCount, showHidden, setShowHidden,
    windowSpan, setWindowSpan,
    loading, busy, error, notice,
    load, hide, unhide, allowFor, draft, setError,
  };
}
