// Live events tab of the dock: the ticker, newest on top, 200 lines, pause and tone filter.
import { For, Show, createSignal, onCleanup } from 'solid-js';
import { useLive } from '../../state/live.jsx';
import { navigate } from '../../routing.js';
import { lineFor, pushLine, filterTone } from './eventLines.js';

const NAMES = ['sandbox', 'node', 'control', 'job', 'alert', 'models', 'policy_rollout'];
const TONES = ['all', 'bad', 'warn', 'ok', 'info'];

export default function LiveEventsTab() {
  const live = useLive();
  const [lines, setLines] = createSignal([]);
  const [paused, setPaused] = createSignal(false);
  const [tone, setTone] = createSignal('all');
  let seq = 0;

  for (const name of NAMES) {
    onCleanup(live.on(name, (payload) => {
      if (paused()) return;
      seq += 1;
      setLines((list) => pushLine(list, { ...lineFor(name, payload), id: seq, at: new Date().toLocaleTimeString() }, 200));
    }));
  }

  const shown = () => filterTone(lines(), tone());

  return (
    <>
      <div class="dock-tools">
        <For each={TONES}>
          {(t) => (
            <button class="ghost tiny" aria-pressed={tone() === t} onClick={() => setTone(t)}>{t}</button>
          )}
        </For>
        <button class="secondary tiny" onClick={() => setPaused(!paused())}>{paused() ? 'Resume' : 'Pause'}</button>
        <button class="ghost tiny" onClick={() => setLines([])}>Clear</button>
      </div>
      <Show when={shown().length} fallback={<p class="muted">{paused() ? 'Paused.' : 'Waiting for events…'}</p>}>
        <ul class="dock-events">
          <For each={shown()}>
            {(line) => (
              <li class={`dock-event ${line.tone}`}>
                <span class="mono small muted">{line.at}</span>
                <span class={`badge ${line.tone}`}><i />{line.tone}</span>
                <span>{line.text}</span>
                <Show when={line.path}>
                  <button class="ghost tiny" onClick={() => navigate(line.path)}>Open</button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </>
  );
}
