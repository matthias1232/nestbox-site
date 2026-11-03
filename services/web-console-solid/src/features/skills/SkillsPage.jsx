// The skill catalogue is deliberately separate from models and CUDA-X: a skill
// is an agent capability, not a model download or a host runtime component.
// Today skills are the controlled tool catalogue that recipes install. The page
// names that boundary rather than pretending a tool was silently injected into
// a running sandbox.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import * as packs from '../../api/packs.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

export default function SkillsPage() {
  const [items, setItems] = createSignal([]);
  const [search, setSearch] = createSignal('');
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);

  async function load() {
    setError(null);
    try { setItems((await packs.toolCatalog()).items || []); }
    catch (failure) { setError(failure.message); }
  }
  onMount(load);
  const visible = createMemo(() => {
    const needle = search().trim().toLowerCase();
    return items().filter((skill) => !needle || `${skill.name || ''} ${skill.key || ''} ${skill.description || ''}`.toLowerCase().includes(needle));
  });

  return <section class="page">
    <PageHeader title="Skills" subtitle={<>Controlled capabilities agents can receive through a reproducible recipe. A skill never receives a provider secret; it uses the sandbox’s assigned provider reference.</>} actions={<><button class="secondary" onClick={load}>Refresh</button><HelpTip anchor="getting-started" /></>} />
    <p class="notice">To assign a skill today, select it in <b>Build image</b> or a recipe, then create/recreate the sandbox. Direct live injection is intentionally not offered because it would make the running image unauditable.</p>
    <Show when={error()}><p class="error">{error()}</p></Show><Show when={notice()}><p class="notice">{notice()}</p></Show>
    <label class="field"><span>Find a skill</span><input value={search()} placeholder="browser, git, CUDA …" onInput={(e) => setSearch(e.currentTarget.value)} /><small class="muted">Searches the server-owned skill catalogue; no client-side capabilities are invented.</small></label>
    <div class="table-scroll"><table><thead><tr><th>Skill</th><th>Install package</th><th>Purpose</th><th>Assignment</th></tr></thead><tbody><For each={visible()}>{(skill) => <tr><td><b>{skill.name || skill.key}</b><div class="muted small mono">{skill.key}</div></td><td class="mono small">{skill.package || skill.resolved_package || 'built in'}</td><td class="small">{skill.description || 'No description supplied by the catalogue.'}</td><td><button class="tiny" onClick={() => setNotice(`Use “${skill.name || skill.key}” in Build image or a recipe; the resulting image can then be assigned to any sandbox.`)}>How to assign</button></td></tr>}</For></tbody></table></div>
  </section>;
}
