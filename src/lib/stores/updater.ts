import { writable, get } from "svelte/store";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type UpdaterState = {
  checked: boolean;
  checking: boolean;
  installing: boolean;
  hasUpdate: boolean;
  latestVersion: string;
  update: Update | null;
  /** Markdown notes of the latest version. */
  releaseNotes: string;
  error: string;
  /** Bytes received by the running install. */
  downloaded: number;
  /** Size of the update package; 0 when the server does not send it. */
  contentLength: number;
};

const initialState: UpdaterState = {
  checked: false,
  checking: false,
  installing: false,
  hasUpdate: false,
  latestVersion: "",
  update: null,
  releaseNotes: "",
  error: "",
  downloaded: 0,
  contentLength: 0,
};

/** Minimum time between two app update checks; see docs/decisions/0002-app-update-check-policy.md. */
export const APP_UPDATE_CHECK_INTERVAL_MS = 60 * 60 * 1000;

/** Releases are tagged `v<version>`; the release body holds the changelog of that version. */
const RELEASE_BY_TAG_URL = "https://api.github.com/repos/wanggang316/you-skill/releases/tags/v";

type Unlisten = () => void;

type AutomaticCheckTriggers = {
  /** Registers a window focus listener and resolves to its remover. */
  onFocus: (listener: () => void) => Promise<Unlisten>;
  setInterval?: (callback: () => void, ms: number) => unknown;
  clearInterval?: (id: unknown) => void;
};

const getErrorMessage = (error: unknown, fallback: string): string => {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  try {
    const text = JSON.stringify(error);
    return text && text !== "{}" ? text : fallback;
  } catch {
    return fallback;
  }
};

export function createAppUpdater(deps = { check, relaunch, now: () => Date.now() }) {
  const updaterState = writable<UpdaterState>({ ...initialState });
  /** Visibility of the update dialog, shared by every entry point. */
  const updateDialogOpen = writable(false);

  let checkPromise: Promise<void> | null = null;
  /** End time of the last check of any kind; in memory only, so each app start checks again. */
  let lastCheckAt: number | null = null;

  async function ensureUpdateChecked(force = false): Promise<void> {
    if (checkPromise) {
      await checkPromise;
      return;
    }

    const current = get(updaterState);
    if (!force && (current.checked || current.checking)) {
      return;
    }

    const task = (async () => {
      updaterState.update((state) => ({ ...state, checking: true, error: "" }));
      try {
        const update = await deps.check();
        updaterState.update((state) => ({
          ...state,
          checked: true,
          checking: false,
          hasUpdate: Boolean(update),
          latestVersion: update?.version ?? "",
          update: update ?? null,
          releaseNotes: update?.body?.trim() ?? "",
        }));
        if (update && !update.body?.trim()) {
          void loadReleaseNotes(update.version);
        }
      } catch (error) {
        updaterState.update((state) => ({
          ...state,
          checked: true,
          checking: false,
          error: getErrorMessage(error, "Failed to check for update"),
        }));
        throw error;
      } finally {
        lastCheckAt = deps.now();
      }
    })();

    checkPromise = task;
    try {
      await task;
    } finally {
      checkPromise = null;
    }
  }

  /** The update manifest can come without notes; use the GitHub release body instead. */
  async function loadReleaseNotes(version: string): Promise<void> {
    try {
      const response = await fetch(`${RELEASE_BY_TAG_URL}${version}`, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!response.ok) return;
      const release: { body?: string | null } = await response.json();
      const notes = release.body?.trim() ?? "";
      updaterState.update((state) =>
        state.latestVersion === version ? { ...state, releaseNotes: notes } : state
      );
    } catch (error) {
      console.error("Failed to load release notes:", error);
    }
  }

  async function installAvailableUpdate(): Promise<void> {
    const current = get(updaterState);
    if (current.installing) return;

    if (!current.update) {
      await ensureUpdateChecked(true);
    }

    const refreshed = get(updaterState);
    if (!refreshed.update) return;

    updaterState.update((state) => ({
      ...state,
      installing: true,
      error: "",
      downloaded: 0,
      contentLength: 0,
    }));
    try {
      await refreshed.update.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === "Started") {
          const contentLength = event.data.contentLength ?? 0;
          updaterState.update((state) => ({ ...state, contentLength }));
        } else if (event.event === "Progress") {
          const { chunkLength } = event.data;
          updaterState.update((state) => ({
            ...state,
            downloaded: state.downloaded + chunkLength,
          }));
        }
      });
      await deps.relaunch();
    } catch (error) {
      updaterState.update((state) => ({
        ...state,
        installing: false,
        error: getErrorMessage(error, "Failed to install update"),
      }));
      throw error;
    }
  }

  /** Opens the update dialog and starts a fresh check, like Sparkle's "Check for Updates…". */
  function openUpdateDialog(): void {
    updateDialogOpen.set(true);
    if (get(updaterState).installing) return;
    ensureUpdateChecked(true).catch(console.error);
  }

  /**
   * Silent automatic check: runs only when the interval has passed since the last check.
   * A check changes the dialog to its checking view, so skip while the dialog is open.
   */
  async function checkForUpdateIfDue(): Promise<void> {
    const state = get(updaterState);
    if (state.checking || state.installing || get(updateDialogOpen)) return;
    if (lastCheckAt !== null && deps.now() - lastCheckAt < APP_UPDATE_CHECK_INTERVAL_MS) return;
    try {
      await ensureUpdateChecked(true);
    } catch (error) {
      console.error("Automatic app update check failed:", error);
    }
  }

  /** Checks on window focus and every hour; returns a function that removes both triggers. */
  function startAutomaticUpdateChecks({
    onFocus,
    setInterval: startTimer = (callback, ms) => globalThis.setInterval(callback, ms),
    clearInterval: stopTimer = (id) =>
      globalThis.clearInterval(id as ReturnType<typeof globalThis.setInterval>),
  }: AutomaticCheckTriggers): Unlisten {
    const runCheck = () => void checkForUpdateIfDue();
    const timer = startTimer(runCheck, APP_UPDATE_CHECK_INTERVAL_MS);
    let stopped = false;
    let unlistenFocus: Unlisten | null = null;
    onFocus(runCheck)
      .then((unlisten) => {
        // The layout can be destroyed before the listener is registered.
        if (stopped) unlisten();
        else unlistenFocus = unlisten;
      })
      .catch(console.error);

    return () => {
      stopped = true;
      stopTimer(timer);
      unlistenFocus?.();
      unlistenFocus = null;
    };
  }

  return {
    updaterState,
    updateDialogOpen,
    ensureUpdateChecked,
    installAvailableUpdate,
    openUpdateDialog,
    checkForUpdateIfDue,
    startAutomaticUpdateChecks,
  };
}

const appUpdater = createAppUpdater();

export const {
  updaterState,
  updateDialogOpen,
  ensureUpdateChecked,
  installAvailableUpdate,
  openUpdateDialog,
  startAutomaticUpdateChecks,
} = appUpdater;
