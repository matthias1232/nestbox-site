// Who is in a sandbox right now, as initials.
//
// Five circles fit any card; beyond that the count matters more than the names, so
// the rest is summed up as "+N" and the full list stays in the tooltips.

import { For, Show } from 'solid-js';

import '../styles/controls.css';

const MAX_SHOWN = 5;

// The first letters of the first two words of a name, or "?" when there is none.
function initialsOf(name) {
  const words = String(name || '').trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return '?';
  const letters = words.slice(0, 2).map((word) => word[0]);
  return letters.join('').toUpperCase();
}

// The tooltip of one person: the name, what they are using, and whether they can act.
function personTitle(person) {
  const name = person?.name || '?';
  const kind = person?.kind || 'terminal';
  return `${name} · ${kind}${person?.view_only ? ' (watching)' : ''}`;
}

export default function PresenceDots(props) {
  const people = () => props.people || [];
  const shown = () => people().slice(0, MAX_SHOWN);
  const hidden = () => Math.max(0, people().length - MAX_SHOWN);

  return (
    <div class="nb-people">
      <For each={shown()}>
        {(person) => (
          <span class="nb-person" title={personTitle(person)}>
            {initialsOf(person.name)}
          </span>
        )}
      </For>
      <Show when={hidden() > 0}>
        <span class="nb-person nb-person-more" title={people().slice(MAX_SHOWN).map(personTitle).join(', ')}>
          +{hidden()}
        </span>
      </Show>
    </div>
  );
}
