// The builder: one page, six sections, and a draft that is only a recipe.
//
// It exists because a recipe names things the API validates — a baseline, tool
// keys the catalog knows for that baseline, agent packs that baseline supports,
// limits, a node, and roots that exist. A free-text form would let an operator
// type a name nobody serves and learn about it from a 409; every field here is
// a choice from what the server returned, and the only text is the recipe's own
// name and the container paths.
//
// Nothing is stored until "Save recipe". After that the recipe has an id, a
// revision, and two actions of its own: build the image, create a sandbox.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import * as packs from '../../api/packs.js';
import * as recipes from '../../api/recipes.js';
import * as nodesApi from '../../api/nodes.js';
import * as persistent from '../../api/persistent.js';
import * as mcpApi from '../../api/mcp.js';
import * as templatesApi from '../../api/imageTemplates.js';
import StatusBadge from '../../components/StatusBadge.jsx';
import '../../styles/builder.css';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const DEFAULT_RESOURCES = {
  memory_limit: '1g',
  cpu_limit: 1,
  gpu_count: 0,
  pids_limit: 256,
};

export default function BuilderPage() {
  const [baselines, setBaselines] = createSignal([]);
  const [packsForBaseline, setPacksForBaseline] = createSignal([]);
  const [toolsForBaseline, setToolsForBaseline] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [roots, setRoots] = createSignal([]);
  const [catalogsLoaded, setCatalogsLoaded] = createSignal(false);

  const [name, setName] = createSignal('');
  const [baseline, setBaseline] = createSignal('');
  const [tools, setTools] = createSignal([]);
  const [agents, setAgents] = createSignal([]);
  const [resources, setResources] = createSignal({ ...DEFAULT_RESOURCES });
  const [node, setNode] = createSignal('local');
  const [mounts, setMounts] = createSignal([]);

  // Distribution packages, repositories, MCP assignments and the template the
  // selection can be saved as. All four are part of the recipe, so the review
  // section counts them and the save sends them.
  const [extraPackages, setExtraPackages] = createSignal([]);
  const [packageQuery, setPackageQuery] = createSignal('');
  const [packageHits, setPackageHits] = createSignal([]);
  const [packageState, setPackageState] = createSignal(null);
  const [repositories, setRepositories] = createSignal([]);
  const [repositoryDraft, setRepositoryDraft] = createSignal({
    name: '', url: '', suite: '', components: '', signing_key: '',
  });
  const [mcpServers, setMcpServers] = createSignal([]);
  const [mcpKeys, setMcpKeys] = createSignal([]);
  const [templates, setTemplates] = createSignal([]);
  const [templateChoice, setTemplateChoice] = createSignal('');
  const [templateForm, setTemplateForm] = createSignal({
    name: '', description: '', is_default: false, is_shared: true,
  });

  const [saved, setSaved] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);

  onMount(async () => {
    try {
      const [baselineAnswer, nodeAnswer, rootAnswer] = await Promise.all([
        packs.baselines(),
        nodesApi.list(),
        persistent.list(),
      ]);
      const baselineList = baselineAnswer.items || [];
      setBaselines(baselineList);
      const recommended = baselineList.find((entry) => entry.recommended) || baselineList[0];
      if (recommended) setBaseline(recommended.key);

      const nodeList = nodeAnswer.items || [];
      setNodes(nodeList);
      const local = nodeList.find((entry) => entry.is_local) || nodeList[0];
      if (local) setNode(local.name);

      setRoots(rootAnswer.items || []);

      const [serverAnswer, templateAnswer, defaultAnswer] = await Promise.all([
        mcpApi.list(),
        templatesApi.list(),
        templatesApi.defaultTemplate(),
      ]);
      setMcpServers(serverAnswer.items || []);
      setTemplates(templateAnswer.items || []);
      const fallback = defaultAnswer.template || null;
      if (fallback) {
        applyTemplateValues(fallback);
        setTemplateChoice(fallback.id);
        setNotice(
          `Loaded the default template "${fallback.name}": ${fallback.extra_packages.length} packages, ` +
          `${fallback.repositories.length} repositories, ${fallback.mcp_server_keys.length} MCP servers.`,
        );
      }

      await loadForBaseline(baseline());
    } catch (failure) {
      setError(failure.message);
    } finally {
      setCatalogsLoaded(true);
    }
  });

  // The catalog depends on the baseline: a tool is resolved to the package name
  // of the baseline's manager, and a pack declares which baselines it supports.
  async function loadForBaseline(key) {
    if (!key) return;
    try {
      const [packAnswer, toolAnswer] = await Promise.all([
        packs.agentPacks(key),
        packs.toolCatalog(key),
      ]);
      setPacksForBaseline(packAnswer.items || []);
      setToolsForBaseline(toolAnswer.items || []);
      // A selection the new baseline does not offer is dropped, with a sentence
      // rather than silently: the recipe must not contain a tool nobody serves.
      const offered = new Set((toolAnswer.items || []).map((tool) => tool.key));
      const dropped = tools().filter((tool) => !offered.has(tool));
      setTools(tools().filter((tool) => offered.has(tool)));
      if (dropped.length) {
        setNotice(`Removed ${dropped.join(', ')}: not available on ${key}.`);
      }
      const supported = new Set((packAnswer.items || []).map((pack) => pack.key));
      const lostAgents = agents().filter((agent) => !supported.has(agent.pack_key));
      setAgents(agents().filter((agent) => supported.has(agent.pack_key)));
      if (lostAgents.length) {
        setNotice(`Removed ${lostAgents.length} agent(s) this baseline does not support.`);
      }
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function changeBaseline(key) {
    setBaseline(key);
    setNotice(null);
    await loadForBaseline(key);
  }

  function toggleTool(key) {
    setTools(tools().includes(key) ? tools().filter((entry) => entry !== key) : [...tools(), key]);
  }

  function addAgent(pack) {
    if (!pack) return;
    const existing = agents().filter((agent) => agent.pack_key === pack.key).length;
    setAgents([
      ...agents(),
      {
        pack_key: pack.key,
        instance: existing ? `${pack.key}-${existing + 1}` : pack.key,
        home_mode: 'shared',
        persistence: pack.requires_gpu ? 'persistent' : 'ephemeral',
        port: pack.ports && pack.ports.length ? pack.ports[0] : null,
      },
    ]);
  }

  function patchAgent(index, patch) {
    setAgents(agents().map((agent, position) => (position === index ? { ...agent, ...patch } : agent)));
  }

  function removeAgent(index) {
    setAgents(agents().filter((_, position) => position !== index));
  }

  // The live catalogue: the answer comes from the baseline's own package index,
  // so it changes when the distribution changes and is never a seeded list.
  async function searchPackages() {
    const fragment = packageQuery().trim();
    if (!fragment || !baseline()) return;
    setPackageState({ busy: true, error: null, detail: null });
    try {
      const answer = await packs.catalogue(baseline(), fragment);
      setPackageHits(answer.items || []);
      setPackageState({
        busy: false,
        error: answer.available ? null : answer.detail,
        detail: answer.detail,
      });
    } catch (failure) {
      setPackageHits([]);
      setPackageState({ busy: false, error: failure.message, detail: null });
    }
  }

  function addPackage(packageName) {
    if (!packageName || extraPackages().includes(packageName)) return;
    setExtraPackages([...extraPackages(), packageName]);
  }

  function removePackage(packageName) {
    setExtraPackages(extraPackages().filter((entry) => entry !== packageName));
  }

  function addRepository() {
    const draft = repositoryDraft();
    if (!draft.name.trim() || !draft.url.trim()) return;
    setRepositories([
      ...repositories(),
      {
        name: draft.name.trim().toLowerCase(),
        url: draft.url.trim(),
        suite: draft.suite.trim() || null,
        components: draft.components.split(/[\s,]+/).filter(Boolean),
        signing_key: draft.signing_key.trim() || null,
        credential_ref: null,
      },
    ]);
    setRepositoryDraft({ name: '', url: '', suite: '', components: '', signing_key: '' });
  }

  function removeRepository(index) {
    setRepositories(repositories().filter((_, position) => position !== index));
  }

  function toggleMcp(key) {
    setMcpKeys(mcpKeys().includes(key)
      ? mcpKeys().filter((entry) => entry !== key)
      : [...mcpKeys(), key]);
  }

  // Copy a template's selection into the draft. The name, agents, resources and
  // mounts stay: a template says what is installed, not how big the sandbox is.
  function applyTemplateValues(template) {
    if (!template) return;
    if (template.baseline_key) setBaseline(template.baseline_key);
    setTools(template.tools || []);
    setExtraPackages(template.extra_packages || []);
    setRepositories(template.repositories || []);
    setMcpKeys(template.mcp_server_keys || []);
  }

  async function applyTemplate() {
    const chosen = templates().find((entry) => entry.id === templateChoice());
    if (!chosen) return;
    applyTemplateValues(chosen);
    if (saved()) {
      setBusy(true);
      setError(null);
      try {
        const fresh = await recipes.applyTemplate(saved().id, chosen.id);
        setSaved(fresh);
        setNotice(
          `Template "${chosen.name}" applied to revision ${fresh.revision}. The previous image no ` +
          'longer matches this selection; build again.',
        );
      } catch (failure) {
        setError(failure.message);
      } finally {
        setBusy(false);
      }
    } else {
      setNotice(`Template "${chosen.name}" loaded into the draft.`);
    }
    await loadForBaseline(chosen.baseline_key || baseline());
  }

  // A template is edited as a whole because the server replaces the selection;
  // sending the fields we are not changing keeps that explicit.
  async function patchTemplate(template, patch) {
    setBusy(true);
    setError(null);
    try {
      await templatesApi.update(template.id, {
        name: template.name,
        description: template.description,
        baseline_key: template.baseline_key,
        tools: template.tools,
        extra_packages: template.extra_packages,
        repositories: template.repositories,
        mcp_server_keys: template.mcp_server_keys,
        is_default: template.is_default,
        is_shared: template.is_shared,
        ...patch,
      });
      const answer = await templatesApi.list();
      setTemplates(answer.items || []);
      setNotice(`Template "${template.name}" updated.`);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function removeTemplate(template) {
    if (!window.confirm(`Delete the template "${template.name}"? Recipes that used it keep their copy.`)) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await templatesApi.remove(template.id);
      const answer = await templatesApi.list();
      setTemplates(answer.items || []);
      if (templateChoice() === template.id) setTemplateChoice('');
      setNotice(`Template "${template.name}" deleted.`);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function saveTemplate() {
    const form = templateForm();
    setBusy(true);
    setError(null);
    try {
      const answer = await templatesApi.create({
        name: form.name.trim(),
        description: form.description.trim(),
        baseline_key: baseline(),
        tools: tools(),
        extra_packages: extraPackages(),
        repositories: repositories(),
        mcp_server_keys: mcpKeys(),
        is_default: form.is_default,
        is_shared: form.is_shared,
      });
      const listAnswer = await templatesApi.list();
      setTemplates(listAnswer.items || []);
      setTemplateChoice(answer.id);
      setTemplateForm({ name: '', description: '', is_default: false, is_shared: true });
      setNotice(
        `Template "${answer.name}" saved${answer.is_default ? ' as the default' : ''}` +
        `${answer.is_shared ? ' and shared with the environment' : ' (private)'}.`,
      );
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  function addMount(root) {
    if (!root) return;
    if (mounts().some((mount) => mount.root_id === root.id)) return;
    setMounts([
      ...mounts(),
      { root_id: root.id, mount_path: `/workspace/${root.slug}`, read_only: false },
    ]);
  }

  const nodeBudget = createMemo(() => nodes().find((entry) => entry.name === node())?.budget || null);

  const problems = createMemo(() => {
    const list = [];
    if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(name().trim())) {
      list.push('A recipe name is a lowercase slug (letters, digits, hyphens).');
    }
    if (!baseline()) list.push('Choose a baseline.');
    if (!node()) list.push('Choose a node.');
    return list;
  });

  const canSave = createMemo(() => problems().length === 0 && !busy() && !saved());

  async function save() {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const answer = await recipes.create({
        name: name().trim(),
        baseline_key: baseline(),
        tools: tools(),
        extra_packages: extraPackages(),
        repositories: repositories(),
        mcp_server_keys: mcpKeys(),
        agents: agents().map((agent) => ({
          pack_key: agent.pack_key,
          instance: agent.instance,
          home_mode: agent.home_mode,
          persistence: agent.persistence,
          port: agent.port || null,
        })),
        resources: resources(),
        project_mounts: mounts().map((mount) => ({
          root_id: mount.root_id,
          mount_path: mount.mount_path,
          read_only: mount.read_only,
        })),
        node: node(),
      });
      setSaved(answer);
      setNotice(
        `Recipe "${answer.name}" saved as revision ${answer.revision}. Build it to get an image, ` +
        'then create a sandbox from that image.',
      );
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function refresh() {
    if (!saved()) return;
    try {
      const fresh = await recipes.get(saved().id);
      setSaved(fresh);
      if (fresh.state === 'built') {
        setNotice(`Built: ${fresh.image_ref}. Create a sandbox from it when you are ready.`);
      }
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function build() {
    setBusy(true);
    setError(null);
    try {
      const answer = await recipes.build(saved().id);
      setSaved(answer.recipe || saved());
      setNotice(
        answer.reused
          ? 'This content already has an image; the tag is reused instead of rebuilt.'
          : `Build queued as job ${answer.job_id}. The state moves from building to built or failed.`,
      );
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function instantiate() {
    setBusy(true);
    setError(null);
    try {
      const sandbox = await recipes.instantiate(saved().id, `${saved().name}-${Date.now() % 10000}`);
      setNotice(`Sandbox "${sandbox.name}" created from the recipe image on ${node()}.`);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="page builder-page">
      <PageHeader
        title="Build a sandbox image"
        subtitle={<>A recipe names a baseline, its tools, its agents, the limits, a node, and the roots to mount. The image is built from it; the sandbox is created from the image.</>}
        actions={<>
          <Show when={saved()}>
            <span class="muted small">rev {saved().revision}</span>
            <StatusBadge status={saved().state} />
            <button class="secondary" onClick={refresh} disabled={busy()}>Refresh</button>
          </Show>
          <HelpTip anchor="getting-started" />
        </>}
      />

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Show when={catalogsLoaded()} fallback={<p class="muted">Loading the catalog…</p>}>
        <div class="builder-sections">
          <section class="builder-section">
            <h2>1 · Baseline</h2>
            <p class="muted small">The image everything else is added to.</p>
            <For each={baselines()}>
              {(entry) => (
                <label class="builder-choice">
                  <input
                    type="radio"
                    name="baseline"
                    checked={baseline() === entry.key}
                    onChange={() => changeBaseline(entry.key)}
                    disabled={!!saved()}
                  />
                  <span>
                    {entry.name}
                    <Show when={entry.recommended}><span class="muted small"> · recommended</span></Show>
                    <Show when={!entry.openshell_ready}>
                      <span class="muted small"> · not supported by NVIDIA OpenShell</span>
                    </Show>
                  </span>
                </label>
              )}
            </For>
          </section>

          <section class="builder-section">
            <h2>2 · Tools</h2>
            <p class="muted small">
              Names resolve per package manager. A tool this baseline does not package is not
              offered.
            </p>
            <div class="builder-tools">
              <For each={toolsForBaseline()}>
                {(tool) => (
                  <label class={`chip ${tools().includes(tool.key) ? 'active' : ''}`}>
                    <input
                      type="checkbox"
                      checked={tools().includes(tool.key)}
                      disabled={!!saved() || tool.available === false}
                      onChange={() => toggleTool(tool.key)}
                    />
                    {tool.name || tool.key}
                    <Show when={tool.package}>
                      <span class="muted small"> · {tool.package}</span>
                    </Show>
                  </label>
                )}
              </For>
            </div>
          </section>


          <section class="builder-section">
            <h2>3 · Distribution packages</h2>
            <p class="muted small">
              Live from this baseline's own package index. A selected package is installed during
              the image build, and the build fails if it is not in the finished image.
            </p>
            <div class="builder-add">
              <input
                class="builder-input"
                placeholder="nginx, postgresql, ffmpeg…"
                value={packageQuery()}
                onInput={(event) => setPackageQuery(event.currentTarget.value)}
                onKeyDown={(event) => { if (event.key === 'Enter') searchPackages(); }}
              />
              <button class="secondary" disabled={!baseline() || !packageQuery().trim()} onClick={searchPackages}>
                Search {baseline() ? '' : '(choose a baseline first)'}
              </button>
            </div>
            <Show when={packageState()?.busy}><p class="muted small">Asking the package index…</p></Show>
            <Show when={packageState()?.error}>
              <p class="notice small">{packageState().error}</p>
            </Show>
            <Show when={packageHits().length}>
              <p class="muted small">{packageHits().length} result(s) · {packageState()?.detail}</p>
              <div class="builder-tools">
                <For each={packageHits()}>
                  {(hit) => (
                    <label class="chip">
                      <input
                        type="checkbox"
                        checked={extraPackages().includes(hit.name)}
                        onChange={() => (extraPackages().includes(hit.name) ? removePackage(hit.name) : addPackage(hit.name))}
                      />
                      <span class="mono small">{hit.name}</span>
                      <Show when={hit.summary}><span class="muted small"> · {hit.summary}</span></Show>
                    </label>
                  )}
                </For>
              </div>
            </Show>
            <Show when={extraPackages().length} fallback={<p class="muted small">No extra package selected.</p>}>
              <p class="muted small">Selected for the post-image install:</p>
              <For each={extraPackages()}>
                {(packageName) => (
                  <div class="builder-agent">
                    <span class="mono small">{packageName}</span>
                    <span />
                    <span />
                    <button class="secondary" disabled={!!saved()} onClick={() => removePackage(packageName)}>Remove</button>
                  </div>
                )}
              </For>
            </Show>
          </section>

          <section class="builder-section">
            <h2>4 · Repositories</h2>
            <p class="muted small">
              A private repository is registered in the image before the packages are installed: the
              signing key is written to a keyring, so an install from it needs no extra step in the
              sandbox.
            </p>
            <label class="builder-field">
              Name (lowercase slug)
              <input class="builder-input" value={repositoryDraft().name}
                     onInput={(event) => setRepositoryDraft({ ...repositoryDraft(), name: event.currentTarget.value })} />
            </label>
            <label class="builder-field">
              URL (HTTPS)
              <input class="builder-input" placeholder="https://packages.example.com/apt"
                     value={repositoryDraft().url}
                     onInput={(event) => setRepositoryDraft({ ...repositoryDraft(), url: event.currentTarget.value })} />
            </label>
            <label class="builder-field">
              Suite (APT, e.g. stable)
              <input class="builder-input" value={repositoryDraft().suite}
                     onInput={(event) => setRepositoryDraft({ ...repositoryDraft(), suite: event.currentTarget.value })} />
            </label>
            <label class="builder-field">
              Components (space-separated, e.g. main contrib)
              <input class="builder-input" value={repositoryDraft().components}
                     onInput={(event) => setRepositoryDraft({ ...repositoryDraft(), components: event.currentTarget.value })} />
            </label>
            <label class="builder-field">
              Public signing key (ASCII-armoured, optional)
              <textarea class="builder-input" rows="3" value={repositoryDraft().signing_key}
                        onInput={(event) => setRepositoryDraft({ ...repositoryDraft(), signing_key: event.currentTarget.value })} />
            </label>
            <div class="controls">
              <button class="secondary" disabled={!!saved() || !repositoryDraft().name.trim() || !repositoryDraft().url.trim()}
                      onClick={addRepository}>
                Register repository
              </button>
            </div>
            <For each={repositories()}>
              {(repository, index) => (
                <p class="muted small">
                  <span class="mono">{repository.name}</span> · {repository.url}
                  {repository.suite ? ` · ${repository.suite}` : ''}
                  {repository.signing_key ? ' · signed' : ' · unsigned'}
                  {' '}
                  <button class="secondary" disabled={!!saved()} onClick={() => removeRepository(index())}>Remove</button>
                </p>
              )}
            </For>
            <Show when={repositoryDraft().signing_key}>
              <p class="muted small">Private credentials belong in the vault; a key here is public material.</p>
            </Show>
          </section>

          <section class="builder-section">
            <h2>5 · MCP servers</h2>
            <p class="muted small">
              Attached when a sandbox is created from this recipe, so the agent starts with the
              servers it needs instead of an operator adding them afterwards.
            </p>
            <Show when={mcpServers().length} fallback={<p class="muted small">No MCP server is registered yet.</p>}>
              <div class="builder-tools">
                <For each={mcpServers()}>
                  {(server) => (
                    <label class="chip">
                      <input type="checkbox" disabled={!!saved()}
                             checked={mcpKeys().includes(server.key)}
                             onChange={() => toggleMcp(server.key)} />
                      <span class="mono small">{server.key}</span>
                      <span class="muted small"> · {server.transport}</span>
                    </label>
                  )}
                </For>
              </div>
            </Show>
          </section>

          <section class="builder-section">
            <h2>10 · Templates</h2>
            <p class="muted small">
              Save this selection — tools, packages, repositories and MCP servers — as a template a
              colleague can reuse. Marking one as default makes it the starting point for new recipes.
            </p>
            <Show when={templates().length}>
              <div class="builder-add">
                <select value={templateChoice()} onChange={(event) => setTemplateChoice(event.currentTarget.value)}>
                  <option value="">Choose a template…</option>
                  <For each={templates()}>
                    {(template) => (
                      <option value={template.id}>
                        {(template.is_default ? '★ ' : '') + template.name + (template.is_shared ? '' : ' (private)')}
                      </option>
                    )}
                  </For>
                </select>
                <button class="secondary" disabled={!templateChoice() || busy()} onClick={applyTemplate}>
                  Apply
                </button>
              </div>
            </Show>
            <For each={templates()}>
              {(template) => (
                <div class="builder-agent">
                  <span class="small">
                    {template.is_default ? '★ ' : ''}{template.name}
                    {template.is_shared ? '' : ' (private)'}
                  </span>
                  <span class="muted small">
                    {template.extra_packages.length} pkgs · {template.repositories.length} repos ·{' '}
                    {template.mcp_server_keys.length} MCP
                  </span>
                  <button class="secondary" disabled={busy() || template.is_default}
                          onClick={() => patchTemplate(template, { is_default: true })}>
                    Make default
                  </button>
                  <button class="secondary" disabled={busy()}
                          onClick={() => patchTemplate(template, { is_shared: !template.is_shared })}>
                    {template.is_shared ? 'Unshare' : 'Share'}
                  </button>
                  <button class="secondary" disabled={busy()} onClick={() => removeTemplate(template)}>
                    Delete
                  </button>
                </div>
              )}
            </For>
            <label class="builder-field">
              New template name
              <input class="builder-input" value={templateForm().name}
                     onInput={(event) => setTemplateForm({ ...templateForm(), name: event.currentTarget.value })} />
            </label>
            <label class="builder-field">
              Description
              <input class="builder-input" value={templateForm().description}
                     onInput={(event) => setTemplateForm({ ...templateForm(), description: event.currentTarget.value })} />
            </label>
            <label class="builder-choice muted small">
              <input type="checkbox" checked={templateForm().is_default}
                     onChange={(event) => setTemplateForm({ ...templateForm(), is_default: event.currentTarget.checked })} />
              Environment default
            </label>
            <label class="builder-choice muted small">
              <input type="checkbox" checked={templateForm().is_shared}
                     onChange={(event) => setTemplateForm({ ...templateForm(), is_shared: event.currentTarget.checked })} />
              Shared with the environment
            </label>
            <div class="controls">
              <button class="secondary" disabled={!templateForm().name.trim() || busy() || !!saved()} onClick={saveTemplate}>
                {busy() ? 'Saving…' : 'Save as template'}
              </button>
            </div>
          </section>

          <section class="builder-section">
            <h2>6 · Agents</h2>
            <p class="muted small">
              One row per agent instance. A persistent home is a directory on the node and
              survives the container.
            </p>
            <For each={agents()}>
              {(agent, index) => (
                <div class="builder-agent">
                  <strong>{agent.pack_key}</strong>
                  <input
                    value={agent.instance}
                    disabled={!!saved()}
                    onInput={(event) => patchAgent(index(), { instance: event.currentTarget.value })}
                  />
                  <select
                    value={agent.home_mode}
                    disabled={!!saved()}
                    onChange={(event) => patchAgent(index(), { home_mode: event.currentTarget.value })}
                  >
                    <option value="shared">shared home</option>
                    <option value="isolated">isolated home</option>
                  </select>
                  <select
                    value={agent.persistence}
                    disabled={!!saved()}
                    onChange={(event) => patchAgent(index(), { persistence: event.currentTarget.value })}
                  >
                    <option value="ephemeral">ephemeral</option>
                    <option value="persistent">persistent</option>
                  </select>
                  <button class="secondary" disabled={!!saved()} onClick={() => removeAgent(index())}>
                    Remove
                  </button>
                </div>
              )}
            </For>
            <div class="builder-add">
              <For each={packsForBaseline()}>
                {(pack) => (
                  <button class="secondary" disabled={!!saved()} onClick={() => addAgent(pack)}>
                    + {pack.name}
                  </button>
                )}
              </For>
            </div>
          </section>

          <section class="builder-section">
            <h2>7 · Resources</h2>
            <div class="builder-resources">
              <label class="builder-field">
                Memory
                <input
                  class="builder-input"
                  value={resources().memory_limit}
                  disabled={!!saved()}
                  onInput={(event) => setResources({ ...resources(), memory_limit: event.currentTarget.value })}
                />
              </label>
              <label class="builder-field">
                CPU
                <input
                  class="builder-input"
                  type="number"
                  min="0.1"
                  step="0.1"
                  value={resources().cpu_limit}
                  disabled={!!saved()}
                  onInput={(event) => setResources({ ...resources(), cpu_limit: Number(event.currentTarget.value) })}
                />
              </label>
              <label class="builder-field">
                GPUs
                <input
                  class="builder-input"
                  type="number"
                  min="0"
                  value={resources().gpu_count}
                  disabled={!!saved()}
                  onInput={(event) => setResources({ ...resources(), gpu_count: Number(event.currentTarget.value) })}
                />
              </label>
              <label class="builder-field">
                PIDs
                <input
                  class="builder-input"
                  type="number"
                  min="32"
                  value={resources().pids_limit}
                  disabled={!!saved()}
                  onInput={(event) => setResources({ ...resources(), pids_limit: Number(event.currentTarget.value) })}
                />
              </label>
            </div>
          </section>

          <section class="builder-section">
            <h2>8 · Node</h2>
            <select value={node()} disabled={!!saved()} onChange={(event) => setNode(event.currentTarget.value)}>
              <For each={nodes()}>
                {(entry) => <option value={entry.name}>{entry.display_name || entry.name}</option>}
              </For>
            </select>
            <Show when={nodeBudget()}>
              <p class="muted small">
                {nodeBudget().ram_free_bytes
                  ? `${(nodeBudget().ram_free_bytes / 2 ** 30).toFixed(1)} GiB of RAM free`
                  : 'no free RAM measured'}
                {' · '}
                {nodeBudget().cpu_cores} cores · {nodeBudget().sandboxes} sandboxes
              </p>
            </Show>
          </section>

          <section class="builder-section">
            <h2>9 · Project mounts</h2>
            <p class="muted small">
              Optional. The host path is derived by the server; a mounted root is bound into the
              sandbox at the path you type.
            </p>
            <Show
              when={roots().length}
              fallback={<p class="muted small">No persistent root exists yet. Create one under Persistent data.</p>}
            >
              <select disabled={!!saved()} onChange={(event) => {
                const chosen = roots().find((root) => root.id === event.currentTarget.value);
                addMount(chosen);
                event.currentTarget.value = '';
              }}>
                <option value="">Add a root…</option>
                <For each={roots()}>
                  {(root) => <option value={root.id}>{root.slug} ({root.kind})</option>}
                </For>
              </select>
            </Show>
            <For each={mounts()}>
              {(mount, index) => (
                <div class="builder-agent">
                  <span class="mono small">
                    {roots().find((root) => root.id === mount.root_id)?.slug || mount.root_id}
                  </span>
                  <input
                    value={mount.mount_path}
                    disabled={!!saved()}
                    onInput={(event) => setMounts(mounts().map((entry, position) =>
                      position === index() ? { ...entry, mount_path: event.currentTarget.value } : entry))}
                  />
                  <label class="muted small">
                    <input
                      type="checkbox"
                      checked={mount.read_only}
                      disabled={!!saved()}
                      onChange={(event) => setMounts(mounts().map((entry, position) =>
                        position === index() ? { ...entry, read_only: event.currentTarget.checked } : entry))}
                    />
                    read-only
                  </label>
                  <button class="secondary" disabled={!!saved()}
                          onClick={() => setMounts(mounts().filter((_, position) => position !== index()))}>
                    Remove
                  </button>
                </div>
              )}
            </For>
          </section>

          <section class="builder-section">
            <h2>11 · Review and save</h2>
            <label class="builder-field">
              Recipe name
              <input
                class="builder-input"
                placeholder="dev-box"
                value={name()}
                disabled={!!saved()}
                onInput={(event) => setName(event.currentTarget.value)}
              />
            </label>
            <p class="muted small">
              {baseline() || 'no baseline'} · {tools().length} tools · {extraPackages().length} packages ·{' '}
              {repositories().length} repositories · {mcpKeys().length} MCP · {agents().length} agents ·{' '}
              {mounts().length} mounts · on {node()}
            </p>
            <For each={problems()}>{(problem) => <p class="muted small">· {problem}</p>}</For>
            <div class="controls">
              <button class="primary" disabled={!canSave()} onClick={save}>
                {busy() ? 'Saving…' : 'Save recipe'}
              </button>
              <Show when={saved()}>
                <button class="secondary" disabled={busy()} onClick={build}>Build image</button>
                <button class="secondary" disabled={busy() || saved().state !== 'built'} onClick={instantiate}>
                  Create sandbox
                </button>
              </Show>
            </div>
            <Show when={saved()}>
              <p class="muted small">
                Image: <span class="mono">{saved().image_ref || 'not built yet'}</span>
                <Show when={saved().build_error}> · {saved().build_error}</Show>
              </p>
            </Show>
          </section>
        </div>
      </Show>
    </section>
  );
}
