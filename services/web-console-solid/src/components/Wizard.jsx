import { For } from 'solid-js';

// A multi-step page: numbered steps on the left, one step body, Back / Next / Finish below.
// Nothing is written before Finish; Escape does nothing because a wizard is a page, not a dialog.
// Props: steps [{id,label}], step (id), onStep(id), canNext(), onFinish(), finishLabel, busy, children.
export default function Wizard(props) {
  const index = () => Math.max(0, props.steps.findIndex((s) => s.id === props.step));
  const isLast = () => index() === props.steps.length - 1;
  const go = (delta) => {
    const target = props.steps[index() + delta];
    if (target) props.onStep(target.id);
  };
  return (
    <div class="wizard">
      <ol class="wizard-steps">
        <For each={props.steps}>
          {(step, i) => (
            <li aria-current={i() === index() ? 'step' : undefined} classList={{ done: i() < index() }}>
              <span class="wizard-num mono">{i() + 1}</span>
              <span>{step.label}</span>
            </li>
          )}
        </For>
      </ol>
      <div class="wizard-main">
        <div class="wizard-body">{props.children}</div>
        <div class="wizard-footer">
          <button type="button" class="secondary" disabled={index() === 0 || props.busy} onClick={() => go(-1)}>
            Back
          </button>
          {isLast() ? (
            <button type="button" class="primary" disabled={props.busy || (props.canNext && !props.canNext())} onClick={() => props.onFinish()}>
              {props.busy ? 'Working…' : props.finishLabel || 'Finish'}
            </button>
          ) : (
            <button type="button" class="primary" disabled={props.busy || (props.canNext && !props.canNext())} onClick={() => go(1)}>
              Next
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
