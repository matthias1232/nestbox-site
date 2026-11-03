// Tour: the five-step first-run overlay over the shell. Runs once per user (the server keeps
// `shell-v1` in ui_tours_seen) and again when the help drawer asks for a restart.
// Mounted once by the Shell.
import { Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import '../../styles/help.css';
import { onKey } from '../../state/ui.jsx';
import * as helpApi from '../../api/help.js';
import { TOUR_ID, TOUR_STEPS } from './tourSteps.js';
import { tourRequests, trapFocus } from './useHelp.js';

// A fallback memory in this browser when the server cannot be asked: a UI preference only.
const LOCAL_KEY = `nestbox:tour:${TOUR_ID}`;

function seenLocally() {
  try {
    return localStorage.getItem(LOCAL_KEY) === '1';
  } catch {
    return false;
  }
}

function rememberLocally() {
  try {
    localStorage.setItem(LOCAL_KEY, '1');
  } catch {
    // Storage may be blocked; the server record is the real one.
  }
}

export default function Tour() {
  const [step, setStep] = createSignal(-1);
  const [rect, setRect] = createSignal(null);
  let card;
  let handled = tourRequests();

  onMount(async () => {
    try {
      const seen = await helpApi.toursSeen();
      if (!seen.includes(TOUR_ID)) setStep(0);
    } catch {
      // The list is not available: fall back to this browser's memory.
      if (!seenLocally()) setStep(0);
    }
  });

  // A restart request from the help drawer.
  createEffect(() => {
    const count = tourRequests();
    if (count !== handled) {
      handled = count;
      setStep(0);
    }
  });

  // Highlight the region of the current step; a missing region shows the card alone.
  function measure() {
    const current = TOUR_STEPS[step()];
    const element = current ? document.querySelector(current.selector) : null;
    setRect(element ? element.getBoundingClientRect() : null);
  }
  createEffect(() => {
    if (step() < 0) return;
    measure();
    window.addEventListener('resize', measure);
    onCleanup(() => window.removeEventListener('resize', measure));
  });
  createEffect(() => {
    if (step() < 0) return;
    const stopEscape = onKey('Escape', finish);
    const stopTrap = card ? trapFocus(card) : () => {};
    onCleanup(() => {
      stopEscape();
      stopTrap();
    });
  });

  // End the tour (finished or skipped) and remember it.
  function finish() {
    setStep(-1);
    rememberLocally();
    helpApi.markTour(TOUR_ID).catch(() => {});
  }

  function next() {
    if (step() >= TOUR_STEPS.length - 1) finish();
    else setStep(step() + 1);
  }

  const current = () => TOUR_STEPS[step()];
  // Place the card beside the highlighted region, or in the middle of the screen.
  const cardStyle = () => {
    const r = rect();
    if (!r) return {};
    const left = Math.min(Math.max(8, r.left), window.innerWidth - 348);
    const below = r.bottom + 12;
    const top = below + 190 < window.innerHeight ? below : Math.max(8, r.top - 190);
    return { left: `${left}px`, top: `${top}px`, transform: 'none' };
  };

  return (
    <Show when={step() >= 0 && current()}>
      <div class="tour-shade" onClick={finish} />
      <Show when={rect()}>
        <div class="tour-spot" style={{ left: `${rect().left - 4}px`, top: `${rect().top - 4}px`, width: `${rect().width + 8}px`, height: `${rect().height + 8}px` }} aria-hidden="true" />
      </Show>
      <div ref={card} class="help-dialog tour-card" role="dialog" aria-modal="true" aria-label="Console tour" style={cardStyle()}>
        <span class="eyebrow">Step {step() + 1} of {TOUR_STEPS.length}</span>
        <h2>{current().title}</h2>
        <p>{current().text}</p>
        <div class="tour-actions">
          <button class="ghost" onClick={finish}>Skip tour</button>
          <button class="primary" onClick={next}>{step() >= TOUR_STEPS.length - 1 ? 'Finish' : 'Next'}</button>
        </div>
      </div>
    </Show>
  );
}
