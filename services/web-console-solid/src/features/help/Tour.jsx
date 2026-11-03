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

// Distance between the highlighted region and the card, and the smallest distance to an edge.
const GAP = 12;
const MARGIN = 8;

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

// The nearest ancestor that scrolls, so a step follows its region inside the sidebar or a page.
function scrollParent(element) {
  let node = element ? element.parentElement : null;
  while (node && node !== document.body && node !== document.documentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if (overflow === 'auto' || overflow === 'scroll') return node;
    node = node.parentElement;
  }
  return null;
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

  // The region the current step highlights, or null when the shell does not have it.
  function targetElement() {
    const current = TOUR_STEPS[step()];
    return current ? document.querySelector(current.selector) : null;
  }

  // Re-read the region's real box. A region that is not laid out yet counts as missing, so a
  // zero-sized spotlight never flashes at the top-left corner.
  function measure() {
    const element = targetElement();
    if (!element) {
      setRect(null);
      return;
    }
    const box = element.getBoundingClientRect();
    setRect(
      box.width > 0 || box.height > 0
        ? { left: box.left, top: box.top, width: box.width, height: box.height }
        : null,
    );
  }

  // Follow the region: measure now, again once the layout has settled (two frames, plus 60 ms
  // for late fonts or data), and on every resize, scroll and layout change (V20).
  createEffect(() => {
    if (step() < 0) return;
    let firstFrame = 0;
    let secondFrame = 0;
    let settle = 0;
    const follow = () => measure();

    measure();
    firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(measure);
    });
    settle = setTimeout(measure, 60);

    window.addEventListener('resize', follow);
    // Capture catches the scroll of any nested container, the scrolling one included.
    document.addEventListener('scroll', follow, true);
    const scroller = scrollParent(targetElement());
    if (scroller) scroller.addEventListener('scroll', follow, { passive: true });
    // Content that loads, collapses or appears changes the layout without a resize event.
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(follow) : null;
    if (observer) observer.observe(document.body);

    onCleanup(() => {
      cancelAnimationFrame(firstFrame);
      cancelAnimationFrame(secondFrame);
      clearTimeout(settle);
      window.removeEventListener('resize', follow);
      document.removeEventListener('scroll', follow, true);
      if (scroller) scroller.removeEventListener('scroll', follow);
      if (observer) observer.disconnect();
    });
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
  // Place the card beside the highlighted region, below when there is no room beside it, and
  // never over the region itself. Without a region the card keeps its centred position.
  const cardStyle = () => {
    const r = rect();
    if (!r) return {};
    const width = card ? card.offsetWidth : 340;
    const height = card ? card.offsetHeight : 190;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const clamp = (value, max) => Math.min(Math.max(MARGIN, value), Math.max(MARGIN, max));
    const left = clamp(r.left + r.width / 2 - width / 2, vw - width - MARGIN);
    const top = (value) => clamp(value, vh - height - MARGIN);

    // Beside the region when it fits on either side: that covers nothing of it.
    const right = r.right + GAP;
    if (right + width <= vw - MARGIN) return { left: `${right}px`, top: `${top(r.top)}px`, transform: 'none' };
    const besideLeft = r.left - GAP - width;
    if (besideLeft >= MARGIN) return { left: `${besideLeft}px`, top: `${top(r.top)}px`, transform: 'none' };

    // No room beside it: below when that fits, otherwise above.
    const below = r.bottom + GAP;
    if (below + height <= vh - MARGIN) return { left: `${left}px`, top: `${below}px`, transform: 'none' };
    const above = r.top - GAP - height;
    if (above >= MARGIN) return { left: `${left}px`, top: `${above}px`, transform: 'none' };

    // The region is larger than the screen: keep the card on its roomier side.
    const roomier = vh - MARGIN - r.bottom >= r.top - MARGIN ? below : above;
    return { left: `${left}px`, top: `${top(roomier)}px`, transform: 'none' };
  };

  return (
    <Show when={step() >= 0 && current()}>
      {/* The shade only dims: clicks reach the console underneath, and Escape ends the tour. */}
      <div class="tour-shade" />
      <Show when={rect()}>
        <div class="tour-spot" style={{ left: `${rect().left}px`, top: `${rect().top}px`, width: `${rect().width}px`, height: `${rect().height}px` }} aria-hidden="true" />
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
