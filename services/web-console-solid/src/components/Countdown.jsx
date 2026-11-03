// A live "time left" label.
//
// `until` null means there is no end at all, which is the difference between "a
// window until midnight" and "no window": the second one must not read as 0:00.

import { createEffect, createSignal, onCleanup, Show } from 'solid-js';

import { formatRemaining, remainingMs } from './countdown.js';
import '../styles/controls.css';

export default function Countdown(props) {
  const [text, setText] = createSignal('');
  let timer = null;
  let announcedFor = null;

  const stop = () => {
    if (timer != null) clearInterval(timer);
    timer = null;
  };

  const tick = () => {
    const left = remainingMs(props.until, Date.now());
    if (left == null) {
      setText('');
      return;
    }
    setText(formatRemaining(left));
    if (left <= 0) {
      stop();
      if (announcedFor !== props.until) {
        announcedFor = props.until;
        props.onExpire?.();
      }
    }
  };

  createEffect(() => {
    const until = props.until;
    stop();
    announcedFor = null;
    if (until == null) {
      setText('');
      return;
    }
    tick();
    timer = setInterval(tick, 1000);
  });

  onCleanup(stop);

  return (
    <Show
      when={props.until != null}
      fallback={<span class="nb-countdown">{props.prefix}unlimited</span>}
    >
      <span class="nb-countdown">{props.prefix}{text()}</span>
    </Show>
  );
}
