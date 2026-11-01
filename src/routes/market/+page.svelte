<script lang="ts">
  import { get } from "svelte/store";
  import { onMount, onDestroy } from "svelte";
  import CatalogIcon from "$lib/components/CatalogIcon.svelte";
  import MarketSkillDetail from "$lib/components/MarketSkillDetail.svelte";
  import SplitView from "$lib/components/SplitView.svelte";
  import SegmentedTabs from "$lib/components/ui/SegmentedTabs.svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import {
    Check,
    ChevronDown,
    ArrowDownWideNarrow,
    Heart,
    Loader2,
    RefreshCw,
    Search,
    Star,
  } from "@lucide/svelte";
  import { DropdownMenu } from "bits-ui";
  import { menuContentClass, menuItemClass } from "$lib/components/ui/menu";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import { t } from "$lib/i18n";
  import {
    detectGithubAuto,
    importSkills,
    recordInstall,
    syncSkill,
    type RemoteSkill,
    type SkillSource,
  } from "$lib/api";
  import { hubSkills, refreshHub } from "$lib/stores/hub";
  import { favorites, favoritesState, filterFavorites } from "$lib/stores/favorites";
  import { marketplace, marketState, type MarketTab } from "$lib/stores/market";
  import { openInstallModal, performAction } from "$lib/stores/modals";
  import { buildMarketHref } from "$lib/navigation/app-shell";

  let busySkillId = $state("");
  let log = $state("");
  let viewport = $state<HTMLElement | null>(null);
  let searchTimeout = $state<ReturnType<typeof setTimeout> | null>(null);
  const showingFavorites = $derived($marketState.view === "favorites");
  const favoriteIds = $derived(new Set($favoritesState.items.map((item) => item.skillId)));
  const shownFavorites = $derived(filterFavorites($favoritesState.items, $marketState.query));
  const title = $derived(
    showingFavorites
      ? $t("market.favorites")
      : $marketState.tab === "categories"
        ? ($marketState.categories.find((item) => item.id === $marketState.categoryId)?.name ??
          $t("market.all"))
        : ($marketState.repository ?? $t("market.repositories"))
  );
  const repositoryStars = $derived(
    $marketState.repositories.find((item) => item.name === $marketState.repository)?.star_count
  );
  const sortItems = $derived([
    { value: "heat_score", label: $t("market.sort.popular") },
    { value: "star_count", label: $t("market.sort.stars") },
  ]);
  const navigationError = $derived(
    $marketState.tab === "categories"
      ? $marketState.categoriesError
      : $marketState.repositoriesError
  );
  const noRepository = $derived(
    !showingFavorites && $marketState.tab === "repositories" && !$marketState.repository
  );
  /** Marketplace id of the skill shown in the content panel (`?skill=`). */
  const detailId = $derived(page.url.searchParams.get("skill"));

  onMount(() => {
    void marketplace.initialize();
    if (!$favoritesState.loaded) void favorites.load();
  });
  // The list remounts when the detail closes; put it back where the user left it.
  $effect(() => {
    if (viewport) viewport.scrollTop = get(marketState).scrollTop;
  });
  onDestroy(() => {
    if (searchTimeout) clearTimeout(searchTimeout);
  });

  function run(action: () => Promise<void>) {
    if (searchTimeout) clearTimeout(searchTimeout);
    searchTimeout = null;
    if (viewport) viewport.scrollTop = 0;
    // Picking a category or repository while a skill is open goes back to the list.
    if (detailId) void goto(buildMarketHref());
    void action();
  }

  function handleSearchInput(event: Event) {
    if (searchTimeout) clearTimeout(searchTimeout);
    marketplace.setSearch((event.currentTarget as HTMLInputElement).value);
    if (viewport) viewport.scrollTop = 0;
    searchTimeout = setTimeout(() => {
      searchTimeout = null;
      void marketplace.reload();
    }, 500);
  }

  const repoSlug = (skill: RemoteSkill) => skill.source.trim();

  /** The hub skill imported from this marketplace entry: same repository and name. */
  function hubEntry(skill: RemoteSkill) {
    const repo = repoSlug(skill).toLowerCase();
    return (
      $hubSkills.find(
        (entry) =>
          entry.name === skill.name &&
          entry.source.type === "github" &&
          entry.source.repo.toLowerCase() === repo
      ) ?? null
    );
  }

  function hasUpdate(skill: RemoteSkill): boolean {
    const entry = hubEntry(skill);
    if (!entry || entry.source.type !== "github") return false;
    if (entry.sourceState === "update_available") return true;
    const remoteSha = skill.skill_path_sha;
    return Boolean(remoteSha && entry.source.remoteSha && entry.source.remoteSha !== remoteSha);
  }

  function buttonLabel(skill: RemoteSkill): string {
    if (busySkillId === skill.id) return $t("market.downloading");
    if (hasUpdate(skill)) return $t("market.update");
    if (hubEntry(skill)) return $t("market.reimport");
    return $t("market.import");
  }

  async function handleImport(skill: RemoteSkill) {
    if (!skill.url || busySkillId) return;
    busySkillId = skill.id;
    log = "";
    try {
      const existing = hubEntry(skill);
      if (existing && hasUpdate(skill) && existing.source.type === "github") {
        const result = await performAction((force) =>
          syncSkill(existing.id, { kind: "pull_source", force })
        );
        if (!result.applied && result.blockers.length > 0) {
          log = result.blockers.join("\n");
        }
        await refreshHub();
        return;
      }

      const detected = await detectGithubAuto(skill.url, skill.name);
      const repo = repoSlug(skill);
      const source: SkillSource = {
        type: "github",
        repo,
        url: `https://github.com/${repo}.git`,
        skillPath: detected.skill_path,
        branch: detected.branch ?? skill.branch ?? null,
        remoteSha: skill.skill_path_sha ?? null,
        marketplaceId: skill.id,
      };
      const outcomes = await importSkills(
        [{ name: detected.name, tmpPath: detected.tmp_path, source }],
        Boolean(existing)
      );
      recordInstall(skill.id).catch((error) => console.error("Failed to record install:", error));
      await refreshHub();
      openInstallModal(outcomes.map((outcome) => outcome.id));
    } catch (error) {
      log = String(error);
    } finally {
      busySkillId = "";
    }
  }

  function handleView(skillId: string) {
    goto(buildMarketHref(skillId));
  }

  function refresh() {
    if (showingFavorites) void favorites.load();
    run(() => marketplace.refresh());
  }
</script>

<section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden">
  <SplitView>
    <aside class="flex min-h-0 min-w-0 flex-col" aria-label={$t("market.browse")}>
      <header class="flex h-12 shrink-0 items-center px-5" data-window-drag-region>
        <h1 class="text-base-content text-[1.05rem] font-semibold tracking-[-0.02em]">
          {$t("market.title")}
        </h1>
      </header>
      <div class="px-3 pb-2">
        <button
          type="button"
          class="market-nav-row"
          class:active={showingFavorites}
          aria-current={showingFavorites ? "true" : undefined}
          onclick={() => run(() => marketplace.showFavorites())}
        >
          <Heart size={16} class="shrink-0" aria-hidden="true" />
          <span class="min-w-0 flex-1">{$t("market.favorites")}</span>
          <span class="text-base-content-subtle shrink-0 text-xs tabular-nums"
            >{$favoritesState.items.length}</span
          >
        </button>
      </div>
      <div class="px-3 pb-3">
        <SegmentedTabs
          items={[
            { value: "categories", label: $t("market.categories") },
            { value: "repositories", label: $t("market.repositories") },
          ]}
          className="[&>button]:px-2 [&>button]:text-xs [&>button]:whitespace-nowrap"
          value={$marketState.tab}
          onChange={(value) => run(() => marketplace.selectTab(value as MarketTab))}
          fullWidth
          surface="sidebar"
        />
      </div>
      <nav
        class="min-h-0 flex-1 space-y-1 overflow-y-auto px-3 pb-4"
        aria-label={$t(
          $marketState.tab === "categories" ? "market.categories" : "market.repositories"
        )}
      >
        {#if $marketState.tab === "categories"}
          <button
            type="button"
            class="market-nav-row"
            class:active={!showingFavorites && $marketState.categoryId === null}
            aria-current={!showingFavorites && $marketState.categoryId === null
              ? "true"
              : undefined}
            onclick={() => run(() => marketplace.selectCategory(null))}
          >
            <CatalogIcon fallback="all" />
            <span class="min-w-0 flex-1">{$t("market.all")}</span>
          </button>
          {#each $marketState.categories as category (category.id)}
            <button
              type="button"
              class="market-nav-row"
              class:active={!showingFavorites && $marketState.categoryId === category.id}
              aria-current={!showingFavorites && $marketState.categoryId === category.id
                ? "true"
                : undefined}
              onclick={() => run(() => marketplace.selectCategory(category.id))}
            >
              <CatalogIcon icon={category.icon} />
              <span class="min-w-0 flex-1 truncate" title={category.name}>{category.name}</span>
              <span class="text-base-content-subtle shrink-0 text-xs tabular-nums"
                >{category.skill_count}</span
              >
            </button>
          {/each}
        {:else}
          {#each $marketState.repositories as repository (repository.id)}
            <button
              type="button"
              class="market-nav-row"
              class:active={!showingFavorites && $marketState.repository === repository.name}
              aria-current={!showingFavorites && $marketState.repository === repository.name
                ? "true"
                : undefined}
              onclick={() => run(() => marketplace.selectRepository(repository.name))}
            >
              <CatalogIcon icon={repository.icon} fallback="repository" />
              <span class="min-w-0 flex-1 text-left">
                <span class="block truncate font-medium" title={repository.name}
                  >{repository.name.split("/").slice(1).join("/") || repository.name}</span
                >
                <span class="text-base-content-subtle block truncate text-xs"
                  >{repository.name.split("/")[0]}</span
                >
              </span>
              <span class="text-base-content-subtle shrink-0 text-xs tabular-nums"
                >{repository.skill_count}</span
              >
            </button>
          {/each}
        {/if}
        {#if $marketState.navigationLoading}
          <p class="text-base-content-muted px-3 py-4 text-xs" role="status">
            {$t("market.loading")}
          </p>
        {:else if $marketState.tab === "repositories" && !$marketState.repositories.length && !$marketState.repositoriesError}
          <p class="text-base-content-muted px-3 py-4 text-xs">{$t("market.repositories.empty")}</p>
        {/if}
        {#if navigationError}
          <div class="px-3 py-4 text-xs">
            <p class="text-error break-words" role="alert">{navigationError}</p>
            <button
              type="button"
              class="settings-button mt-3"
              onclick={() => run(() => marketplace.refresh())}>{$t("common.retry")}</button
            >
          </div>
        {/if}
      </nav>
    </aside>

    {#if detailId}
      <MarketSkillDetail id={detailId} onBack={() => goto(buildMarketHref())} />
    {:else}
      <div class="surface-panel flex min-h-0 min-w-0 flex-col">
        <header
          class="border-base-300 flex h-12 shrink-0 items-center justify-between gap-3 border-b px-6"
          data-window-drag-region
        >
          <div class="flex min-w-0 items-center gap-3">
            <h2 class="text-base-content truncate text-sm font-semibold" {title}>{title}</h2>
            {#if !showingFavorites && $marketState.tab === "repositories" && repositoryStars != null}
              <span
                class="text-base-content-subtle inline-flex shrink-0 items-center gap-1 text-xs tabular-nums"
                aria-label={`${repositoryStars.toLocaleString()} stars`}
              >
                <Star size={13} aria-hidden="true" />{repositoryStars.toLocaleString()}
              </span>
            {/if}
          </div>
          <span class="text-base-content-subtle shrink-0 text-xs tabular-nums" role="status">
            {#if showingFavorites}{$t("library.count", {
                count: shownFavorites.length,
              })}{:else if $marketState.loading}{$t(
                "market.loading"
              )}{:else if $marketState.loaded}{$t("library.count", {
                count: $marketState.total,
              })}{/if}
          </span>
        </header>
        <div class="border-base-300 flex shrink-0 flex-wrap items-center gap-3 border-b px-6 py-4">
          <div class="relative min-w-32 flex-1">
            <Search
              class="text-base-content-subtle absolute top-1/2 left-3 -translate-y-1/2"
              size={16}
            />
            <input
              class="border-base-300 bg-base-200 text-base-content placeholder:text-base-content-subtle focus:ring-primary h-9 w-full rounded-lg border pr-3 pl-9 text-sm focus:ring-1 focus:outline-none disabled:opacity-50"
              placeholder={$t("market.search.scope", { name: title })}
              aria-label={$t("market.search.scope", { name: title })}
              value={$marketState.query}
              oninput={handleSearchInput}
              disabled={noRepository}
            />
          </div>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger
              class="border-base-300 text-base-content-muted hover:bg-base-200 hover:text-base-content data-[state=open]:bg-base-200 flex size-9 shrink-0 items-center justify-center rounded-lg border transition disabled:opacity-40"
              aria-label={$t("market.sort")}
              title={`${$t("market.sort")}: ${sortItems.find((item) => item.value === $marketState.sortBy)?.label}`}
              disabled={noRepository || showingFavorites}
            >
              <ArrowDownWideNarrow size={16} />
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                class={menuContentClass}
                align="end"
                sideOffset={4}
                collisionPadding={8}
              >
                <DropdownMenu.RadioGroup
                  value={$marketState.sortBy}
                  onValueChange={(value) => run(() => marketplace.sort(value))}
                >
                  {#each sortItems as item (item.value)}
                    <DropdownMenu.RadioItem
                      value={item.value}
                      class={`${menuItemClass()} flex items-center justify-between gap-3`}
                    >
                      {#snippet children({ checked })}
                        {item.label}<Check size={13} class={checked ? "" : "invisible"} />
                      {/snippet}
                    </DropdownMenu.RadioItem>
                  {/each}
                </DropdownMenu.RadioGroup>
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
          <IconButton
            variant="outline"
            onclick={refresh}
            disabled={$marketState.navigationLoading || $marketState.loading}
            title={$t("library.refresh")}
            ariaLabel={$t("library.refresh")}
            class="h-9 w-9"
          >
            <RefreshCw
              size={16}
              class={$marketState.loading || $marketState.navigationLoading ? "animate-spin" : ""}
            />
          </IconButton>
        </div>
        <main
          bind:this={viewport}
          class="min-h-0 flex-1 overflow-y-auto"
          onscroll={() => {
            // Unmounting the list fires a last scroll event; keep the saved position.
            if (viewport) marketplace.saveScroll(viewport.scrollTop);
          }}
          aria-busy={$marketState.loading || searchTimeout !== null}
        >
          {#if $favoritesState.error}
            <p class="text-error px-6 pt-4 text-sm" role="alert">{$favoritesState.error}</p>
          {/if}
          {#if $favoritesState.limitReached}
            <p class="text-error px-6 pt-4 text-sm" role="alert">
              {$t("market.favorites.limitReached")}
            </p>
          {/if}
          {#if $favoritesState.otherRejected}
            <p class="text-error px-6 pt-4 text-sm" role="alert">
              {$t("market.favorites.rejected")}
            </p>
          {/if}
          {#if showingFavorites}
            {#if !shownFavorites.length}
              <p class="text-base-content-muted p-12 text-center text-sm">
                {$t($marketState.query.trim() ? "market.empty" : "market.favorites.empty")}
              </p>
            {:else}
              <ul class="divide-base-300 divide-y px-6">
                {#each shownFavorites as favorite (favorite.skillId)}
                  {@const unavailable = $favoritesState.unavailable.includes(favorite.skillId)}
                  <li class="hover:bg-base-200 flex items-center gap-4 py-4 transition-colors">
                    <button
                      type="button"
                      class="focus-visible:outline-primary min-w-0 flex-1 rounded text-left"
                      onclick={() => handleView(favorite.skillId)}
                    >
                      <span class="flex min-w-0 items-center gap-2">
                        <span class="text-base-content truncate text-sm font-medium"
                          >{favorite.snapshot.name}</span
                        >
                        {#if unavailable}<span class="text-warning shrink-0 text-xs"
                            >{$t("market.unavailable")}</span
                          >{/if}
                      </span>
                      <span class="text-base-content-subtle mt-1 block truncate text-xs"
                        >{favorite.snapshot.source}</span
                      >
                      {#if favorite.snapshot.description}<span
                          class="text-base-content-muted mt-2 line-clamp-2 text-xs leading-relaxed"
                          >{favorite.snapshot.description}</span
                        >{/if}
                    </button>
                    <IconButton
                      onclick={() => favorites.remove(favorite.skillId)}
                      title={$t("market.unfavorite")}
                      class="text-secondary h-8 w-8 shrink-0 p-0"
                    >
                      <Heart size={16} fill="currentColor" />
                    </IconButton>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else}
            {#if $marketState.error}
              <div class="px-6 py-4 text-sm" role="alert">
                <p class="text-error">{$marketState.error}</p>
                <button
                  class="settings-button mt-3"
                  type="button"
                  onclick={() =>
                    $marketState.skills.length ? marketplace.loadMore() : marketplace.reload()}
                  >{$t("common.retry")}</button
                >
              </div>
            {/if}
            {#if log}<div
                class="bg-base-200 text-base-content-muted mx-6 mt-4 rounded-lg px-3 py-2 text-xs whitespace-pre-wrap"
              >
                {log}
              </div>{/if}
            {#if ($marketState.loading || searchTimeout !== null) && !$marketState.skills.length}
              <div
                class="text-base-content-muted flex items-center justify-center gap-2 p-12 text-sm"
                role="status"
              >
                <Loader2 size={16} class="animate-spin" />{$t("market.loading")}
              </div>
            {:else if noRepository}
              <p class="text-base-content-muted p-12 text-center text-sm">
                {$t(
                  $marketState.navigationLoading ? "market.loading" : "market.repositories.empty"
                )}
              </p>
            {:else if !$marketState.skills.length && !$marketState.error}
              <p class="text-base-content-muted p-12 text-center text-sm">{$t("market.empty")}</p>
            {:else}
              <ul class="divide-base-300 divide-y px-6">
                {#each $marketState.skills as skill (skill.id)}
                  {@const inHub = Boolean(hubEntry(skill))}
                  {@const favorite = favoriteIds.has(skill.id)}
                  <li class="hover:bg-base-200 flex items-center gap-4 py-4 transition-colors">
                    <button
                      type="button"
                      class="focus-visible:outline-primary min-w-0 flex-1 rounded text-left"
                      onclick={() => handleView(skill.id)}
                    >
                      <span class="flex min-w-0 items-center gap-2">
                        <span class="text-base-content truncate text-sm font-medium"
                          >{skill.name}</span
                        >
                        {#if inHub}<span
                            class="text-success inline-flex shrink-0 items-center gap-1 text-xs"
                            ><Check size={12} />{$t("market.imported")}</span
                          >{/if}
                      </span>
                      {#if $marketState.tab === "categories"}
                        <span
                          class="text-base-content-subtle mt-1 flex min-w-0 items-center gap-3 text-xs"
                        >
                          <span class="truncate">{skill.source}</span>
                          <span
                            class="inline-flex shrink-0 items-center gap-1 tabular-nums"
                            aria-label={`${skill.star_count.toLocaleString()} stars`}
                          >
                            <Star size={12} aria-hidden="true" />{skill.star_count.toLocaleString()}
                          </span>
                        </span>
                      {/if}
                      {#if skill.description}<span
                          class="text-base-content-muted mt-2 line-clamp-2 text-xs leading-relaxed"
                          >{skill.description}</span
                        >{/if}
                    </button>
                    <IconButton
                      onclick={() => favorites.toggle(skill)}
                      title={$t(favorite ? "market.unfavorite" : "market.favorite")}
                      class={`h-8 w-8 shrink-0 p-0 ${favorite ? "text-secondary" : "text-base-content-subtle"}`}
                    >
                      <Heart size={16} fill={favorite ? "currentColor" : "none"} />
                    </IconButton>
                    <button
                      class="settings-button shrink-0"
                      onclick={() => handleImport(skill)}
                      disabled={Boolean(busySkillId)}
                      type="button"
                    >
                      {#if busySkillId === skill.id}<Loader2
                          size={12}
                          class="animate-spin"
                        />{/if}{buttonLabel(skill)}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if $marketState.skills.length}
              <div class="flex justify-center p-6">
                <button
                  class="settings-button"
                  onclick={() => marketplace.loadMore()}
                  disabled={!$marketState.hasMore || $marketState.loading}
                  type="button"
                >
                  {#if $marketState.loading}<Loader2
                      size={14}
                      class="animate-spin"
                    />{:else if $marketState.hasMore}<ChevronDown size={14} />{/if}
                  {$t($marketState.hasMore ? "market.loadMore" : "market.noMore")}
                </button>
              </div>
            {/if}
          {/if}
        </main>
      </div>
    {/if}
  </SplitView>
</section>

<style>
  .market-nav-row {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    border-radius: 0.5rem;
    padding: 0.625rem 0.75rem;
    text-align: left;
    font-size: 0.8125rem;
    color: var(--color-base-content-muted);
  }
  .market-nav-row:hover {
    background: color-mix(in oklch, var(--color-base-content) 4.5%, transparent);
    color: var(--color-base-content);
  }
  .market-nav-row.active {
    background: color-mix(in oklch, var(--color-base-content) 7%, transparent);
    color: var(--color-base-content);
    font-weight: 500;
  }
  .market-nav-row:focus-visible {
    outline: 2px solid var(--color-primary);
    outline-offset: -2px;
  }
</style>
