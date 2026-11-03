// The sandbox's live topology: who talks to what, right now, with blocked and
// would-be-blocked targets highlighted.
//
// The diagram itself is the one the sandbox deck draws (`features/sandboxes`),
// so there is only one truth about what the stack is and what state each piece
// is in. This panel only re-reads it when telemetry for this sandbox arrives,
// at most every two seconds, so a busy agent does not cause a burst of reads.
import { createSignal, onCleanup, onMount } from 'solid-js';
import { useLive } from '../../state/live.jsx';
import StackTopology from '../sandboxes/StackTopology.jsx';

const DEBOUNCE_MS = 2000;

// The panel: the reason it redraws, then the stack diagram for this sandbox.
export default function TopologyPanel(props) {
  const live = useLive();
  const [rev, setRev] = createSignal(0);
  let timer = null;

  function bump() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      setRev(rev() + 1);
    }, DEBOUNCE_MS);
  }

  onMount(() => {
    // The telemetry stream is the app's one connection; this panel asks for it
    // while it is on screen and hands it back when it leaves.
    const release = live.wantTelemetry();
    const handle = (event) => {
      if (event?.sandbox_id && event.sandbox_id === props.sandbox?.id) bump();
    };
    const stops = [
      live.on('network', handle),
      live.on('dns', handle),
      live.on('inference', handle),
    ];
    onCleanup(() => {
      release();
      for (const stop of stops) stop();
    });
  });
  onCleanup(() => timer && clearTimeout(timer));

  return (
    <section>
      <div class="controls">
        <h3 style="margin:0">Live topology</h3>
        <span class="muted small">re-read while this sandbox talks</span>
      </div>
      <StackTopology sandbox={props.sandbox} revision={rev()} />
    </section>
  );
}
