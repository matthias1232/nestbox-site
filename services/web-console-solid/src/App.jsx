// The app: sign-in gate, the shell, and the route switch.
//
// The address bar is the single source of truth (routing.js, contract C1). This file maps what
// `parsePath` returns to one page component and wraps it in the shell, except for the pop-out
// windows, which own their whole window. Menu pages come from `features/shell/pages.js`.

import { Match, Show, Suspense, Switch, createMemo, createSignal, onMount, onCleanup, lazy } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { useSession } from './state/session.jsx';
import * as auth from './api/auth.js';
import LoginPage from './features/auth/LoginPage.jsx';
const AdminCenter = lazy(() => import('./features/sandboxes/AdminCenter.jsx'));
const WorkboxPage = lazy(() => import('./features/workbox/WorkboxPage.jsx'));
const MyBoxesPage = lazy(() => import('./features/workbox/MyBoxesPage.jsx'));
const TerminalPopout = lazy(() => import('./features/workbox/TerminalPopout.jsx'));
const DesktopPopout = lazy(() => import('./features/workbox/DesktopPopout.jsx'));
const HomePage = lazy(() => import('./features/home/HomePage.jsx'));
const DataCenterPage = lazy(() => import('./features/inventory/DataCenterPage.jsx'));
const SitePage = lazy(() => import('./features/inventory/SitePage.jsx'));
const NodePage = lazy(() => import('./features/inventory/NodePage.jsx'));
const ModelPage = lazy(() => import('./features/models/ModelPage.jsx'));
const ClusterDetailPage = lazy(() => import('./features/clusters/ClusterDetailPage.jsx'));
const DocsPage = lazy(() => import('./features/help/DocsPage.jsx'));
const LogsPopout = lazy(() => import('./features/logs/LogsPopout.jsx'));
const TrafficPopout = lazy(() => import('./features/traffic/TrafficPopout.jsx'));
import Shell from './features/shell/Shell.jsx';
import { PAGES } from './features/shell/pages.js';
import { SHELLLESS, navigate, parsePath, paths } from './routing.js';
import { adoptOpenerCsrf } from './api/client.js';

const readPlace = () => parsePath(window.location.pathname, window.location.search);

export default function App() {
  const session = useSession();
  const [checked, setChecked] = createSignal(false);
  const [place, setPlace] = createSignal(readPlace());

  const role = () => session.identity()?.role;
  const openAdmin = (id) => navigate(paths.sandbox(id));
  const closeAdmin = () => navigate(paths.menu('sandboxes'));
  // Named, not inline: a zero-argument arrow anywhere near a keyed `Show` is what the guard
  // test looks for, and it cannot tell a handler from a render function.
  const openNodes = () => navigate(paths.menu('nodes'));

  // The back button must move the screen, otherwise the address bar and the screen disagree.
  const onPop = () => setPlace(readPlace());
  window.addEventListener('popstate', onPop);
  onCleanup(() => window.removeEventListener('popstate', onPop));

  // A reload must not show the login form to someone who is still signed in. The session
  // cookie is HttpOnly, so the only way to know is to ask the server; a 401 here is the
  // normal "not signed in" answer, not a failure.
  onMount(async () => {
    try {
      const data = await auth.me();
      // The CSRF token lives in memory only; a reload loses it, so ask for a new one. A pop-out
      // window shares its opener's token instead: asking for a new one would invalidate the
      // token the main window holds.
      session.restore(data);
      if (!(SHELLLESS.has(place().kind) && adoptOpenerCsrf())) {
        await session.refreshCsrf();
      }
      // A member's day starts with their own sandboxes, not with the operator's overview.
      if (session.identity()?.role === 'member' && window.location.pathname === '/') {
        navigate(paths.my());
      }
    } catch {
      /* not signed in */
    } finally {
      setChecked(true);
    }
  });

  // The page for a place that lives inside the shell.
  //
  // `place()` is the address bar's signal, and a component body runs once: the page follows it
  // through a memo and a `Switch`, not by reading it once, or a menu click never changed the
  // screen (R3). A place with an id is remounted when the id changes (keyed `Show`), so
  // `/node/a` → `/node/b` loads b instead of showing a's data.
  function Page() {
    const current = createMemo(() => place());
    return (
      <Switch fallback={<HomePage />}>
        <Match when={current().kind === 'dc'}>
          <Show when={current().id} keyed>
            {(id) => <DataCenterPage id={id} tab={current().tab} />}
          </Show>
        </Match>
        <Match when={current().kind === 'site'}>
          <Show when={current().id} keyed>
            {(id) => <SitePage id={id} tab={current().tab} />}
          </Show>
        </Match>
        <Match when={current().kind === 'node'}>
          <Show when={current().id} keyed>
            {(id) => <NodePage id={id} tab={current().tab} />}
          </Show>
        </Match>
        <Match when={current().kind === 'admin'}>
          <Show when={current().id} keyed>
            {(id) => <AdminCenter sandboxId={id} initialTab={current().tab || undefined} onClose={closeAdmin} onFollow={openAdmin} />}
          </Show>
        </Match>
        <Match when={current().kind === 'box'}>
          <Show when={current().id} keyed>
            {(id) => <WorkboxPage sandboxId={id} />}
          </Show>
        </Match>
        <Match when={current().kind === 'my'}><MyBoxesPage /></Match>
        <Match when={current().kind === 'model'}>
          <Show when={current().id} keyed>
            {(id) => <ModelPage id={id} />}
          </Show>
        </Match>
        <Match when={current().kind === 'cluster'}>
          <Show when={current().id} keyed>
            {(id) => <ClusterDetailPage id={id} />}
          </Show>
        </Match>
        <Match when={current().kind === 'docs'}><DocsPage slug={current().slug} /></Match>
        <Match when={current().kind === 'menu' && PAGES[current().id]}>
          <Dynamic component={PAGES[current().id]} role={role()} onOpenAdmin={openAdmin} onOpenSandbox={openAdmin} onOpenNode={openNodes} />
        </Match>
      </Switch>
    );
  }

  return (
    <Show when={checked()} fallback={<div class="login"><p class="muted">Loading…</p></div>}>
      <Show when={session.identity()} fallback={<LoginPage />}>
        {/* Pop-out windows own their window: no sidebar, no menu. */}
        <Show when={place().kind === 'terminal'}><TerminalPopout sandboxId={place().id} tab={place().tab} /></Show>
        <Show when={place().kind === 'desktop'}><DesktopPopout sandboxId={place().id} /></Show>
        <Show when={place().kind === 'poplogs'}><LogsPopout id={place().id} /></Show>
        <Show when={place().kind === 'poptraffic'}><TrafficPopout /></Show>
        <Show when={!SHELLLESS.has(place().kind)}>
          <Shell place={place()}>
            {/* Keyed: choosing another data center remounts the page, so every screen reloads its data. */}
            {/* The child function takes the key: a zero-argument child is called inside a
                tracked effect, and creating Suspense plus a lazy page there reads and writes
                signals on every pass, which re-created the page forever (2026-11-03). */}
            <Show when={session.activeEnvironment()} keyed>
              {(environment) => (
                <Suspense fallback={<p class="muted loading">Loading…</p>}>
                  <Page environment={environment} />
                </Suspense>
              )}
            </Show>
          </Shell>
        </Show>
      </Show>
    </Show>
  );
}
