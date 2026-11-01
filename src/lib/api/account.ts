/**
 * Account API
 *
 * Sign-in to a youskill account with GitHub or Apple. The tokens stay in the backend and the
 * OS keychain; the frontend gets only the identity.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { apiCall } from "./index";

export type AccountProvider = "github" | "apple";

export interface AccountIdentity {
  accountId: string;
  provider: AccountProvider;
  displayName: string;
}

export interface AccountStatus {
  /** `null` when signed out. */
  identity: AccountIdentity | null;
}

export interface AccountChanged {
  status: AccountStatus;
  /** A sign-in failure to show. A cancelled sign-in has none. */
  error: string | null;
}

export function getAccountStatus(): Promise<AccountStatus> {
  return apiCall("account_status");
}

/** Starts a sign-in and returns the URL to open in the system browser. */
export function startAccountSignIn(provider: AccountProvider): Promise<string> {
  return apiCall("account_sign_in", { provider });
}

/** Revokes the session, deletes the tokens and clears the favorites of this device. */
export function signOutAccount(): Promise<AccountStatus> {
  return apiCall("account_sign_out");
}

/** The result of a sign-in callback, or a session that the server revoked. */
export function onAccountChanged(handler: (event: AccountChanged) => void): Promise<UnlistenFn> {
  return listen<AccountChanged>("account://changed", (event) => handler(event.payload));
}
