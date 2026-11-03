// The workbox's view of the state stream, filtered per sandbox.
//
// Every workbox panel wants the same events: an agent's sign-in changed, the
// proxy mode changed, a window opened, a model became ready. They all arrive on
// the one state stream the app already holds (`state/live.jsx`), so this module
// only filters and fans out; it opens no connection of its own.

import { useLive } from '../../state/live.jsx';

/**
 * Call `callback(event)` for control and model events of one sandbox.
 *
 * The accessor is a function so the caller may pass a signal. Returns a stop
 * function; the shared stream itself stays open for the other subscribers.
 */
export function onControl(sandboxId, callback) {
  const live = useLive();
  const handle = (event) => {
    if (event?.type !== 'sandbox_control_changed' && event?.type !== 'models_changed') return;
    if (event.sandbox_id && event.sandbox_id !== sandboxId()) return;
    callback(event);
  };
  // `control` carries `sandbox_control_changed`, `models` carries `models_changed`.
  const stops = [live.on('control', handle), live.on('models', handle)];
  return () => {
    for (const stop of stops) stop();
  };
}
