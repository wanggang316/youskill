import { writable } from "svelte/store";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import {
  deleteAccount,
  getAccountStatus,
  onAccountChanged,
  signOutAccount,
  startAccountSignIn,
  type AccountChanged,
  type AccountIdentity,
  type AccountProvider,
} from "../api/account";
import { favorites } from "./favorites";

interface AccountState {
  /** `null` when signed out. */
  identity: AccountIdentity | null;
  loaded: boolean;
  /** The provider whose browser sign-in waits for its callback. */
  signingIn: AccountProvider | null;
  signingOut: boolean;
  deleting: boolean;
  error: string;
}

const message = (error: unknown) =>
  error instanceof Error && error.message ? error.message : String(error);

export function createAccount(
  api = {
    status: getAccountStatus,
    start: startAccountSignIn,
    signOut: signOutAccount,
    deleteAccount,
    openUrl: (url: string) => openExternal(url),
    listen: onAccountChanged,
  },
  onSignedOut: () => void = () => {}
) {
  const state = writable<AccountState>({
    identity: null,
    loaded: false,
    signingIn: null,
    signingOut: false,
    deleting: false,
    error: "",
  });
  const patch = (value: Partial<AccountState>) =>
    state.update((current) => ({ ...current, ...value }));
  let listening: Promise<unknown> | null = null;

  function changed({ status, error }: AccountChanged) {
    patch({ identity: status.identity, loaded: true, signingIn: null, error: error ?? "" });
  }

  return {
    state: { subscribe: state.subscribe },
    /** Reads the status and, once, starts to follow sign-in callbacks. */
    async load() {
      listening ??= api.listen(changed).catch((error) => {
        listening = null;
        console.error("Failed to listen for account changes:", error);
      });
      await listening;
      try {
        const { status, error } = await api.status();
        patch({ identity: status.identity, loaded: true });
        if (error) patch({ signingIn: null, error });
      } catch (error) {
        patch({ loaded: true, error: message(error) });
      }
    },
    /** Opens the sign-in in the system browser. A new sign-in replaces a waiting one. */
    async signIn(provider: AccountProvider) {
      patch({ error: "", signingIn: provider });
      try {
        await api.openUrl(await api.start(provider));
      } catch (error) {
        patch({ signingIn: null, error: message(error) });
      }
    },
    async signOut() {
      patch({ error: "", signingOut: true });
      try {
        const status = await api.signOut();
        patch({ identity: status.identity, signingIn: null });
        onSignedOut();
      } catch (error) {
        patch({ error: message(error) });
      } finally {
        patch({ signingOut: false });
      }
    },
    /**
     * Deletes the account. Returns `true` when it is deleted. On a failure the account stays
     * and the error shows; an ended session shows signed out.
     */
    async deleteAccount() {
      patch({ error: "", deleting: true });
      try {
        const { status, error } = await api.deleteAccount();
        patch({ identity: status.identity, error: error ?? "" });
        if (error) return false;
        onSignedOut();
        return true;
      } catch (error) {
        patch({ error: message(error) });
        return false;
      } finally {
        patch({ deleting: false });
      }
    },
  };
}

/** Sign-out and account deletion clear the favorites of this device, so the list reloads. */
export const account = createAccount(undefined, () => void favorites.load());
export const accountState = account.state;
