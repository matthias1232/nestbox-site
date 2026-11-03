// A dialog that creates an invitation link; the link is shown once in a copy box.
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import * as api from '../../api/tenancy.js';
import { rolesFor, roleSentence, inviteLink } from './permissions.js';

// Props: scopeType, scopeId (sandbox id for a sandbox scope), onClose(), onDone().
export default function InviteDialog(props) {
  const isSandbox = () => props.scopeType === 'sandbox';
  const [role, setRole] = createSignal(isSandbox() ? 'member' : 'operator');
  const [email, setEmail] = createSignal('');
  const [days, setDays] = createSignal(7);
  const [link, setLink] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  let root;

  onMount(() => {
    const onKey = (event) => {
      if (event.key === 'Escape') props.onClose();
      if (event.key === 'Tab' && root) {
        const items = [...root.querySelectorAll('button, input, select')].filter((el) => !el.disabled);
        if (items.length === 0) return;
        const first = items[0];
        const last = items[items.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      }
    };
    document.addEventListener('keydown', onKey);
    onCleanup(() => document.removeEventListener('keydown', onKey));
    root?.querySelector('select, input')?.focus();
  });

  async function submit(event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const body = { role: isSandbox() ? 'member' : role(), valid_days: Number(days()) || 7 };
      if (isSandbox()) body.sandbox_id = props.scopeId;
      if (email().trim()) body.email = email().trim();
      const made = await api.invite(body);
      setLink(inviteLink(window.location.origin, made.token));
      props.onDone && props.onDone();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="users-dialog-backdrop" onClick={props.onClose}>
      <div class="users-dialog" role="dialog" aria-modal="true" aria-label="Invite by link" ref={root} onClick={(e) => e.stopPropagation()}>
        <h2>Invite by link</h2>
        <Show when={error()}><p class="error">{error()}</p></Show>
        <Show when={!link()} fallback={
          <div>
            <p class="notice">Send this link once. It is not shown again.</p>
            <input class="mono users-link" readonly value={link()} aria-label="Invitation link" onFocus={(e) => e.currentTarget.select()} />
            <div class="actions">
              <button type="button" class="secondary" onClick={() => navigator.clipboard?.writeText(link())}>Copy link</button>
              <button type="button" class="primary" onClick={props.onClose}>Done</button>
            </div>
          </div>
        }>
          <form onSubmit={submit}>
            <label class="field"><span>Role</span>
              <select value={role()} disabled={isSandbox()} onChange={(e) => setRole(e.currentTarget.value)}>
                <For each={isSandbox() ? ['member'] : rolesFor('environment')}>{(r) => <option value={r}>{r}</option>}</For>
              </select>
              <small class="muted">{roleSentence(isSandbox() ? 'member' : role())}</small>
            </label>
            <label class="field"><span>Email (optional)</span>
              <input type="email" value={email()} placeholder="only this address may use the link"
                     onInput={(e) => setEmail(e.currentTarget.value)} />
            </label>
            <label class="field"><span>Valid for (days)</span>
              <input type="number" min="1" max="30" value={days()} onInput={(e) => setDays(e.currentTarget.value)} />
            </label>
            <div class="actions">
              <button type="submit" class="primary" disabled={busy()}>Create invitation link</button>
              <button type="button" class="secondary" onClick={props.onClose}>Cancel</button>
            </div>
          </form>
        </Show>
      </div>
    </div>
  );
}
