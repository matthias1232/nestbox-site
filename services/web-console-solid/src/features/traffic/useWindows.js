// The master switches: a temporary window per firewall, and observation mode.
//
// A switch is not a setting. It is a time-bounded window that expires on its own
// and is recorded, which is why every action here takes a duration and nothing
// here can be left on by accident (ADR: a window is the safe "turn it off for a
// minute"). The OpenShell supervisor is listed but never switchable: it carries
// the sandbox's API and MCP forwarding, so switching it off would not open
// anything, it would cut the sandbox off from its own tools.

import { createSignal } from 'solid-js';
import * as policies from '../../api/policies.js';

/** The channels an operator may switch, in the order the panel shows them. */
export const CHANNELS = [
  {
    id: 'proxy',
    label: 'Web and API egress',
    description: 'HTTPS and HTTP through the proxy, including the model endpoints.',
  },
  { id: 'dns', label: 'DNS', description: 'Name lookups, before any connection is opened.' },
  { id: 'mcp', label: 'MCP', description: 'JSON-RPC to MCP servers the sandbox is attached to.' },
  { id: 'gateway', label: 'Model gateway', description: 'The appliance model endpoint.' },
];

/** The supervisor, shown so the panel is complete and honest about the exception. */
export const ALWAYS_ON = {
  id: 'openshell',
  label: 'OpenShell supervisor',
  description:
    'Always on. It carries the sandbox’s API and MCP forwarding; switching it off would not open traffic, it would cut the sandbox off from its tools.',
};

export const DURATIONS = [15, 30, 60];

/** Why a switch was flipped, written into the window and the audit trail. */
const REASON = 'Master switch from the live traffic console';

export function useWindows() {
  const [windows, setWindows] = createSignal([]);
  const [loading, setLoading] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);

  async function load() {
    setLoading(true);
    setError(null);
    try {
      const answer = await policies.windows();
      setWindows(answer.items || []);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoading(false);
    }
  }

  /** The window covering one channel now, if any. `all` covers every channel. */
  function stateFor(channel) {
    return (
      windows().find((item) => item.channel === 'all' || item.channel === channel) || null
    );
  }

  /** Whether observation mode is on for anything: the console must say so. */
  const observing = () => windows().some((item) => item.mode === 'audit');

  async function flip(channel, minutes, mode) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await policies.openWindow(null, minutes, REASON, mode, channel);
      setNotice(
        mode === 'audit'
          ? `Observation mode on ${channel} for ${minutes} minutes: requests pass and are recorded as blocks.`
          : `${channel} is open for ${minutes} minutes.`,
      );
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function revoke(id) {
    setBusy(true);
    setError(null);
    try {
      await policies.closeWindow(id);
      setNotice('The window is closed.');
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return {
    windows, loading, busy, error, notice,
    load, flip, revoke, stateFor, observing,
    setError,
  };
}
