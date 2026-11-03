// Content privacy: what is inspected on its way into and out of a sandbox, what
// happens to a finding, and the incidents the policy recorded.
//
// The screen decides. It does not inspect anything itself: an inspection here is
// one test call with the saved policy, not a conversation, and an agent tool is
// what runs the same filter in production.

import { For, Show, createSignal, onMount } from 'solid-js';
import { usePrivacy } from './usePrivacy.js';
import * as sandboxApi from '../../api/sandboxes.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

// Noise-level severity to a colour that means the same thing as elsewhere in the
// console; an unknown value stays neutral instead of looking healthy.
function severityTone(severity) {
  switch (String(severity || '').toLowerCase()) {
    case 'critical':
    case 'high':
      return 'bad';
    case 'warning':
    case 'warn':
    case 'medium':
      return 'warn';
    case 'info':
    case 'low':
      return 'info';
    default:
      return 'muted';
  }
}

// What the filter did with a match. `allow` is green because nothing happened;
// it is not an endorsement, which is why the page spells the action out.
function actionTone(action) {
  switch (String(action || '').toLowerCase()) {
    case 'block':
    case 'blocked':
      return 'bad';
    case 'redact':
    case 'replace':
    case 'replaced':
      return 'warn';
    case 'allow':
    case 'allowed':
      return 'ok';
    default:
      return 'muted';
  }
}

function whenLabel(value) {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
}

function deliveredLabel(value) {
  if (value === true) return 'yes';
  if (value === false) return 'no';
  return '—';
}

export default function PrivacyPage(props) {
  const privacy = usePrivacy();
  const [sandboxes, setSandboxes] = createSignal([]);
  const [sandboxListError, setSandboxListError] = createSignal(null);
  const [selected, setSelected] = createSignal('');
  const [direction, setDirection] = createSignal('user_to_agent');
  const [destination, setDestination] = createSignal('');
  const [content, setContent] = createSignal('');
  const [purgeDays, setPurgeDays] = createSignal(30);
  const isAdmin = () => props.role === 'admin';

  const scopeName = () =>
    sandboxes().find((sandbox) => sandbox.id === selected())?.name || 'this sandbox';

  onMount(async () => {
    await privacy.load('');
    try {
      setSandboxes((await sandboxApi.list({ limit: 200 })).items || []);
    } catch (failure) {
      // The picker is a core control of this page, so its failure is said out
      // loud; the environment default above stays usable either way.
      setSandboxListError(failure.message || 'The sandbox list could not be read.');
    }
  });

  async function chooseSandbox(id) {
    setSelected(id);
    await privacy.load(id);
  }

  function submitInspect(event) {
    event.preventDefault();
    privacy.inspect({ direction: direction(), destination: destination(), content: content() });
  }

  return (
    <div class="page">
      <PageHeader
        title="Content privacy"
        subtitle={<>What is inspected on its way into a sandbox and on its way out to a cloud provider. A finding is recorded masked; the original text is never stored, and the console cannot show it back to you.</>}
        actions={<>
          <HelpTip anchor="policies" />
        </>}
      />

      <PrivacyDlpSection />

      <Show when={privacy.error()}><p class="error">{privacy.error()}</p></Show>
      <Show when={privacy.notice()}><p class="notice">{privacy.notice()}</p></Show>

      <section class="editor">
        <h3>Applies to</h3>
        <label class="field">
          Scope
          <select
            class="input"
            value={selected()}
            onChange={(event) => chooseSandbox(event.currentTarget.value)}
          >
            <option value="">Environment default (a sandbox without settings of its own)</option>
            <For each={sandboxes()}>
              {(sandbox) => <option value={sandbox.id}>{sandbox.name}</option>}
            </For>
          </select>
        </label>
        <Show when={sandboxListError()}>
          <p class="warn-note small">{sandboxListError()}</p>
        </Show>
        <p class="muted small">
          A sandbox with its own row uses it instead of the environment default. Inspecting
          needs a sandbox, because it is that sandbox's policy which answers.
        </p>
      </section>

      <Show
        when={privacy.view()}
        fallback={
          <p class="muted">
            {privacy.loading()
              ? 'Loading privacy settings…'
              : 'No settings were returned for this scope.'}
          </p>
        }
      >
        <section class="editor">
          <h3>Mode</h3>
          <div class="form-grid">
            <For each={privacy.MODES}>
              {(mode) => (
                <label class="draft-card">
                  <span class="mono small">
                    <input
                      type="radio"
                      name="privacy-mode"
                      value={mode.id}
                      checked={privacy.draft().mode === mode.id}
                      disabled={!isAdmin()}
                      onChange={() => privacy.setMode(mode.id)}
                    />{' '}
                    {mode.id}
                  </span>
                  <p class="muted small">{mode.hint}</p>
                </label>
              )}
            </For>
          </div>
          <Show when={privacy.draft().mode === 'off'}>
            <p class="muted small">
              With the mode off, the detectors below keep their saved actions but nothing runs.
            </p>
          </Show>
        </section>

        <section class="editor">
          <h3>Detectors</h3>
          <Show
            when={privacy.detectors().length}
            fallback={
              <p class="muted small">
                The server returned no detector catalog for this environment. Nothing can be
                inspected until it has one.
              </p>
            }
          >
            <div class="table-scroll">
              <table class="table">
                <thead>
                  <tr>
                    <th>Detector</th>
                    <th>Default action</th>
                    <th>Inspected</th>
                    <th>Action</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={privacy.detectors()}>
                    {(detector) => (
                      <tr>
                        <td>
                          <div>{detector.label}</div>
                          <div class="muted small mono">{detector.key}</div>
                        </td>
                        <td class="mono small">{detector.default_action || '—'}</td>
                        <td>
                          <input
                            type="checkbox"
                            aria-label={`Inspect ${detector.label}`}
                            checked={privacy.draft().detectors.includes(detector.key)}
                            disabled={!isAdmin()}
                            onChange={(event) =>
                              privacy.toggleDetector(detector.key, event.currentTarget.checked)
                            }
                          />
                        </td>
                        <td>
                          <select
                            class="input"
                            aria-label={`Action for ${detector.label}`}
                            value={privacy.actionFor(detector)}
                            disabled={!isAdmin()}
                            onChange={(event) =>
                              privacy.setAction(detector.key, event.currentTarget.value)
                            }
                          >
                            <For each={privacy.ACTIONS}>
                              {(action) => <option value={action.id}>{action.label}</option>}
                            </For>
                          </select>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
          <p class="muted small">
            An action applies while its detector is ticked: <span class="mono">block</span>{' '}
            refuses the request, <span class="mono">redact</span> and{' '}
            <span class="mono">replace</span> transform it, <span class="mono">allow</span>{' '}
            leaves the match in place. <span class="mono">allow</span> on a secret detector is
            refused by the server, so a save can fail with that message rather than quietly
            weakening the policy.
          </p>
        </section>

        <section class="editor">
          <h3>Cloud egress and retention</h3>
          <label class="small">
            <input
              type="checkbox"
              checked={privacy.draft().block_cloud_egress}
              disabled={!isAdmin()}
              onChange={(event) => privacy.setBlockCloudEgress(event.currentTarget.checked)}
            />{' '}
            Block cloud egress when the policy blocks the content
          </label>
          <p class="muted small">
            Off means only the per-detector action applies; on, content the policy blocks does
            not leave for a cloud provider at all.
          </p>

          <label class="field">
            Retention (days)
            <input
              class="input"
              type="number"
              min="1"
              max="3650"
              value={privacy.draft().retention_days}
              disabled={!isAdmin()}
              onInput={(event) => privacy.setRetentionDays(event.currentTarget.value)}
            />
          </label>
          <p class="muted small">
            How long the server keeps incidents. The purge below takes its own day count.
          </p>
          <PrivacyExtraSettings privacy={privacy} isAdmin={isAdmin()} />
          <label class="small">
            Hosts to check in addition to the model hosts
            <textarea
              class="input"
              rows="3"
              placeholder="one host per line"
              value={(privacy.draft().enforce_hosts || []).join('\n')}
              disabled={!isAdmin()}
              onInput={(event) => privacy.setEnforceHosts(event.currentTarget.value)}
            />
          </label>
          <p class="muted small">
            The model providers are always checked. Everything else a sandbox reaches — a browser, a package
            registry — is only recorded, unless you name it here: rewriting a shop's form because a value looked
            like an e-mail is breakage, not protection.
          </p>

          <Show
            when={isAdmin()}
            fallback={
              <p class="muted small">
                Only an admin can change these settings. For you this page is read-only.
              </p>
            }
          >
            <button class="primary" disabled={privacy.busy()} onClick={() => privacy.save()}>
              {selected() ? 'Save sandbox settings' : 'Save environment default'}
            </button>
          </Show>
        </section>
      </Show>

      <section class="editor">
        <h3>Test the filter</h3>
        <p class="muted small">
          One test call with the saved policy: no agent is started, no conversation happens and
          nothing is sent to a provider. The answer shows what the filter would do.
        </p>
        <Show
          when={selected()}
          fallback={
            <p class="warn-note small">
              Choose a sandbox above. Inspection needs a sandbox id, because the sandbox's own
              policy decides the answer.
            </p>
          }
        >
          <form onSubmit={submitInspect}>
            <div class="form-grid">
              <label class="field">
                Direction
                <select
                  class="input"
                  value={direction()}
                  onChange={(event) => setDirection(event.currentTarget.value)}
                >
                  <For each={privacy.DIRECTIONS}>
                    {(option) => <option value={option.id}>{option.label}</option>}
                  </For>
                </select>
              </label>
              <label class="field">
                Destination (optional)
                <input
                  class="input"
                  value={destination()}
                  placeholder="api.example.com"
                  onInput={(event) => setDestination(event.currentTarget.value)}
                />
              </label>
            </div>
            <label class="field">
              Content
              <textarea
                rows="5"
                spellcheck={false}
                value={content()}
                placeholder="Paste the prompt or the request body to test."
                onInput={(event) => setContent(event.currentTarget.value)}
              />
            </label>
            <button class="primary" type="submit" disabled={privacy.inspecting()}>
              {privacy.inspecting() ? 'Inspecting…' : 'Inspect'}
            </button>
          </form>
        </Show>

        <Show when={privacy.inspectError()}>
          <p class="error small">{privacy.inspectError()}</p>
        </Show>

        <Show
          when={privacy.result()}
          fallback={<p class="muted small">No inspection has been run for this sandbox yet.</p>}
        >
          {(result) => (
            <div class="draft-card">
              <p>
                <span class={`badge ${result().permitted ? 'ok' : 'bad'}`}>
                  <i />
                  {result().permitted ? 'permitted' : 'blocked'}
                </span>{' '}
                <span class="muted small">action</span>{' '}
                <span class="mono small">{result().action}</span>
              </p>
              <h4>Content that would be sent</h4>
              <pre class="logs mono small">{result().transformed_content || '(empty)'}</pre>
              <h4>Findings</h4>
              <Show
                when={result().findings?.length}
                fallback={<p class="muted small">No detector matched this text.</p>}
              >
                <div class="table-scroll">
                  <table class="table dense">
                    <thead>
                      <tr>
                        <th>Detector</th>
                        <th>Masked value</th>
                        <th>Action</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={result().findings}>
                        {(finding) => (
                          <tr>
                            <td class="mono small">{finding.detector}</td>
                            <td class="mono small">{finding.masked_value || '—'}</td>
                            <td>
                              <span class={`badge ${actionTone(finding.action)}`}>
                                <i />
                                {finding.action}
                              </span>
                            </td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                </div>
              </Show>
            </div>
          )}
        </Show>
      </section>

      <section class="editor">
        <h3>Recent incidents</h3>
        <p class="muted small">
          The last 50 findings for {selected() ? scopeName() : 'the environment default'}.
          Incidents keep the masked value only; the original value is never stored.
        </p>

        <Show
          when={selected()}
          fallback={
            <p class="muted small">
              Incidents belong to a sandbox. Choose one above to see its findings; the
              environment default has no incidents of its own.
            </p>
          }
        >
          <Show when={privacy.incidentsError()}>
            <p class="error small">{privacy.incidentsError()}</p>
          </Show>
          <Show when={privacy.incidentsLoading()}>
            <p class="muted small">Loading incidents…</p>
          </Show>
          <Show when={!privacy.incidentsLoading() && !privacy.incidentsError()}>
            <Show
              when={privacy.incidents().length}
              fallback={<p class="muted small">No incidents have been recorded for this sandbox yet.</p>}
            >
              <div class="table-scroll">
                <table class="table dense">
                  <thead>
                    <tr>
                      <th>When</th>
                      <th>Detector</th>
                      <th>Severity</th>
                      <th>Action</th>
                      <th>Masked value</th>
                      <th>Destination</th>
                      <th>Cloud delivered</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={privacy.incidents()}>
                      {(incident) => (
                        <tr>
                          <td class="small">{whenLabel(incident.created_at)}</td>
                          <td class="mono small">{incident.detector}</td>
                          <td>
                            <span class={`badge ${severityTone(incident.severity)}`}>
                              <i />
                              {incident.severity || 'unknown'}
                            </span>
                          </td>
                          <td>
                            <span class={`badge ${actionTone(incident.action)}`}>
                              <i />
                              {incident.action}
                            </span>
                          </td>
                          <td class="mono small">{incident.masked_value || '—'}</td>
                          <td class="small">{incident.destination || '—'}</td>
                          <td class="small">{deliveredLabel(incident.cloud_delivery)}</td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Show>

          <Show
            when={isAdmin()}
            fallback={<p class="muted small">Only an admin can purge incidents.</p>}
          >
            <div class="form-grid">
              <label class="field">
                Purge incidents older than (days)
                <input
                  class="input"
                  type="number"
                  min="1"
                  max="3650"
                  value={purgeDays()}
                  onInput={(event) => setPurgeDays(Number(event.currentTarget.value))}
                />
              </label>
              <Show
                when={privacy.pendingPurge()}
                fallback={
                  <div class="controls">
                    <button
                      class="tiny danger"
                      disabled={privacy.busy()}
                      onClick={() => privacy.askPurge()}
                    >
                      Purge incidents
                    </button>
                  </div>
                }
              >
                <div class="controls">
                  <button
                    class="tiny danger"
                    disabled={privacy.busy()}
                    onClick={() => privacy.purge(purgeDays())}
                  >
                    Purge older than {purgeDays()} days — cannot be undone
                  </button>
                  <button
                    class="tiny secondary"
                    disabled={privacy.busy()}
                    onClick={() => privacy.cancelPurge()}
                  >
                    Cancel
                  </button>
                </div>
              </Show>
            </div>
            <p class="muted small">
              Deletes the incident rows of this sandbox older than the day count. Those rows
              hold masked values and the metadata in the table above; the original content is
              not part of them.
            </p>
          </Show>
        </Show>
      </section>
    </div>
  );
}
