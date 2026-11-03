// The model catalogue: find a model, see where it fits, deploy it on one node or many.
//
// Every verdict is the server's (`fit.fit`, `fit.reason`, `fit.bars`); the filters only switch
// verdicts on and off, and the wizard's own preview never replaces the server's answer.
// Install and uninstall are jobs; the Phase 2 note on weights stays at the bottom.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import '../../styles/models.css';
import CapacityOverview from '../../components/CapacityOverview.jsx';
import ModelFitCell from '../../components/ModelFitCell.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import Empty from '../../components/Empty.jsx';
import { gib } from '../../components/ResourceBars.jsx';
import * as nodeApi from '../../api/nodes.js';
import * as models from '../../api/models.js';
import { navigate, paths } from '../../routing.js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import ModelGrantsSection from './ModelGrantsSection.jsx';
import ModelCard from './ModelCard.jsx';
import BulkDeployWizard from './BulkDeployWizard.jsx';
import { useModels, FITS } from './useModels.js';

// The cards/table choice is a UI preference; it has its own key because state/ui.jsx is not ours.
const VIEW_KEY = 'nestbox:models-view';
const readView = () => {
  try { return localStorage.getItem(VIEW_KEY) === 'table' ? 'table' : 'cards'; } catch { return 'cards'; }
};

export default function ModelsPage() {
  const [nodes, setNodes] = createSignal([]);
  // `local` until the node list says otherwise: a page on "no node" would show nothing.
  const [nodeName, setNodeName] = createSignal('local');
  const data = useModels(nodeName);
  const [view, setViewSignal] = createSignal(readView());
  const [wizard, setWizard] = createSignal(null); // null | { modelKey, nodeName }
  const [notice, setNotice] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [syncing, setSyncing] = createSignal(false);
  const [selected, setSelected] = createSignal(new Set());

  const setView = (value) => {
    setViewSignal(value);
    try { localStorage.setItem(VIEW_KEY, value); } catch { /* a blocked store only forgets the choice */ }
  };

  onMount(async () => {
    // URL opener: ?deploy=1&model=<key>&node=<name> opens the wizard preselected (read once).
    const params = new URLSearchParams(window.location.search);
    if (params.get('deploy') === '1') setWizard({ modelKey: params.get('model') || null, nodeName: params.get('node') || null });
    try {
      const list = (await nodeApi.list()).items || [];
      setNodes(list);
      const preferred = params.get('node') && list.find((n) => n.name === params.get('node'));
      const local = preferred || list.find((n) => n.is_local) || list[0];
      if (local) setNodeName(local.name);
    } catch {
      // Without a node list the server still classifies against the local node.
    }
    await data.refresh(false);
  });

  async function probe() {
    const catalog = await data.refresh(true);
    if (catalog) {
      const fitting = (catalog.items || []).filter((m) => FITS.has(m.fit?.fit)).length;
      data.filters.setScope('fits');
      setNotice(`Hardware probed. ${fitting} of ${(catalog.items || []).length} models fit this node; the filter now shows only those.`);
    }
  }

  async function syncCatalog() {
    setSyncing(true);
    setError(null);
    try {
      const result = await models.sync();
      const sum = (field) => (result.sources || []).reduce((total, s) => total + s[field], 0);
      setNotice(`Catalogue synced: ${sum('added')} added, ${sum('updated')} updated. Job ${result.job_id}.`);
      await data.refresh(false);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setSyncing(false);
    }
  }

  async function uninstallSelected() {
    const keys = [...selected()];
    if (!window.confirm(`Uninstall ${keys.length} model(s) from ${nodeName()}?`)) return;
    setError(null);
    const failed = [];
    for (const key of keys) {
      try { await models.uninstall(key, nodeName()); } catch (failure) { failed.push(`${key}: ${failure.message}`); }
    }
    setSelected(new Set());
    if (failed.length) setError(failed.join(' · '));
    else setNotice(`${keys.length} model(s) removed from ${nodeName()}.`);
    await data.refresh(false);
  }

  const reference = (resources) => {
    let max = 0;
    for (const model of data.items()) {
      if (!FITS.has(model.fit?.fit)) continue;
      for (const bar of model.fit.bars || []) {
        if (!resources.includes(bar.resource)) continue;
        max = Math.max(max, bar.recommended || bar.minimum || 0);
      }
    }
    return max;
  };
  const ramReference = createMemo(() => reference(['ram']));
  const vramReference = createMemo(() => reference(['vram', 'unified_memory']));

  const columns = [
    {
      key: 'name', label: 'Model', sortable: true,
      render: (m) => (
        <>
          <strong>{m.name}</strong>
          <Show when={m.is_remote}><span class="badge info tiny-badge"><i />Cloud</span></Show>
          <div class="muted small mono">{m.key}</div>
        </>
      ),
    },
    { key: 'parameters', label: 'Size', render: (m) => <span class="small">{m.parameters}{m.quantization ? ` · ${m.quantization}` : ''}</span> },
    { key: 'runtime', label: 'Runtime', sortable: true },
    { key: 'fit', label: 'Fit', render: (m) => <ModelFitCell fit={m.fit} /> },
    { key: 'installs', label: 'Installed', render: (m) => (data.installsByModel()[m.key] || []).length },
    {
      key: 'actions', label: 'Actions',
      render: (m) => (
        <span class="row-actions">
          <button type="button" class="primary tiny" disabled={m.is_remote} onClick={() => setWizard({ modelKey: m.key, nodeName: nodeName() })}>Deploy</button>
          <button type="button" class="secondary tiny" onClick={() => navigate(paths.model(m.key))}>Open</button>
        </span>
      ),
    },
  ];

  const f = data.filters;
  return (
    <section class="page models-page">
      <Breadcrumb items={[{ label: 'Models' }]} />
      <PageHeader
        title="Models"
        subtitle="What each node can run, what each model needs, and how far apart the two are."
        actions={
          <>
            <label class="fit-against">Fit against:{' '}
              <select
                aria-label="Fit against node"
                value={nodeName()}
                onChange={(event) => { setNodeName(event.currentTarget.value); queueMicrotask(() => data.refresh(false)); }}
              >
                <For each={nodes()}>{(node) => <option value={node.name}>{node.display_name || node.name}</option>}</For>
              </select>
            </label>
            <button type="button" class="secondary" onClick={syncCatalog} disabled={syncing()}>{syncing() ? 'Syncing…' : 'Sync catalogue'}</button>
            <button type="button" class="secondary" onClick={probe} disabled={data.busy()}>{data.busy() ? 'Probing…' : 'Probe hardware again'}</button>
            <button type="button" class="primary" onClick={() => setWizard({ modelKey: null, nodeName: null })}>Deploy model</button>
          </>
        }
      />
      <Show when={error() || data.error()}><p class="error" role="alert">{error() || data.error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Show when={wizard()}>
        <BulkDeployWizard
          models={data.items()}
          modelKey={wizard().modelKey}
          nodeName={wizard().nodeName}
          onClose={() => setWizard(null)}
          onDone={() => data.refresh(false)}
        />
      </Show>

      <Show when={data.hardware()}>
        <CapacityOverview hardware={data.hardware()} items={data.items()} ramReference={ramReference()} vramReference={vramReference()} />
      </Show>
      <Show when={data.budget()}>
        <p class="muted">
          On <strong>{nodeName()}</strong>: {gib(data.budget().ram_total_bytes)} of RAM, of which{' '}
          <strong>{gib(data.budget().ram_reserved_bytes)} reserved</strong> by {data.budget().installs}{' '}
          {data.budget().installs === 1 ? 'install' : 'installs'} and {gib(data.budget().ram_free_bytes || 0)} free.
          {data.budget().vram_total_bytes ? ` VRAM: ${gib(data.budget().vram_reserved_bytes)} of ${gib(data.budget().vram_total_bytes)} reserved.` : ''}
        </p>
      </Show>

      <div class="filters">
        <input placeholder="Search model, key, publisher…" aria-label="Search models" value={f.search()} onInput={(event) => f.setSearch(event.currentTarget.value)} />
        <div class="facet-row">
          <button type="button" class={`chip ${f.scope() === 'all' ? 'active' : ''}`} onClick={() => f.setScope('all')}>All <b>{data.counts().total}</b></button>
          <button type="button" class={`chip ${f.scope() === 'fits' ? 'active' : ''}`} onClick={() => f.setScope('fits')}>Fits this node <b>{data.counts().fits}</b></button>
          <button type="button" class={`chip ${f.scope() === 'not' ? 'active' : ''}`} onClick={() => f.setScope('not')}>Does not fit <b>{data.counts().notFits}</b></button>
          <button type="button" class={`chip ${f.artefactOnly() ? 'active' : ''}`} onClick={() => f.setArtefactOnly(!f.artefactOnly())}>Has artefact</button>
        </div>
        <div class="facet-row">
          <select aria-label="Family" value={f.family()} onChange={(event) => f.setFamily(event.currentTarget.value)}>
            <option value="all">All families</option>
            <For each={data.families()}>{(value) => <option value={value}>{value}</option>}</For>
          </select>
          <select aria-label="Runtime" value={f.runtime()} onChange={(event) => f.setRuntime(event.currentTarget.value)}>
            <option value="all">Any runtime</option>
            <option value="cuda">GPU only</option>
            <option value="hybrid">GPU or CPU</option>
            <option value="cpu">CPU only</option>
          </select>
          <span class="muted small">{data.visible().length} shown</span>
          <span class="view-toggle" role="group" aria-label="View">
            <button type="button" class={`tiny ${view() === 'cards' ? 'primary' : 'secondary'}`} aria-pressed={view() === 'cards'} onClick={() => setView('cards')}>Cards</button>
            <button type="button" class={`tiny ${view() === 'table' ? 'primary' : 'secondary'}`} aria-pressed={view() === 'table'} onClick={() => setView('table')}>Table</button>
          </span>
        </div>
      </div>

      <Show when={view() === 'table' && selected().size}>
        <div class="models-bulkbar" role="toolbar" aria-label="Selected models">
          <strong>{selected().size} selected</strong>
          <button type="button" class="secondary tiny" onClick={() => setWizard({ modelKey: [...selected()][0], nodeName: null })}>Deploy on…</button>
          <button type="button" class="danger tiny" onClick={uninstallSelected}>Uninstall from {nodeName()}</button>
          <button type="button" class="ghost tiny" onClick={() => setSelected(new Set())}>Clear</button>
        </div>
      </Show>

      <Show
        when={data.visible().length}
        fallback={<Empty title="No model matches these filters" hint="Clear a filter, or sync the catalogue." action={<button type="button" class="secondary" onClick={syncCatalog}>Sync catalogue</button>} />}
      >
        <Show
          when={view() === 'table'}
          fallback={
            <div class="model-grid">
              <For each={data.visible()}>
                {(model) => (
                  <ModelCard
                    model={model}
                    installed={(data.installsByModel()[model.key] || []).length}
                    onDeploy={(m) => setWizard({ modelKey: m.key, nodeName: nodeName() })}
                    onOpen={(m) => navigate(paths.model(m.key))}
                  />
                )}
              </For>
            </div>
          }
        >
          <SelectTable columns={columns} rows={data.visible()} rowKey="key" selectable selected={selected()} onSelect={setSelected} />
        </Show>
      </Show>

      <p class="muted small phase-note">
        Install creates a job and records the intent. The artefact download is Phase 2:
        no bytes are fetched yet, so a job does not make a model usable on its own.
      </p>
      <ModelGrantsSection />
    </section>
  );
}
