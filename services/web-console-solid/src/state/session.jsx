// Session and environment state.
//
// Small and explicit: one provider holding the identity, the CSRF token, and the
// active environment. Everything else is local to a feature.

import { createContext, createSignal, useContext } from 'solid-js';
import * as auth from '../api/auth.js';
import { setCsrfMissingHandler, setCsrfToken, setEnvironment } from '../api/client.js';
import { pickEnvironment, storageKey } from './environments.js';

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
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  // After a reload the security token is gone; go back to the sign-in form
  // with the reason instead of failing every action with a 403.
  setCsrfMissingHandler(() => {
    setIdentity(null);
    setCsrfToken(null);
    setError('Please sign in again: a page reload ends the security token that changes need.');
  });

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
      // lists it. Without such a choice the server's current data center opens (X15).
      const remembered = readRemembered(userId);
      const chosen = pickEnvironment(result.environments || [], remembered, result.environment_id)
        || result.environment_id;
      setRememberedEnvironment(remembered && chosen === remembered ? remembered : null);
      setActiveEnvironment(chosen);
      setEnvironment(chosen);
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
      const chosen = pickEnvironment(list, remembered, identityPayload.environment_id);
      if (chosen && chosen !== identityPayload.environment_id) {
        setActiveEnvironment(chosen);
        setEnvironment(chosen);
      }
      setRememberedEnvironment(remembered && chosen === remembered ? remembered : null);
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

  function switchEnvironment(id) {
    setActiveEnvironment(id);
    setEnvironment(id);
    // The choice is remembered per user, as an id: the next sign-in opens this one (X15).
    setRememberedEnvironment(id);
    writeRemembered(identity()?.user_id, id);
  }

  return (
    <SessionContext.Provider
      value={{
        identity,
        environments,
        activeEnvironment,
        rememberedEnvironment,
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
