// What the content policy found in this sandbox, and whether it can check at all.
//
// The same section the Content privacy page shows, narrowed to one sandbox: a
// reader should not have to learn two tables for one question.
import { Show, createResource } from 'solid-js';
import * as privacy from '../../api/privacy.js';
import DlpBadge from '../../components/DlpBadge.jsx';
import PrivacyDlpSection from '../privacy/PrivacyDlpSection.jsx';

// Whether a body can be checked on its way out, by the sandbox's egress mode.
function inlineNote(mode) {
  if (mode === 'transparent') {
    return 'Bodies are checked on their way out for every host this sandbox reaches; an answer that streams back is recorded, and pseudonymised placeholders are put back.';
  }
  if (mode === 'hybrid') {
    return 'Bodies are checked for model hosts only; every other host leaves as a tunnel this layer cannot read.';
  }
  return 'This sandbox reaches the internet without a body-readable proxy, so nothing is inspected inline. Findings appear only when the agent routes through the accounting proxy.';
}

export default function PrivacyTab(props) {
  const [settings] = createResource(() => props.sandbox?.id, (id) => privacy.settings(id));
  const policy = () => settings()?.settings;
  const mode = () => props.sandbox?.egress_mode || 'transparent';

  return (
    <section class="tab-panel">
      <div class="controls">
        <Show when={policy()}>
          <DlpBadge state="clean" title="wirksamer Modus" />
          <span class="mono small">{policy().mode}</span>
          <span class="muted small">
            {policy().detectors.length} Detektoren · {policy().retention_days} Tage Aufbewahrung
          </span>
        </Show>
        <span class="badge"><i />Egress: {mode()}</span>
      </div>
      <p class="muted small">{inlineNote(mode())}</p>
      <PrivacyDlpSection sandboxId={props.sandbox?.id} />
    </section>
  );
}
