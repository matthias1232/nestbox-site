// One badge per content-check state, in the same colours everywhere.
import '../styles/dlp.css';

const LABELS = {
  blocked: 'Blocked',
  replaced: 'Replaced',
  redacted: 'Replaced',
  pseudonymized: 'Pseudonymized',
  allowed: 'Recorded',
  unavailable: 'Not checked',
  too_large: 'Too large',
  exempt: 'Exempt',
  skipped: 'Exempt',
  clean: 'Clean',
};

const EXPLANATIONS = {
  blocked: 'The request never reached the model.',
  replaced: 'A value was replaced before sending.',
  redacted: 'A value was masked before sending.',
  pseudonymized: 'The model saw a placeholder; the agent gets the value back.',
  allowed: 'Recorded only: the request went out unchanged.',
  unavailable: 'The check could not answer; the request was refused.',
  too_large: 'The request was larger than the check reads; it was refused.',
  exempt: 'An exception allows this value to this destination.',
  skipped: 'The check is off for this destination.',
  clean: 'No pattern matched.',
};

/** A coloured label for a content-check state, with the reason as its tooltip. */
export default function DlpBadge(props) {
  const state = () => String(props.state || 'clean');
  return (
    <span
      class="dlp-badge"
      data-state={state()}
      title={props.title || EXPLANATIONS[state()] || 'Content-check state'}
    >
      {LABELS[state()] || state()}
    </span>
  );
}
