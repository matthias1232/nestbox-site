// Session and environment state.
//
// Small and explicit: one provider holding the identity, the CSRF token, and the
// active environment. Everything else is local to a feature.

import { createContext, createSignal, useContext } from 'solid-js';
import * as auth from '../api/auth.js';
import * as tenancy from '../api/tenancy.js';
import { setCsrfMissingHandler, setCsrfToken, setEnvironment } from '../api/client.js';
import { environmentsWithNodes, firstUsable, pickEnvironment, storageKey } from './environments.js';

const SessionContext = createContext();

// The remembered data center of one user, or null. The value is an id; a token never
// goes into storage.
function readRemembered(userId) {
  const key = storageKey(userId);
  if (!key) return null;
  try {
    return typeof localStorage === 'undefined' ? null : localStorage.getItem(key);
  } catch {
    return null;
  }
}

// Remember a choice, or forget it. A browser that refuses storage still works, it only
// forgets where the user was.
function writeRemembered(userId, environmentId) {
  const key = storageKey(userId);
  if (!key) return;
  try {
    if (environmentId) localStorage.setItem(key, environmentId);
    else localStorage.removeItem(key);
  } catch {
    /* storage is optional; the console works without it */
  }
}

export function SessionProvider(props) {
  const [identity, setIdentity] = createSignal(null);
  const [environments, setEnvironments] = createSignal([]);
  const [activeEnvironment, setActiveEnvironment] = createSignal(null);
  // The data center this browser remembers for the user, so the empty-data-center notice
  // can stay quiet when the user already chose one.
  const [rememberedEnvironment, setRememberedEnvironment] = createSignal(null);
  // The data center the console opened by itself, and the empty one it left, so the sidebar can
  // say so once. Null means it did not switch and has nothing to explain (U02).
  const [autoSwitched, setAutoSwitched] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  // After a reload the security token is gone; go back to the sign-in form
  // with the reason instead of failing every action with a 403.
  setCsrfMissingHandler(() => {
    setIdentity(null);
    setCsrfToken(null);
    setError('Please sign in again: a page reload ends the security token that changes need.');
  });

  // The data centers of this session with their node counts.
  //
  // `/environments` says which ones the user may open but not whether anything is in them, so
  // the counts come from the organization list. Only data centers the session may open are
  // kept, and a failed count is an empty list: a first sign-in must never fail because the
  // count could not be read, it only stays where the server put it (U02).
  async function countedEnvironments(list) {
    try {
      const answer = await tenancy.organizations();
      const known = new Set(list.map((entry) => entry?.id));
      return environmentsWithNodes(answer?.items || []).filter((entry) => known.has(entry.id));
    } catch {
      return [];
    }
  }

  // Open the data center this session starts on, and explain a switch the console made alone.
  //
  // The remembered choice always wins, and the server's current one is next (X15). Only when
  // the browser remembers nothing does the node count matter: an empty current data center
  // gives way to one that has nodes, is switched without remembering (the user did not pick
  // it), and is reported once so the move is not silent (U02).
  async function openEnvironment(list, current, remembered) {
    const chosen = pickEnvironment(list, remembered, current) || current;
    setRememberedEnvironment(remembered && chosen === remembered ? remembered : null);
    setActiveEnvironment(chosen);
    setEnvironment(chosen);
    if (remembered) {
      setAutoSwitched(null);
      return;
    }
    const counted = await countedEnvironments(list);
    const target = firstUsable(counted, chosen);
    if (!target || target === chosen) {
      setAutoSwitched(null);
      return;
    }
    const nameOf = (id) => counted.find((entry) => entry.id === id)?.name;
    const name = nameOf(target) || 'the other data center';
    const currentName = nameOf(chosen)
      || (list.find((entry) => entry?.id === chosen)?.name)
      || 'This data center';
    switchEnvironment(target, { remember: false });
    setAutoSwitched({ id: target, name, currentName });
  }

  async function signIn(email, password) {
    setBusy(true);
    setError(null);
    try {
      const result = await auth.login(email, password);
      // The CSRF token lives in memory only. Putting it in storage would make it
      // readable by any script on the page, which is the thing it protects.
      setCsrfToken(result.csrf_token);
      // The login answer names no user id, and the remembered data center is per user, so
      // ask once who this is. Without an id the console still works, it only forgets the
      // choice.
      let userId = result.user_id ?? null;
      if (!userId) {
        try {
          userId = (await auth.me())?.user_id ?? null;
        } catch {
          /* the session works; only the remembered data center is lost */
        }
      }
      setIdentity({
        display_name: result.display_name,
        role: result.role,
        environment_id: result.environment_id,
        user_id: userId,
      });
      setEnvironments(result.environments || []);
      // A returning user opens the data center they chose last, as long as the server still
      // lists it. Without such a choice the server's current data center opens (X15), unless
      // it has no nodes and another one has (U02).
      await openEnvironment(result.environments || [], result.environment_id, readRemembered(userId));
      return true;
    } catch (failure) {
      setError(failure.message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function signOut() {
    try {
      await auth.logout();
    } catch {
      // A failed logout still clears the local state; the cookie is cleared by
      // the response when it succeeds, and the session expires regardless.
    }
    setIdentity(null);
    setCsrfToken(null);
    setEnvironment(null);
    setActiveEnvironment(null);
    setAutoSwitched(null);
  }

  // Restore the identity after a page reload, without a CSRF token: the token
  // is deliberately memory-only, so a reload requires a fresh sign-in before any
  // mutating action. Read-only views keep working.
  //
  // `/auth/me` names the session's data center but not the ones the user reaches, so the
  // list is fetched too: the remembered choice opens once it is known to be reachable
  // (X15), and the server's data center stands until that answer arrives.
  async function restore(identityPayload) {
    setIdentity({
      display_name: identityPayload.display_name,
      role: identityPayload.role,
      environment_id: identityPayload.environment_id,
      // The owner of a sandbox is matched by id; a reload must not lose it.
      user_id: identityPayload.user_id ?? null,
    });
    const remembered = readRemembered(identityPayload.user_id);
    setActiveEnvironment(identityPayload.environment_id);
    setEnvironment(identityPayload.environment_id);
    try {
      const answer = await auth.environments();
      const list = answer?.items || [];
      setEnvironments(list);
      // The remembered choice opens once it is known to be reachable, and without one an empty
      // data center gives way to one with nodes, exactly as on sign-in (X15, U02).
      await openEnvironment(list, identityPayload.environment_id, remembered);
    } catch {
      /* the server's data center stands, and the empty notice reads its inventory */
    }
  }

  // After a reload: a fresh token for the live session. If the session itself is gone the
  // 401 sends the user to the sign-in form, which is the honest answer.
  async function refreshCsrf() {
    try {
      const result = await auth.csrf();
      setCsrfToken(result.csrf_token);
    } catch {
      /* read-only views still work; the first change asks for a sign-in */
    }
  }

  // Switch the data center. A link or a pop-out window adopts its own without
  // overwriting the user's remembered choice (`remember: false`, U01).
  function switchEnvironment(id, { remember = true } = {}) {
    setActiveEnvironment(id);
    setEnvironment(id);
    if (!remember) return;
    // The choice is remembered per user, as an id: the next sign-in opens this one (X15).
    setRememberedEnvironment(id);
    writeRemembered(identity()?.user_id, id);
    // The user picked a data center, so the notice about the one the console opened is done.
    setAutoSwitched(null);
  }

  return (
    <SessionContext.Provider
      value={{
        identity,
        environments,
        activeEnvironment,
        rememberedEnvironment,
        autoSwitched,
        error,
        busy,
        signIn,
        signOut,
        restore,
        refreshCsrf,
        switchEnvironment,
      }}
    >
      {props.children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const context = useContext(SessionContext);
  if (!context) throw new Error('useSession must be used inside SessionProvider');
  return context;
}
