import { get, writable } from "svelte/store";
import { listFavorites, setFavorite, type LocalFavorite } from "../api/favorites";
import type { RemoteSkill } from "../api/skills";

interface FavoritesState {
  /** Favorited records, newest change first. */
  items: LocalFavorite[];
  loaded: boolean;
  error: string;
  /** Favorites whose skill the marketplace no longer has, found during this session. */
  unavailable: string[];
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

export function createFavorites(api = { list: listFavorites, set: setFavorite }) {
  const state = writable<FavoritesState>({
    items: [],
    loaded: false,
    error: "",
    unavailable: [],
  });
  const patch = (value: Partial<FavoritesState>) =>
    state.update((current) => ({ ...current, ...value }));
  /** Skill ids with a change in flight, so a double click cannot send two changes. */
  const busy = new Set<string>();

  const isFavorite = (skillId: string) => get(state).items.some((item) => item.skillId === skillId);

  async function change(skillId: string, favorited: boolean, skill?: RemoteSkill) {
    if (busy.has(skillId)) return;
    busy.add(skillId);
    patch({ error: "" });
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
    async load() {
      try {
        patch({ items: await api.list(), loaded: true, error: "" });
      } catch (error) {
        patch({ loaded: true, error: String(error) });
      }
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
