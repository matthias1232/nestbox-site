// The app: sign-in gate, the shell, and the route switch.
//
// The address bar is the single source of truth (routing.js, contract C1). This file maps what
// `parsePath` returns to one page component and wraps it in the shell, except for the pop-out
// windows, which own their whole window. Menu pages come from `features/shell/pages.js`.

import { Match, Show, Suspense, Switch, createEffect, createMemo, createSignal, onMount, onCleanup, lazy } from 'solid-js';
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
import { useInventory } from './features/shell/useInventory.js';
import { placeEnvironmentInfo } from './features/shell/placeEnvironment.js';
import { PAGES } from './features/shell/pages.js';
import { SHELLLESS, navigate, parsePath, paths } from './routing.js';
import { adoptOpenerCsrf, adoptOpenerEnvironment } from './api/client.js';
import { environmentFromUrl } from './api/environmentUrl.js';

const readPlace = () => parsePath(window.location.pathname, window.location.search);

// The last data center a link moved the session to, as `kind:id:data center`. Module scope on
// purpose: a switch remounts the page, and a guard inside the component would follow the same
// place again on every mount, which turns the data-center picker into a fight (V15).
let followed = null;

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
      // A pop-out window carries no data center of its own: it takes the opener's, or the
      // one the link named. It never overwrites the user's remembered choice (U01).
      const fromOpener = SHELLLESS.has(place().kind) ? adoptOpenerEnvironment() : null;
      const wanted = environmentFromUrl(window.location.search) || fromOpener;
      if (wanted && wanted !== session.activeEnvironment()) {
        session.switchEnvironment(wanted, { remember: false });
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

    // A link can name an entity of another data center (V15). The session's data center scopes
    // every read, so `/box/<id>` of `default` answered "The requested resource does not exist"
    // while the session sat in `Berlin`. The inventory tree is the only client-side map from an
    // id to its data center, so wait for it and move the session once before the page renders.
    const inventory = useInventory();
    const elsewhere = createMemo(() => {
      const p = current();
      // Only a place that names a tree entity can live in another data center.
      if (!p.id) return null;
      const root = inventory.root();
      if (!root) return null;
      const target = placeEnvironmentInfo(root, p);
      if (!target || target.environmentId === session.activeEnvironment()) return null;
      return target;
    });
    // No tree yet: the usual loading state, not a bare "does not exist" from the API. A tree
    // that failed to load is not "missing" either; the page itself then says what is wrong.
    const waiting = createMemo(() => Boolean(current().id) && !inventory.root() && !inventory.error());

    // The link's target once its automatic switch has already been followed (F5). `followed` is
    // a module variable and therefore not reactive, so the notice keeps its own answer; without
    // that the screen would sit on "Loading…" forever after the data-center picker moved the
    // session back.
    const [stranded, setStranded] = createSignal(null);

    createEffect(() => {
      const target = elsewhere();
      if (!target) {
        setStranded(null);
        return;
      }
      // One follow per address and target: switching remounts this page, so a guard that lives
      // only in the component would follow the same link again and again.
      const key = `${current().kind}:${current().id}:${target.environmentId}`;
      if (followed === key) {
        // Following the link again would fight the picker that moved the session, so offer the
        // choice instead of an automatic switch that never happens.
        setStranded(target);
        return;
      }
      setStranded(null);
      followed = key;
      session.switchEnvironment(target.environmentId);
    });

    // The notice's buttons: switch to the data center the link names, or go home and leave the
    // session where it is.
    const openTarget = () => {
      const target = stranded();
      if (target) session.switchEnvironment(target.environmentId);
    };
    const goHome = () => navigate('/');

    return (
      <Switch fallback={<HomePage />}>
        {/* A link whose automatic switch was already followed names its data center and offers
            the move, instead of a loading line that never ends (F5). It comes first, so it wins
            over the loading match below. */}
        <Match when={stranded()}>
          <p class="notice">
            This link points into data center {stranded().environmentName}.{' '}
            <button class="ghost tiny" onClick={openTarget}>
              Open in {stranded().environmentName}
            </button>{' '}
            <button class="ghost tiny" onClick={goHome}>
              Home
            </button>
          </p>
        </Match>
        <Match when={waiting() || elsewhere()}>
          <p class="muted loading">Loading…</p>
        </Match>
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
