/**
 * Favorites API
 *
 * Local favorites of marketplace skills. They need no account; while signed in, they sync
 * with the account in the backend.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { apiCall } from "./index";

/** Saved name, source and description, shown when the skill leaves the marketplace. */
export interface FavoriteSnapshot {
  name: string;
  source: string;
  description?: string | null;
}

export interface LocalFavorite {
  /** Marketplace skill id. */
  skillId: string;
  favorited: boolean;
  /** Milliseconds since the Unix epoch. */
  updatedAt: number;
  snapshot: FavoriteSnapshot;
  /** The change is not sent to an account yet. */
  pending: boolean;
}

export function listFavorites(): Promise<LocalFavorite[]> {
  return apiCall("list_favorites");
}

/** Adds or removes a favorite and returns the updated list. An add needs the snapshot. */
export function setFavorite(
  skillId: string,
  favorited: boolean,
  snapshot?: FavoriteSnapshot
): Promise<LocalFavorite[]> {
  return apiCall("set_favorite", { skillId, favorited, snapshot: snapshot ?? null });
}

/** Result of a sync with the account. */
export interface FavoritesSynced {
  /** Adds reverted because the account has the maximum number of favorites. */
  limitReached: string[];
  /** Changes the account rejected for another reason. */
  rejected: string[];
}

/** Sent after each sync with the account. */
export function onFavoritesSynced(handler: (event: FavoritesSynced) => void): Promise<UnlistenFn> {
  return listen<FavoritesSynced>("favorites://synced", (event) => handler(event.payload));
}
