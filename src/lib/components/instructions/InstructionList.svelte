<script lang="ts">
  import { untrack } from "svelte";
  import { ChevronRight, Folder, Loader2, Plus, RefreshCw, Search, UserRound } from "@lucide/svelte";
  import ReferenceMarker from "$lib/components/ReferenceMarker.svelte";
  import SymlinkMarker from "$lib/components/SymlinkMarker.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import SelectField from "$lib/components/ui/SelectField.svelte";
  import { t } from "$lib/i18n";
  import type { AgentFileView, InstructionView } from "$lib/api/instructions";
  import type { LibraryFilter } from "$lib/navigation/app-shell";
  import { baseName } from "$lib/scopes";

  let {
    templates = [],
    files = [],
    totalTemplates = 0,
    selectedId = null,
    selectedFile = null,
    loading = false,
    error = "",
    search = $bindable(""),
    filter = "all",
    onSelectTemplate,
    onSelectFile,
    onRefresh,
    onImport,
    onFilterChange,
  }: {
    templates?: InstructionView[];
    files?: AgentFileView[];
    totalTemplates?: number;
    selectedId?: string | null;
    selectedFile?: string | null;
    loading?: boolean;
    error?: string;
    search?: string;
    filter?: LibraryFilter;
    onSelectTemplate: (id: string) => void;
    onSelectFile: (path: string) => void;
    onRefresh: () => void;
    onImport: () => void;
    onFilterChange: (filter: LibraryFilter) => void;
  } = $props();

  const filters: LibraryFilter[] = ["all", "changed", "uninstalled"];

  type FileGroup = { key: string; label: string; path: string | null; files: AgentFileView[] };

  /** The user level first, then one group per project, in the order the files came. */
  const fileGroups = $derived.by((): FileGroup[] => {
    const groups: FileGroup[] = [];
    for (const file of files) {
      const key = file.scope === "user" ? "user" : `project:${file.projectPath ?? ""}`;
      let group = groups.find((item) => item.key === key);
      if (!group) {
        group = {
          key,
          label: file.scope === "user" ? $t("scope.user") : baseName(file.projectPath ?? ""),
          path: file.projectPath ?? null,
          files: [],
        };
        groups.push(group);
      }
      group.files.push(file);
    }
    return groups;
  });

  /** Collapsed file group keys, remembered per machine. */
  const COLLAPSED_STORAGE_KEY = "youskill.collapsedInstructionFileGroups";

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

  // Selecting a file elsewhere (deep links, installs) must not land in a hidden row.
  $effect(() => {
    const group = fileGroups.find((item) => item.files.some((file) => file.path === selectedFile));
    if (!group) return;
    const current = untrack(() => collapsed);
    if (current.includes(group.key)) setCollapsed(current.filter((key) => key !== group.key));
  });

  const rowClass = (selected: boolean) =>
    `mx-1.5 flex w-[calc(100%-0.75rem)] flex-col gap-1 rounded-lg px-2.5 py-2 text-left transition ${
      selected ? "bg-base-content/[0.07]" : "hover:bg-base-content/[0.045]"
    }`;
</script>

<div class="flex min-h-0 min-w-0 flex-col">
  <header class="flex h-12 flex-none items-center justify-between px-4" data-window-drag-region>
    <h1 class="text-base-content truncate text-[1.05rem] font-semibold tracking-[-0.02em]">
      {$t("instructions.title")}
    </h1>
    <span class="text-base-content-faint text-xs">
      {$t("instructions.count", { count: totalTemplates })}
    </span>
  </header>

  <div class="space-y-2 px-3 pb-2.5">
    <div class="relative">
      <Search class="text-base-content-subtle absolute top-1/2 left-3 -translate-y-1/2" size={14} />
      <input
        class="border-base-300 bg-base-100 text-base-content placeholder:text-base-content-subtle focus:border-base-300 h-8 w-full rounded-xl border pr-3 pl-8 text-[13px] focus:outline-none"
        placeholder={$t("instructions.search")}
        bind:value={search}
      />
    </div>
    <div class="flex items-center gap-1.5">
      <SelectField
        value={filter}
        className="min-w-0 flex-1"
        selectClassName="h-8 text-[12px]"
        onchange={(event) => onFilterChange(event.currentTarget.value as LibraryFilter)}
      >
        {#each filters as item}
          <option value={item}>{$t(`instructions.filter.${item}`)}</option>
        {/each}
      </SelectField>
      <IconButton
        variant="outline"
        onclick={onImport}
        title={$t("instructions.import")}
        ariaLabel={$t("instructions.import")}
        class="h-8 w-8 p-0"
      >
        <Plus size={15} />
      </IconButton>
      <IconButton
        variant="outline"
        onclick={onRefresh}
        title={$t("library.refresh")}
        ariaLabel={$t("library.refresh")}
        class="h-8 w-8 p-0"
      >
        <RefreshCw size={15} class={loading ? "animate-spin" : ""} />
      </IconButton>
    </div>
  </div>

  <div
    class="min-h-0 flex-1 overflow-y-auto py-1.5 [scrollbar-color:var(--scrollbar-thumb)_transparent] [scrollbar-width:thin]"
  >
    {#if error}
      <p class="text-error px-4 py-3 text-xs">{error}</p>
    {:else if loading && templates.length === 0 && files.length === 0}
      <div class="text-base-content-muted flex items-center justify-center gap-2 py-10 text-xs">
        <Loader2 size={16} class="animate-spin" />
        {$t("instructions.loading")}
      </div>
    {:else}
      <p
        class="text-base-content-subtle px-4 pt-1 pb-1.5 text-[11px] font-medium tracking-wide uppercase"
      >
        {$t("instructions.templates")}
      </p>
      {#if templates.length === 0}
        <p class="text-base-content-faint px-4 pb-2 text-[11px]">
          {totalTemplates === 0 ? $t("instructions.empty") : $t("instructions.emptyFiltered")}
        </p>
      {/if}
      {#each templates as item (item.id)}
        {@const selected = item.id === selectedId}
        <button
          class={rowClass(selected)}
          type="button"
          onclick={() => onSelectTemplate(item.id)}
          aria-current={selected ? "true" : undefined}
        >
          <div class="flex w-full items-center gap-2">
            <span class="text-base-content min-w-0 flex-1 truncate text-[13px] font-medium">
              {item.name}
            </span>
            {#if item.hasDrift}
              <span class="tag tag-warning shrink-0">{$t("library.tag.changed")}</span>
            {:else if item.installs.length === 0}
              <span class="tag tag-neutral shrink-0">{$t("library.tag.uninstalled")}</span>
            {/if}
          </div>
          {#if item.description}
            <p class="text-base-content-subtle line-clamp-1 text-[11px]">
              {item.description}
            </p>
          {/if}
        </button>
      {/each}

      <p
        class="text-base-content-subtle px-4 pt-3 pb-1.5 text-[11px] font-medium tracking-wide uppercase"
      >
        {$t("instructions.files")}
      </p>
      {#if fileGroups.length === 0}
        <p class="text-base-content-faint px-4 pb-2 text-[11px]">
          {$t("instructions.files.empty")}
        </p>
      {/if}
      {#each fileGroups as group (group.key)}
        {@const open = !collapsed.includes(group.key)}
        <button
          class="text-base-content-muted hover:text-base-content flex w-full items-center gap-1.5 px-4 pt-1.5 pb-1 text-left text-[11px] transition"
          type="button"
          onclick={() => toggleGroup(group.key)}
          aria-expanded={open}
          title={group.path ?? undefined}
        >
          <ChevronRight size={12} class={`shrink-0 transition ${open ? "rotate-90" : ""}`} />
          {#if group.path}
            <Folder size={11} class="shrink-0" />
          {:else}
            <UserRound size={11} class="shrink-0" />
          {/if}
          <span class="min-w-0 flex-1 truncate">{group.label}</span>
          <span class="text-base-content-faint shrink-0">{group.files.length}</span>
        </button>
        {#if open}
          {#each group.files as file (file.path)}
            {@const selected = file.path === selectedFile}
            <button
              class={rowClass(selected)}
              type="button"
              onclick={() => onSelectFile(file.path)}
              aria-current={selected ? "true" : undefined}
              title={file.path}
            >
              <!-- Chevron (12px) + gap (6px): puts the file name under the group icon. -->
              <div class="flex w-full items-center gap-2 pl-[1.125rem]">
                <span class="text-base-content min-w-0 flex-1 truncate font-mono text-[12px]">
                  {file.fileName}
                </span>
                <SymlinkMarker target={file.linkTarget} />
                <ReferenceMarker references={file.references} />
                {#if file.template}
                  {#if file.template.state !== "in_sync"}
                    <span class="tag tag-warning shrink-0">{$t("library.tag.changed")}</span>
                  {:else}
                    <span class="tag tag-neutral max-w-24 shrink-0 truncate"
                      >{file.template.name}</span
                    >
                  {/if}
                {/if}
              </div>
            </button>
          {/each}
        {/if}
      {/each}
    {/if}
  </div>
</div>
