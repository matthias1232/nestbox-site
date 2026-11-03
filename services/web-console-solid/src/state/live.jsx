// The shared live stream: one EventSource for the whole app (contract C16, pattern J6).
//
// The state stream opens after sign-in and reconnects with a growing delay. The telemetry
// stream opens only while some screen asked for it (`wantTelemetry`, reference counted).

import { createContext, createEffect, createSignal, onCleanup, useContext } from 'solid-js';
import { openStream } from '../api/events.js';
import { useSession } from './session.jsx';
import { createRegistry, nextDelay, payloadFor } from './liveCore.js';

const STATE_EVENTS = ['sandbox', 'gateway', 'job', 'policy_rollout', 'control', 'node',
  'chat_started', 'chat_delta', 'chat_done', 'models', 'alert'];
const TELEMETRY_EVENTS = ['network', 'dns', 'inference'];

const LiveContext = createContext();

// Keep a stream open and reopen it after a failure; returns a function that stops it for good.
function keepOpen(path, names, registry, setStatus) {
  let stopStream = null;
  let timer = null;
  let delay = 0;
  let alive = true;

  function connect() {
    if (!alive) return;
    stopStream = openStream(
      path,
      names,
      (name, data) => registry.emit(name, payloadFor(name, data)),
      {
        onOpen: () => {
          delay = 0;
          setStatus?.('connected');
        },
        onError: () => {
          // EventSource retries by itself with a fixed pause; we close it and apply ours.
          stopStream?.();
          stopStream = null;
          setStatus?.('reconnecting');
          delay = nextDelay(delay);
          timer = setTimeout(connect, delay);
        },
      },
    );
  }
  connect();
  return () => {
    alive = false;
    if (timer) clearTimeout(timer);
    stopStream?.();
    setStatus?.('off');
  };
}

// Provide `useLive()` to the tree. Mount it inside the session provider.
export function LiveProvider(props) {
  const session = useSession();
  const registry = createRegistry();
  const [status, setStatus] = createSignal('off');
  const [telemetryWanted, setTelemetryWanted] = createSignal(0);

  // The state stream follows the sign-in.
  createEffect(() => {
    if (!session.identity()) return;
    const stop = keepOpen('/api/v1/events/stream', STATE_EVENTS, registry, setStatus);
    onCleanup(stop);
  });

  // The telemetry stream follows demand.
  createEffect(() => {
    if (!session.identity() || telemetryWanted() <= 0) return;
    const stop = keepOpen('/api/v1/telemetry/stream', TELEMETRY_EVENTS, registry, null);
    onCleanup(stop);
  });

  const value = {
    // Listen to one event name; returns the function that stops listening.
    on: (name, handler) => registry.on(name, handler),
    // Ask for the telemetry stream; returns the function that gives the request back.
    wantTelemetry() {
      setTelemetryWanted((n) => n + 1);
      let released = false;
      return () => {
        if (released) return;
        released = true;
        setTelemetryWanted((n) => Math.max(0, n - 1));
      };
    },
    // `connected`, `reconnecting` or `off`.
    status,
  };

  return <LiveContext.Provider value={value}>{props.children}</LiveContext.Provider>;
}

// The live store of the current app.
export function useLive() {
  return useContext(LiveContext);
}
