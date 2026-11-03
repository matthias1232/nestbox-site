// The shared live streams: at most one state stream and one telemetry stream per window
// (contract C16, pattern J6).
//
// The state stream opens after sign-in and reconnects with a growing delay. The telemetry
// stream opens only while some screen asked for it (`wantTelemetry`, reference counted).
//
// Every window mounts this provider (main.jsx wraps the whole app), so a page or a pop-out
// never opens a stream of its own: it subscribes with `on(name, handler)`, and the one
// stream here answers every subscriber. That is the whole budget: a browser speaks
// HTTP/1.1 to the console and keeps six connections per host for all tabs, so a window
// that opens one stream per panel runs out of connections and looks frozen.

import { createContext, createEffect, createMemo, createSignal, onCleanup, useContext } from 'solid-js';
import { openStream } from '../api/events.js';
import { SHELLLESS, parsePath } from '../routing.js';
import { useSession } from './session.jsx';
import {
  STATE_EVENTS, TELEMETRY_EVENTS, createRefCount, createRegistry, nextDelay, payloadFor,
} from './liveCore.js';

const STATE_PATH = '/api/v1/events/stream';
const TELEMETRY_PATH = '/api/v1/telemetry/stream';

const STATE_NAMES = new Set(STATE_EVENTS);

const LiveContext = createContext();

// Keep a stream open and reopen it after a failure; returns a function that stops it for good.
function keepOpen(path, names, registry, setStatus) {
  let stopStream = null;
  let timer = null;
  let delay = 0;
  let alive = true;

  function connect() {
    if (!alive) return;
    timer = null;
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
          // A source can report the same failure twice, so only one reconnect may be pending.
          if (!alive || timer) return;
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
    timer = null;
    stopStream?.();
    stopStream = null;
    setStatus?.('off');
  };
}

// Whether this window renders without the shell (a terminal, desktop or log pop-out).
// A pop-out owns its window, so it opens the state stream only if its page asks for it.
function opensWithoutShell() {
  return SHELLLESS.has(parsePath(window.location.pathname, window.location.search).kind);
}

// Provide `useLive()` to the tree. Mount it inside the session provider.
export function LiveProvider(props) {
  const session = useSession();
  const registry = createRegistry();
  const [status, setStatus] = createSignal('off');

  const shellless = opensWithoutShell();
  const stateDemand = createRefCount();
  const [stateAsked, setStateAsked] = createSignal(false);
  stateDemand.watch(setStateAsked);
  // A shell window always holds the state stream; a pop-out only while a page listens.
  // The memo is what keeps a second subscriber from restarting the open stream.
  const stateNeeded = createMemo(() => !shellless || stateAsked());

  const telemetryDemand = createRefCount();
  const [telemetryAsked, setTelemetryAsked] = createSignal(false);
  telemetryDemand.watch(setTelemetryAsked);

  // The state stream follows the sign-in and, in a pop-out, the first listener.
  createEffect(() => {
    if (!session.identity() || !stateNeeded()) return;
    const stop = keepOpen(STATE_PATH, STATE_EVENTS, registry, setStatus);
    onCleanup(stop);
  });

  // The telemetry stream follows demand: one connection while any screen wants it.
  createEffect(() => {
    if (!session.identity() || !telemetryAsked()) return;
    const stop = keepOpen(TELEMETRY_PATH, TELEMETRY_EVENTS, registry, null);
    onCleanup(stop);
  });

  const value = {
    // Listen to one event name; returns the function that stops listening.
    on(name, handler) {
      const stop = registry.on(name, handler);
      // Only a state name raises the state stream's demand: a pop-out that reads
      // telemetry alone must not open a connection it never uses.
      const release = STATE_NAMES.has(name) ? stateDemand.acquire() : null;
      let released = false;
      return () => {
        if (released) return;
        released = true;
        stop();
        release?.();
      };
    },
    // Ask for the telemetry stream; returns the function that gives the request back.
    wantTelemetry() {
      return telemetryDemand.acquire();
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
