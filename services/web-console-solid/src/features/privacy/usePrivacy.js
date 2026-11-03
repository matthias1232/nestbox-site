// State for the content-privacy screen: the environment default or one
// sandbox's own policy, the detectors it runs, a single test of the filter, the
// incidents it recorded, and the admin purge.
//
// No JSX here on purpose (`features/AGENTS.md`): the page renders, this file
// decides what "saving" and "inspecting" mean and what an error says.

import { createSignal } from 'solid-js';
import * as api from '../../api/privacy.js';

// One sentence per mode, because the mode is the decision an operator is
// actually making. The ids are the server's, not a display name.
const MODES = [
  { id: 'off', hint: 'Nothing is inspected: content passes through unchanged and no findings are recorded.' },
  { id: 'observe', hint: 'Every enabled detector runs and its findings are recorded, but the content is sent unchanged.' },
  { id: 'review', hint: 'A finding is replaced before the content is sent; nothing is blocked.' },
  { id: 'strict', hint: 'Secrets and financial identifiers block the request instead of being replaced.' },
];

// The per-detector actions. `allow` is a deliberate weakening, which is why the
// page says out loud that the server refuses it for a secret detector.
const ACTIONS = [
  { id: 'block', label: 'block' },
  { id: 'redact', label: 'redact' },
  { id: 'replace', label: 'replace' },
  { id: 'pseudonymize', label: 'pseudonymize (the model sees a placeholder, the agent gets the value back)' },
  { id: 'allow', label: 'allow' },
];

const DIRECTIONS = [
  { id: 'user_to_agent', label: 'user to agent (content entering the sandbox)' },
  { id: 'agent_to_cloud', label: 'agent to cloud (content leaving for a provider)' },
];

// The draft is the whole settings object, not a field: the server replaces the
// row, so a partial write would silently clear something the form did not carry.
function draftFrom(settings) {
  return {
    mode: settings?.mode || 'off',
    detectors: [...(settings?.detectors || [])],
    actions: { ...(settings?.actions || {}) },
    block_cloud_egress: Boolean(settings?.block_cloud_egress),
    retention_days: Number.isFinite(settings?.retention_days) ? settings.retention_days : 30,
    enforce_hosts: [...(settings?.enforce_hosts || [])],
  };
}

export function usePrivacy() {
  const [view, setView] = createSignal(null);
  const [draft, setDraft] = createSignal(draftFrom(null));
  const [incidents, setIncidents] = createSignal([]);
  const [sandboxId, setSandboxId] = createSignal('');
  const [loading, setLoading] = createSignal(true);
  const [incidentsLoading, setIncidentsLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [inspecting, setInspecting] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [incidentsError, setIncidentsError] = createSignal(null);
  const [inspectError, setInspectError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [result, setResult] = createSignal(null);
  const [pendingPurge, setPendingPurge] = createSignal(false);

  // The catalog belongs to the server; the console never hard-codes a detector.
  const detectors = () => view()?.detectors || [];

  function clearMessages() {
    setError(null);
    setNotice(null);
  }

  /// Load the incidents of one sandbox. Its own error, because a failed list
  /// must not blank the settings that were read successfully.
  async function loadIncidents(target) {
    if (!target) {
      setIncidents([]);
      setIncidentsError(null);
      return;
    }
    setIncidentsLoading(true);
    setIncidentsError(null);
    try {
      const answer = await api.incidents(target, 50);
      setIncidents(Array.isArray(answer) ? answer : answer?.items || []);
    } catch (failure) {
      setIncidents([]);
      setIncidentsError(failure.message || 'The incidents could not be read.');
    } finally {
      setIncidentsLoading(false);
    }
  }

  /// Read the settings for the environment default (empty target) or one
  /// sandbox, then the incidents that belong to that sandbox.
  async function load(target = '') {
    setSandboxId(target);
    setLoading(true);
    setView(null);
    setIncidents([]);
    setIncidentsError(null);
    setResult(null);
    setInspectError(null);
    setPendingPurge(false);
    clearMessages();
    try {
      const answer = await api.settings(target || undefined);
      setView(answer);
      setDraft(draftFrom(answer?.settings));
    } catch (failure) {
      setError(failure.message || 'The privacy settings could not be read.');
    } finally {
      setLoading(false);
    }
    await loadIncidents(target);
  }

  const setMode = (mode) => setDraft({ ...draft(), mode });

  const toggleDetector = (key, on) =>
    setDraft({
      ...draft(),
      detectors: on
        ? [...new Set([...draft().detectors, key])]
        : draft().detectors.filter((candidate) => candidate !== key),
    });

  const setAction = (key, action) =>
    setDraft({ ...draft(), actions: { ...draft().actions, [key]: action } });

  const setBlockCloudEgress = (value) =>
    setDraft({ ...draft(), block_cloud_egress: Boolean(value) });

  const setRetentionDays = (value) => setDraft({ ...draft(), retention_days: value });
  // One host per line, blank lines dropped: the server checks the shape.
  const setEnforceHosts = (value) =>
    setDraft({
      ...draft(),
      enforce_hosts: String(value)
        .split(/[\s,]+/)
        .map((host) => host.trim())
        .filter(Boolean),
    });

  /// The action a detector carries: this sandbox's saved choice, else the
  /// server's default. An unknown detector falls back to `block`, never to a
  /// silent `allow`.
  function actionFor(detector) {
    return draft().actions[detector.key] || detector.default_action || 'block';
  }

  function retentionIsValid() {
    const days = Number(draft().retention_days);
    return Number.isInteger(days) && days >= 1 && days <= 3650;
  }

  async function save() {
    if (!retentionIsValid()) {
      setError('Retention must be a whole number of days between 1 and 3650.');
      return;
    }
    setBusy(true);
    clearMessages();
    try {
      const body = {
        sandbox_id: sandboxId() || null,
        mode: draft().mode,
        detectors: draft().detectors,
        actions: draft().actions,
        block_cloud_egress: draft().block_cloud_egress,
        retention_days: Number(draft().retention_days),
        enforce_hosts: draft().enforce_hosts,
      };
      const known = detectors();
      const answer = await api.saveSettings(body);
      const saved = answer?.settings || body;
      setView({ settings: saved, detectors: answer?.detectors || known });
      setDraft(draftFrom(saved));
      setNotice(sandboxId() ? 'Sandbox settings saved.' : 'Environment default saved.');
    } catch (failure) {
      // A refusal such as "allow on a secret detector" arrives here with the
      // server's message; it is shown as it came.
      setError(failure.message || 'The privacy settings could not be saved.');
    } finally {
      setBusy(false);
    }
  }

  /// One test of the filter. It answers what the policy would do; it sends
  /// nothing to an agent and stores nothing the page can read back.
  async function inspect({ direction, destination, content }) {
    const target = sandboxId();
    if (!target) {
      setInspectError('Choose a sandbox first: inspection answers with a sandbox policy.');
      return;
    }
    if (!content || !content.trim()) {
      setInspectError('Type something to inspect.');
      return;
    }
    setInspecting(true);
    setInspectError(null);
    setResult(null);
    try {
      const body = {
        sandbox_id: target,
        direction,
        ...(destination && destination.trim() ? { destination: destination.trim() } : {}),
        content,
      };
      setResult(await api.inspect(body));
    } catch (failure) {
      setInspectError(failure.message || 'The inspection failed.');
    } finally {
      setInspecting(false);
    }
  }

  /// A destructive action asks once and says what it destroys.
  const askPurge = () => setPendingPurge(true);
  const cancelPurge = () => setPendingPurge(false);

  async function purge(days) {
    const count = Number(days);
    if (!Number.isInteger(count) || count < 1 || count > 3650) {
      setError('The purge day count must be a whole number between 1 and 3650.');
      return;
    }
    setBusy(true);
    setPendingPurge(false);
    clearMessages();
    try {
      const answer = await api.purge(count);
      const removed =
        answer && typeof answer.incidents === 'number'
          ? ` ${answer.incidents} incident(s) removed.`
          : '';
      setNotice(`Purged incidents older than ${count} days.${removed}`);
      await loadIncidents(sandboxId());
    } catch (failure) {
      setError(failure.message || 'The purge failed.');
    } finally {
      setBusy(false);
    }
  }

  return {
    MODES,
    ACTIONS,
    DIRECTIONS,
    view,
    draft,
    detectors,
    incidents,
    sandboxId,
    loading,
    incidentsLoading,
    busy,
    inspecting,
    error,
    incidentsError,
    inspectError,
    notice,
    result,
    pendingPurge,
    load,
    setEnforceHosts,
    setMode,
    toggleDetector,
    setAction,
    setBlockCloudEgress,
    setRetentionDays,
    actionFor,
    save,
    inspect,
    askPurge,
    cancelPurge,
    purge,
  };
}
