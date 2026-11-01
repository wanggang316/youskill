import { get, writable } from "svelte/store";
import {
  listFavorites,
  onFavoritesSynced,
  setFavorite,
  type FavoritesSynced,
  type LocalFavorite,
} from "../api/favorites";
import type { RemoteSkill } from "../api/skills";

interface FavoritesState {
  /** Favorited records, newest change first. */
  items: LocalFavorite[];
  loaded: boolean;
  error: string;
  /** Favorites whose skill the marketplace no longer has, found during this session. */
  unavailable: string[];
  /** The last sync reverted adds because the account has the maximum number of favorites. */
  limitReached: boolean;
  /** The last sync reverted changes that the account rejected for another reason. */
  otherRejected: boolean;
}

/** A marketplace lookup failed because the skill left the marketplace. */
export function isMissingSkillError(error: unknown): boolean {
  return /\b404 Not Found\b/.test(String(error));
}

/** Favorites whose saved name, source or description contains `query`. */
export function filterFavorites<T extends Pick<LocalFavorite, "snapshot">>(
  items: T[],
  query: string
): T[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return items;
  return items.filter(({ snapshot }) =>
    [snapshot.name, snapshot.source, snapshot.description ?? ""].some((value) =>
      value.toLowerCase().includes(needle)
    )
  );
}

interface FavoritesApi {
  list: typeof listFavorites;
  set: typeof setFavorite;
  /** Follows syncs with the account. */
  listen?: (handler: (event: FavoritesSynced) => void) => Promise<unknown>;
}

export function createFavorites(
  api: FavoritesApi = { list: listFavorites, set: setFavorite, listen: onFavoritesSynced }
) {
  const state = writable<FavoritesState>({
    items: [],
    loaded: false,
    error: "",
    unavailable: [],
    limitReached: false,
    otherRejected: false,
  });
  const patch = (value: Partial<FavoritesState>) =>
    state.update((current) => ({ ...current, ...value }));
  /** Skill ids with a change in flight, so a double click cannot send two changes. */
  const busy = new Set<string>();

  let listening: Promise<unknown> | null = null;

  const isFavorite = (skillId: string) => get(state).items.some((item) => item.skillId === skillId);

  async function reload() {
    try {
      patch({ items: await api.list(), loaded: true, error: "" });
    } catch (error) {
      patch({ loaded: true, error: String(error) });
    }
  }

  /** A sync with the account changed the records of this device. */
  function synced({ limitReached, rejected }: FavoritesSynced) {
    if (limitReached.length) patch({ limitReached: true });
    if (rejected.length) patch({ otherRejected: true });
    void reload();
  }

  async function change(skillId: string, favorited: boolean, skill?: RemoteSkill) {
    if (busy.has(skillId)) return;
    busy.add(skillId);
    patch({ error: "", limitReached: false, otherRejected: false });
    try {
      const items = await api.set(
        skillId,
        favorited,
        skill && {
          name: skill.name,
          source: skill.source,
          description: skill.description ?? null,
        }
      );
      state.update((current) => ({
        ...current,
        items,
        loaded: true,
        unavailable: favorited
          ? current.unavailable
          : current.unavailable.filter((id) => id !== skillId),
      }));
    } catch (error) {
      patch({ error: String(error) });
    } finally {
      busy.delete(skillId);
    }
  }

  return {
    state: { subscribe: state.subscribe },
    isFavorite,
    /** Reads the list and, once, starts to follow syncs with the account. */
    async load() {
      if (api.listen) {
        listening ??= api.listen(synced).catch((error) => {
          listening = null;
          console.error("Failed to listen for favorite syncs:", error);
        });
      }
      await reload();
    },
    /** Favorites `skill`, or removes it when it is a favorite. */
    toggle: (skill: RemoteSkill) => change(skill.id, !isFavorite(skill.id), skill),
    remove: (skillId: string) => change(skillId, false),
    markUnavailable(skillId: string) {
      state.update((current) =>
        current.unavailable.includes(skillId)
          ? current
          : { ...current, unavailable: [...current.unavailable, skillId] }
      );
    },
  };
}

export const favorites = createFavorites();
export const favoritesState = favorites.state;
