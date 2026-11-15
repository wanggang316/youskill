<script lang="ts">
  import { untrack } from "svelte";
  import { Folder, Loader2, RefreshCw, ScanSearch, Search, Zap } from "@lucide/svelte";
  import GithubAvatar from "$lib/components/library/GithubAvatar.svelte";
  import GroupActionsMenu from "$lib/components/library/GroupActionsMenu.svelte";
  import LibraryFilterMenu from "$lib/components/library/LibraryFilterMenu.svelte";
  import CollapsibleGroup from "$lib/components/ui/CollapsibleGroup.svelte";
  import type { MenuItem } from "$lib/components/ui/menu";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import type { HubSkillView } from "$lib/api/hub";
  import { groupLibrarySkills, type LibraryGroup } from "$lib/library-groups";
  import type { RepositoryAction } from "$lib/repository-actions";
  import { skillLabel } from "$lib/stores/hub";
  import {
    buildSettingsHref,
    type LibraryFilter,
    type LibrarySource,
  } from "$lib/navigation/app-shell";

  let {
    skills = [],
    totalCount = 0,
    selectedId = null,
    checkedIds = [],
    loading = false,
    checking = false,
    error = "",
    updatingGroups = [],
    search = $bindable(""),
    filter = "all",
    source = "all",
    onSelect,
    onToggleChecked,
    onBatchInstall,
    onBatchRemove,
    onClearChecked,
    onRefresh,
    onScan,
    onFilterChange,
    onSourceChange,
    onRepositoryAction,
  }: {
    skills?: HubSkillView[];
    totalCount?: number;
    selectedId?: string | null;
    /** Skills picked with Shift+click for a batch install or removal. */
    checkedIds?: string[];
    loading?: boolean;
    /** Source update check in progress. */
    checking?: boolean;
    error?: string;
    /** Keys of the repository groups that an Update runs on. */
    updatingGroups?: string[];
    search?: string;
    filter?: LibraryFilter;
    source?: LibrarySource;
    onSelect: (id: string) => void;
    onToggleChecked: (id: string) => void;
    onBatchInstall: () => void;
    onBatchRemove: () => void;
    onClearChecked: () => void;
    onRefresh: () => void;
    onScan: () => void;
    onFilterChange: (filter: LibraryFilter) => void;
    onSourceChange: (source: LibrarySource) => void;
    onRepositoryAction: (group: LibraryGroup, action: RepositoryAction) => void;
  } = $props();

  const repositoryActions: RepositoryAction[] = ["import", "install", "update", "remove"];

  const repositoryMenu = (group: LibraryGroup): MenuItem[] =>
    repositoryActions.map((action) => ({
      label: $t(`library.repoActions.${action}`),
      danger: action === "remove",
      onSelect: () => onRepositoryAction(group, action),
    }));

  /**
   * Placeholder groups shown while the library loads: label width, then name and description
   * widths in percent per row. Keep in sync with the boot skeleton in app.html.
   */
  const skeletonGroups = [
    {
      label: 52,
      rows: [
        [46, 86],
        [30, 88],
      ],
    },
    {
      label: 44,
      rows: [
        [54, 84],
        [40, 90],
        [58, 82],
        [36, 86],
        [50, 88],
      ],
    },
  ];

  const groups = $derived(groupLibrarySkills(skills));

  /** Collapsed group keys, remembered per machine. */
  const COLLAPSED_STORAGE_KEY = "youskill.collapsedLibraryGroups";

  const readCollapsed = (): string[] => {
    try {
      const parsed: unknown = JSON.parse(localStorage.getItem(COLLAPSED_STORAGE_KEY) ?? "[]");
      return Array.isArray(parsed) ? parsed.filter((key) => typeof key === "string") : [];
    } catch {
      return [];
    }
  };

  let collapsed = $state<string[]>(readCollapsed());

  function setCollapsed(keys: string[]) {
    collapsed = keys;
    try {
      localStorage.setItem(COLLAPSED_STORAGE_KEY, JSON.stringify(keys));
    } catch {
      // Remembering the state is best effort.
    }
  }

  const toggleGroup = (key: string) =>
    setCollapsed(
      collapsed.includes(key) ? collapsed.filter((item) => item !== key) : [...collapsed, key]
    );

  // Selecting a skill elsewhere (deep links, imports) must not land in a hidden row.
  $effect(() => {
    const group = groups.find((item) => item.skills.some((skill) => skill.id === selectedId));
    if (!group) return;
    const current = untrack(() => collapsed);
    if (current.includes(group.key)) setCollapsed(current.filter((key) => key !== group.key));
  });

  function handleRowClick(event: MouseEvent, id: string) {
    if (event.shiftKey) {
      onToggleChecked(id);
      return;
    }
    onSelect(id);
  }
</script>

<div class="flex min-h-0 min-w-0 flex-col">
  <header class="flex h-12 flex-none items-center justify-between px-4" data-window-drag-region>
    <div class="flex min-w-0 items-baseline gap-2">
      <h1 class="text-base-content truncate text-[1.05rem] font-semibold tracking-[-0.02em]">
        {$t("library.title")}
      </h1>
      <span class="text-base-content-faint shrink-0 text-xs">
        {$t("library.count", { count: totalCount })}
      </span>
    </div>
    <div class="flex shrink-0 items-center gap-1.5">
      <IconButton
        variant="outline"
        onclick={onScan}
        title={$t("library.scan")}
        ariaLabel={$t("library.scan")}
        class="h-8 w-8 p-0"
      >
        <ScanSearch size={15} />
      </IconButton>
      <IconButton
        variant="outline"
        onclick={onRefresh}
        title={$t("library.refresh")}
        ariaLabel={$t("library.refresh")}
        class="h-8 w-8 p-0"
      >
        <RefreshCw size={15} class={loading || checking ? "animate-spin" : ""} />
      </IconButton>
    </div>
  </header>

  <div class="flex items-center gap-1.5 px-3 pb-2.5">
    <div class="relative min-w-0 flex-1">
      <Search class="text-base-content-subtle absolute top-1/2 left-3 -translate-y-1/2" size={14} />
      <input
        class="border-base-300 bg-base-100 text-base-content placeholder:text-base-content-subtle focus:border-base-300 h-8 w-full rounded-xl border pr-3 pl-8 text-[13px] focus:outline-none"
        placeholder={$t("library.search")}
        bind:value={search}
      />
    </div>
    <LibraryFilterMenu {filter} {source} {onFilterChange} {onSourceChange} />
  </div>

  <div
    class="min-h-0 flex-1 overflow-y-auto py-1.5 [scrollbar-color:var(--scrollbar-thumb)_transparent] [scrollbar-width:thin]"
  >
    {#if error}
      <p class="text-error px-4 py-3 text-xs">{error}</p>
    {:else if loading && skills.length === 0}
      <div role="status" aria-label={$t("library.loading")}>
        {#each skeletonGroups as group}
          <div class="flex items-center gap-1.5 px-4 pt-2 pb-1" aria-hidden="true">
            <div class="bg-base-content/[0.06] size-4 animate-pulse rounded-full"></div>
            <div
              class="bg-base-content/[0.06] h-[9px] animate-pulse rounded-full"
              style={`width: ${group.label}%`}
            ></div>
          </div>
          {#each group.rows as [name, description]}
            <div class="mx-1.5 flex flex-col gap-1 py-2 pr-2.5 pl-8" aria-hidden="true">
              <div
                class="bg-base-content/[0.09] my-[4.5px] h-[11px] animate-pulse rounded-full"
                style={`width: ${name}%`}
              ></div>
              <div
                class="bg-base-content/[0.06] my-[3.5px] h-[9px] animate-pulse rounded-full"
                style={`width: ${description}%`}
              ></div>
            </div>
          {/each}
        {/each}
      </div>
    {:else if skills.length === 0}
      <p class="text-base-content-muted px-4 py-8 text-center text-xs">
        {totalCount === 0 ? $t("library.empty") : $t("library.emptyFiltered")}
      </p>
      {#if totalCount === 0}
        <p class="-mt-5 px-4 pb-8 text-center text-xs">
          <a class="text-primary hover:underline" href={buildSettingsHref("backup")}>
            {$t("library.restoreFromGithub")}
          </a>
        </p>
      {/if}
    {:else}
      {#each groups as group (group.key)}
        {@const open = !collapsed.includes(group.key)}
        {@const repository = group.kind === "github"}
        {@const updating = updatingGroups.includes(group.key)}
        <CollapsibleGroup
          {open}
          onToggle={() => toggleGroup(group.key)}
          label={group.kind === "none" ? $t("library.group.none") : group.label}
          title={group.path ?? undefined}
          class="relative"
          buttonClass={`text-base-content-subtle hover:text-base-content pt-2 pb-1 pl-4 text-[11px] ${
            repository ? "pr-10" : "pr-4"
          }`}
          labelClass="font-medium"
        >
          {#snippet leading()}
            <!-- A fixed 16px slot keeps the title at one x for every icon kind. -->
            <span class="flex size-4 shrink-0 items-center justify-center">
              {#if group.owner}
                <GithubAvatar owner={group.owner} />
              {:else if group.kind === "folder"}
                <Folder size={13} class="shrink-0" />
              {:else}
                <Zap size={13} class="shrink-0" />
              {/if}
            </span>
          {/snippet}
          {#snippet trailing()}
            {#if updating}
              <span class="text-base-content-muted flex items-center gap-1">
                <Loader2 size={11} class="animate-spin" />
                {$t("library.repoActions.updating")}
              </span>
            {:else}
              <span class="text-base-content-faint">{group.skills.length}</span>
            {/if}
          {/snippet}
          {#snippet actions()}
            {#if repository}
              <div class="absolute top-1/2 right-3.5 mt-0.5 -translate-y-1/2">
                <GroupActionsMenu
                  items={repositoryMenu(group)}
                  label={$t("library.repoActions")}
                  disabled={updating}
                />
              </div>
            {/if}
          {/snippet}
          {#each group.skills as skill (skill.id)}
            {@const selected = skill.id === selectedId}
            {@const checked = checkedIds.includes(skill.id)}
            <!-- mx-1.5 + pl-8 starts the text at the group title: pl-4 + 16px icon + gap-1.5. -->
            <button
              class={`mx-1.5 flex w-[calc(100%-0.75rem)] flex-col gap-1 rounded-lg py-2 pr-2.5 pl-8 text-left transition select-none ${
                checked
                  ? "bg-primary/10 ring-primary/40 ring-1 ring-inset"
                  : selected
                    ? "bg-base-content/[0.07]"
                    : "hover:bg-base-content/[0.045]"
              }`}
              type="button"
              onclick={(event) => handleRowClick(event, skill.id)}
              aria-current={selected ? "true" : undefined}
              aria-pressed={checkedIds.length > 0 ? checked : undefined}
            >
              <div class="flex w-full items-center gap-2">
                <span class="text-base-content min-w-0 flex-1 truncate text-[13px] font-medium">
                  {$skillLabel(skill.id)}
                </span>
                {#if skill.hasDrift}
                  <span class="tag tag-warning shrink-0">{$t("library.tag.changed")}</span>
                {:else if skill.installs.length === 0}
                  <span class="tag tag-neutral shrink-0">{$t("library.tag.uninstalled")}</span>
                {/if}
              </div>
              {#if skill.description}
                <p class="text-base-content-subtle line-clamp-1 text-[11px]">
                  {skill.description}
                </p>
              {/if}
            </button>
          {/each}
        </CollapsibleGroup>
      {/each}
    {/if}
  </div>

  {#if checkedIds.length > 0}
    <div class="flex flex-none items-center gap-2 px-3 py-2.5">
      <PrimaryActionButton
        className="min-w-0 flex-1 px-3 py-1.5 text-[13px] whitespace-nowrap"
        onclick={onBatchInstall}
      >
        {$t("library.batchInstall", { count: checkedIds.length })}
      </PrimaryActionButton>
      <button
        class="border-error/40 text-error hover:bg-error/10 shrink-0 rounded-xl border px-3 py-1.5 text-[13px] whitespace-nowrap transition"
        type="button"
        onclick={onBatchRemove}
      >
        {$t("library.batchRemove")}
      </button>
      <button
        class="border-base-300 text-base-content hover:bg-base-200 shrink-0 rounded-xl border px-3 py-1.5 text-[13px] whitespace-nowrap transition"
        type="button"
        onclick={onClearChecked}
      >
        {$t("common.cancel")}
      </button>
    </div>
  {/if}
</div>
