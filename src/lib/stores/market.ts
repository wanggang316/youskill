import { get, writable } from "svelte/store";
import {
  fetchRemoteSkills,
  fetchMarketCategories,
  fetchHotRepositories,
  type RemoteSkill,
  type MarketCategory,
  type MarketRepository,
} from "../api/skills";

export type MarketTab = "categories" | "repositories";
/** `favorites` shows the local favorites instead of a marketplace list. */
export type MarketView = "browse" | "favorites";

interface MarketState {
  view: MarketView;
  tab: MarketTab;
  categoryId: number | null;
  repository: string | null;
  categories: MarketCategory[];
  repositories: MarketRepository[];
  categoriesError: string;
  repositoriesError: string;
  navigationLoading: boolean;
  navigationLoaded: boolean;
  query: string;
  sortBy: string;
  skills: RemoteSkill[];
  total: number;
  hasMore: boolean;
  loading: boolean;
  loaded: boolean;
  error: string;
  scrollTop: number;
}

const PAGE_SIZE = 20;

export function createMarketplace(
  api = {
    fetchSkills: fetchRemoteSkills,
    fetchCategories: fetchMarketCategories,
    fetchRepositories: fetchHotRepositories,
  }
) {
  const state = writable<MarketState>({
    view: "browse",
    tab: "categories",
    categoryId: null,
    repository: null,
    categories: [],
    repositories: [],
    categoriesError: "",
    repositoriesError: "",
    navigationLoading: false,
    navigationLoaded: false,
    query: "",
    sortBy: "heat_score",
    skills: [],
    total: 0,
    hasMore: false,
    loading: false,
    loaded: false,
    error: "",
    scrollTop: 0,
  });
  let requestId = 0;
  let nextOffset = 0;
  const patch = (value: Partial<MarketState>) =>
    state.update((current) => ({ ...current, ...value }));

  function reset(value: Partial<MarketState> = {}) {
    // Invalidate in-flight pages before changing their query or selection.
    requestId += 1;
    nextOffset = 0;
    patch({
      ...value,
      skills: [],
      total: 0,
      hasMore: false,
      loading: false,
      loaded: false,
      error: "",
      scrollTop: 0,
    });
  }

  async function load(resetResults = true) {
    if (resetResults) reset();
    const current = get(state);
    if (!resetResults && (current.loading || !current.hasMore)) return;
    if (current.view === "favorites" || (current.tab === "repositories" && !current.repository)) {
      patch({ loaded: true });
      return;
    }
    const id = ++requestId;
    const skip = resetResults ? 0 : nextOffset;
    patch({ loading: true, error: "" });
    try {
      const response = await api.fetchSkills({
        skip,
        limit: PAGE_SIZE,
        search: current.query,
        sortBy: current.sortBy,
        sortOrder: "desc",
        tagId: current.tab === "categories" ? (current.categoryId ?? undefined) : undefined,
        source: current.tab === "repositories" ? (current.repository ?? undefined) : undefined,
      });
      if (id !== requestId) return;
      nextOffset = skip + response.skills.length;
      state.update((value) => ({
        ...value,
        skills: resetResults
          ? response.skills
          : [
              ...value.skills,
              ...response.skills.filter(
                (skill) => !value.skills.some((existing) => existing.id === skill.id)
              ),
            ],
        total: response.total,
        hasMore: response.has_more,
        loaded: true,
      }));
    } catch (error) {
      if (id === requestId) patch({ error: String(error) });
    } finally {
      if (id === requestId) patch({ loading: false });
    }
  }

  const isBrowsing = () => get(state).view === "browse";

  async function loadNavigation() {
    if (get(state).navigationLoading) return;
    patch({ navigationLoading: true, categoriesError: "", repositoriesError: "" });
    const [categories, repositories] = await Promise.allSettled([
      api.fetchCategories(),
      api.fetchRepositories(),
    ]);
    const current = get(state);
    const values: Partial<MarketState> = { navigationLoading: false, navigationLoaded: true };
    if (categories.status === "fulfilled") {
      values.categories = categories.value;
      values.categoryId = categories.value.some((item) => item.id === current.categoryId)
        ? current.categoryId
        : null;
    } else values.categoriesError = String(categories.reason);
    if (repositories.status === "fulfilled") {
      values.repositories = repositories.value;
      values.repository = repositories.value.some((item) => item.name === current.repository)
        ? current.repository
        : (repositories.value[0]?.name ?? null);
    } else values.repositoriesError = String(repositories.reason);
    patch(values);
    const next = get(state);
    if (
      next.view === "browse" &&
      ((next.tab === "categories" && next.categoryId !== current.categoryId) ||
        (next.tab === "repositories" && next.repository !== current.repository))
    ) {
      reset({ query: "" });
      await load();
    }
  }

  return {
    state: { subscribe: state.subscribe },
    async initialize() {
      await Promise.all([
        get(state).navigationLoaded ? Promise.resolve() : loadNavigation(),
        get(state).loaded || get(state).loading ? Promise.resolve() : load(),
      ]);
    },
    async refresh() {
      await loadNavigation();
      await load();
    },
    async selectTab(tab: MarketTab) {
      if (isBrowsing() && get(state).tab === tab) return;
      reset({ view: "browse", tab, query: "" });
      await load();
    },
    async selectCategory(categoryId: number | null) {
      const current = get(state);
      if (isBrowsing() && current.tab === "categories" && current.categoryId === categoryId) return;
      reset({ view: "browse", tab: "categories", categoryId, query: "" });
      await load();
    },
    async selectRepository(repository: string) {
      const current = get(state);
      if (isBrowsing() && current.tab === "repositories" && current.repository === repository)
        return;
      reset({ view: "browse", tab: "repositories", repository, query: "" });
      await load();
    },
    async showFavorites() {
      if (!isBrowsing()) return;
      reset({ view: "favorites", query: "" });
      await load();
    },
    setSearch(query: string) {
      reset({ query });
    },
    async search(query: string) {
      reset({ query });
      await load();
    },
    async sort(sortBy: string) {
      reset({ sortBy });
      await load();
    },
    reload: () => load(),
    loadMore: () => load(false),
    saveScroll: (scrollTop: number) => patch({ scrollTop }),
  };
}

export const marketplace = createMarketplace();
export const marketState = marketplace.state;
