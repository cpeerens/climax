// Shared update state, so the three surfaces that care agree with each other.
//
// The dashboard notice, the sidebar version line, and the Settings page all need
// the same two facts (what release exists, what this client is running) and one
// of them can change the third (dismissing). Fetching independently would mean
// three calls on every dashboard mount and a dismissed notice that leaves the
// sidebar still nagging until a reload.
//
// The comparison happens HERE, not in the backend. `update_check` deliberately
// reports only what the newest release is: in client mode the desktop app and
// the server it talks to are separately versioned, so only the client knows
// which version the answer should be measured against.

import { api, isNewerVersion, type UpdateInfo } from "$lib/api";
import { isTauri } from "$lib/transport";

let info = $state<UpdateInfo | null>(null);
let myVersion = $state<string | null>(null);
let loaded = $state(false);
let checking = $state(false);

/** The desktop app's own version. A LOCAL fact - `app_version_get` is native and
 *  answers from Tauri's package info with no server involved - so it is resolved
 *  before, and independently of, the cached read. Gating it behind that read
 *  would blank the version display exactly when a client-mode server is
 *  unreachable, which is when someone is most likely to be looking for it.
 *  No-op in a browser, where the version to be judged against IS the server's. */
async function loadOwnVersion(): Promise<void> {
  if (!isTauri() || myVersion !== null) return;
  try {
    myVersion = await api.appVersionGet();
  } catch (e) {
    console.error("read app version failed", e);
  }
}

/** Whichever version this client should be judged against: the desktop app's
 *  own, or - in a browser - the version of the server serving the page. */
async function resolveMyVersion(fetched: UpdateInfo): Promise<string> {
  if (!isTauri()) return fetched.backend_version;
  try {
    return await api.appVersionGet();
  } catch (e) {
    // A desktop app that cannot read its own version has nothing sensible to
    // compare, so fall back to the backend's rather than guessing.
    console.error("read app version failed", e);
    return fetched.backend_version;
  }
}

export const updateCheck = {
  /** Null until the first load resolves, or if it failed. */
  get info(): UpdateInfo | null {
    return info;
  },
  get latest() {
    return info?.latest ?? null;
  },
  /** The version this client is running. */
  get myVersion(): string | null {
    return myVersion;
  },
  get loaded(): boolean {
    return loaded;
  },
  get checking(): boolean {
    return checking;
  },
  /** A newer release exists than what this client is running. */
  get available(): boolean {
    const latest = info?.latest;
    return !!(latest && myVersion && isNewerVersion(latest.version, myVersion));
  },
  /** Available AND not already waved away. This is what the notice keys on, so
   *  dismissing one release stays dismissed until a newer one is published. */
  get unseen(): boolean {
    return this.available && info?.dismissed_version !== info?.latest?.version;
  },

  /** Read the cached answer. Cheap and network-free; safe on every mount. */
  async load(): Promise<void> {
    await loadOwnVersion();
    try {
      const fetched = await api.updateCheck();
      myVersion = await resolveMyVersion(fetched);
      info = fetched;
    } catch (e) {
      // Stay silent. An update notice is never important enough to turn into an
      // error message, and this call travels to the server in client mode.
      console.error("update check read failed", e);
    } finally {
      loaded = true;
    }
  },

  /** Ask GitHub now. Returns the error sentence to show, or null on success. */
  async checkNow(): Promise<string | null> {
    checking = true;
    await loadOwnVersion();
    try {
      const fetched = await api.updateCheckNow();
      myVersion = await resolveMyVersion(fetched);
      info = fetched;
      // The backend records its own designed sentence for a failed fetch; a
      // resolved promise with last_error set is a failed check, not an error.
      return fetched.last_error ?? null;
    } catch (e) {
      console.error("update check failed", e);
      const s = String(e).trim();
      const looksDesigned =
        /^[A-Z]/.test(s) && s.endsWith(".") && !s.includes(": ") && s.length <= 200;
      return looksDesigned ? s : "Couldn't check for updates. Try again in a moment.";
    } finally {
      checking = false;
    }
  },

  /** Hide the notice for the current release. */
  async dismiss(): Promise<void> {
    const version = info?.latest?.version;
    if (!version || !info) return;
    // Update locally first so the notice goes immediately; the write is a
    // preference, and a failed one only means it reappears next launch.
    info = { ...info, dismissed_version: version };
    try {
      await api.updateDismiss(version);
    } catch (e) {
      console.error("dismiss update notice failed", e);
    }
  },
};
