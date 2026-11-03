// Login.
//
// The password is read from the form at submit and never stored in a signal, so
// it does not sit in the component tree after the request.
//
// Identity providers (ADR 0046) are plain links: the browser leaves for the provider and comes
// back to the API, which sets the session cookie and redirects here. An invitation link
// (`/?invite=<token>`) is carried through the same trip, so the person lands in the right place.

import { For, Show, createSignal, onMount } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import * as api from '../../api/tenancy.js';
import '../../styles/login.css';

// Show/hide glyph for the password toggle (private: the shared eye icons are not built yet).
function EyeGlyph(props) {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
      <path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8Z" />
      <circle cx="8" cy="8" r="2" />
      {props.off && <path d="M2.5 13.5 13.5 2.5" />}
    </svg>
  );
}

export default function LoginPage() {
  const session = useSession();
  const params = new URLSearchParams(window.location.search);
  const invite = params.get('invite');
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [options, setOptions] = createSignal({ providers: [], registration_open: false });
  const [preview, setPreview] = createSignal(null);
  const [reveal, setReveal] = createSignal(false);
  const [imageFailed, setImageFailed] = createSignal(false);
  const [videoFailed, setVideoFailed] = createSignal(false);
  const calm = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const [problem, setProblem] = createSignal(params.get('login_error'));

  onMount(async () => {
    try { setOptions(await api.signInOptions()); } catch { /* password sign-in still works */ }
    if (invite) {
      try { setPreview(await api.invitationPreview(invite)); }
      catch { setProblem('This invitation link is not valid any more. Ask for a new one.'); }
    }
  });

  async function submit(event) {
    event.preventDefault();
    await session.signIn(email().trim(), password());
    setPassword('');
  }

  const providerLink = (id) =>
    `/api/v1/auth/oidc/${id}/start${invite ? `?invitation=${encodeURIComponent(invite)}` : ''}`;

  return (
    <div class="lp">
      <main class="lp-form-side">
        <form class="lp-form" onSubmit={submit}>
          <span class="brand-mark lp-brand">nestbox<i>_</i></span>
          <h1>Your private AI operations center.</h1>
          <p class="muted">Open source · governed sandboxes · local models</p>

          <Show when={preview()}>
            <p class="notice">
              You are invited to <strong>{preview().data_center}</strong> of {preview().organization} as{' '}
              <strong>{preview().role}</strong>
              {preview().sandbox ? <> for the sandbox <strong>{preview().sandbox}</strong></> : null}.
              {preview().email ? <> Use {preview().email}.</> : null}
            </p>
          </Show>
          <Show when={problem()}><p class="error">{problem()}</p></Show>

          <Show when={options().providers.length > 0}>
            <div class="providers">
              <For each={options().providers}>
                {(provider) => (
                  <a class="button secondary" href={providerLink(provider.id)}>Continue with {provider.label}</a>
                )}
              </For>
            </div>
            <p class="muted small">
              {options().registration_open && !invite
                ? 'New here? Continue with a provider to create your workspace.'
                : invite ? '' : 'This installation is by invitation.'}
            </p>
            <hr />
          </Show>

          <Show when={!invite || options().providers.length === 0}>
            <label>
              <span>Email</span>
              <input value={email()} onInput={(e) => setEmail(e.currentTarget.value)}
                     autocomplete="username" required />
            </label>

            <label>
              <span>Password</span>
              <span class="lp-password">
                <input type={reveal() ? 'text' : 'password'} value={password()}
                       onInput={(e) => setPassword(e.currentTarget.value)}
                       autocomplete="current-password" required />
                <button type="button" class="ghost tiny" onClick={() => setReveal(!reveal())}
                        aria-label={reveal() ? 'Hide password' : 'Show password'}
                        aria-pressed={reveal()}>
                  <EyeGlyph off={reveal()} />
                </button>
              </span>
            </label>

            {session.error() && <p class="error">{session.error()}</p>}

            <button type="submit" class="primary" disabled={session.busy()}>
              {session.busy() ? 'Signing in…' : 'Sign in'}
            </button>
            <p class="muted small">Forgot your password? Ask your administrator.</p>
          </Show>

          <p class="lp-foot small">
            <span class="muted">Built in public · </span>
            <a href="https://nestbox.mmbits.com/" rel="noopener">nestbox.mmbits.com</a>
          </p>
        </form>
      </main>

      <aside class="lp-visual" aria-hidden="true">
        <Show when={!imageFailed()} fallback={<div class="lp-fallback" />}>
          <img src="/illustrations/login.webp" alt="" onError={() => setImageFailed(true)} />
        </Show>
        <Show when={!calm && !videoFailed()}>
          <video autoplay muted loop playsinline poster="/illustrations/login.webp"
                 onError={() => setVideoFailed(true)}>
            <source src="/illustrations/login-loop.mp4" type="video/mp4" onError={() => setVideoFailed(true)} />
          </video>
        </Show>
        <span class="lp-tag lp-tag-a">GATEWAY ACTIVE</span>
        <span class="lp-tag lp-tag-b">N ISOLATED WORKLOADS</span>
      </aside>
    </div>
  );
}
