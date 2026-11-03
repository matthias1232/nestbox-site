// One catalogue entry as a card: name, size facts, the fit badge for the chosen node and two buttons.
import { Show } from 'solid-js';
import FitBadge from '../../components/FitBadge.jsx';
import { gib } from '../../components/ResourceBars.jsx';

// Props: model, installed (number of nodes holding it), onDeploy(model), onOpen(model).
export default function ModelCard(props) {
  const m = () => props.model;
  const needs = () => {
    const parts = [];
    if (m().min_vram_bytes) parts.push(`${gib(m().min_vram_bytes)} GPU`);
    if (m().min_ram_bytes) parts.push(`${gib(m().min_ram_bytes)} RAM`);
    return parts.length ? parts.join(' · ') : 'no hard gate';
  };
  return (
    <article class="model-card">
      <header>
        <strong>{m().name}</strong>
        <Show when={m().is_remote}><span class="badge info tiny-badge"><i />Cloud</span></Show>
      </header>
      <p class="muted small mono">{m().key}</p>
      <p class="small">
        {m().parameters || '—'}
        {m().family ? ` · ${m().family}` : ''}
        {m().license ? ` · ${m().license}` : ''}
      </p>
      <p class="small mono muted">Needs {needs()}</p>
      <div class="model-card-fit">
        <FitBadge fit={m().fit || {}} />
        <span class="muted small">{m().fit?.reason}</span>
      </div>
      <footer>
        <span class="muted small">
          {props.installed ? `Installed on ${props.installed} node${props.installed === 1 ? '' : 's'}` : 'Not installed'}
        </span>
        <span class="model-card-actions">
          <button type="button" class="primary tiny" disabled={m().is_remote} onClick={() => props.onDeploy(m())}>Deploy</button>
          <button type="button" class="secondary tiny" onClick={() => props.onOpen(m())}>Open</button>
        </span>
      </footer>
    </article>
  );
}
