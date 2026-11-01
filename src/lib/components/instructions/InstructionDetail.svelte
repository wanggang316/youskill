<script lang="ts">
  import { untrack } from "svelte";
  import { Check, ExternalLink, FolderOpen, Pencil, Plus, Trash2, X } from "@lucide/svelte";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import SegmentedTabs from "$lib/components/ui/SegmentedTabs.svelte";
  import MarkdownEditor from "$lib/components/MarkdownEditor.svelte";
  import InstallTargets from "$lib/components/library/InstallTargets.svelte";
  import ItemScopeField from "$lib/components/library/ItemScopeField.svelte";
  import DriftPanel, { type TargetAction } from "$lib/components/library/DriftPanel.svelte";
  import { t } from "$lib/i18n";
  import {
    instructionSourceLabel,
    instructionSourceUrl,
    readInstruction,
    renameInstruction,
    writeInstruction,
    type InstructionView,
  } from "$lib/api/instructions";
  import { openInFileManager } from "$lib/api/skills";
  import { applyInstructionView, refreshInstructions } from "$lib/stores/instructions";
  import type { InstallScope, InstallView, SyncAction } from "$lib/api/hub";
  import type { AgentInfo } from "$lib/api/skills";
  import type { UserProject } from "$lib/api/user-projects";

  type DetailTab = "general" | "content";

  let {
    instruction,
    projects = [],
    agents,
    homePath = "",
    busy = false,
    actionError = "",
    onInstall,
    onUninstallLocation,
    onManageLocations,
    onTargetAction,
    onSync,
    onRemove,
    onOpenFile,
  }: {
    instruction: InstructionView;
    projects?: UserProject[];
    agents: Map<string, AgentInfo>;
    homePath?: string;
    busy?: boolean;
    actionError?: string;
    onInstall: (scope: InstallScope, projectPath: string | null, lockScope?: boolean) => void;
    onUninstallLocation: (
      scope: InstallScope,
      projectPath: string | null,
      installs: InstallView[]
    ) => void;
    onManageLocations: () => void;
    onTargetAction: (install: InstallView, action: TargetAction) => void;
    onSync: (action: SyncAction) => void;
    onRemove: () => void;
    onOpenFile: () => void;
  } = $props();

  let tab = $state<DetailTab>("general");
  let content = $state("");
  let contentError = $state("");
  let contentLoading = $state(false);
  let loadedKey = $state("");
  let renaming = $state(false);
  let nameDraft = $state("");
  let renameError = $state("");

  const shortHash = $derived(instruction.hash.slice(0, 10));
  const sourceLabel = $derived(instructionSourceLabel(instruction.source));
  const sourceUrl = $derived(instructionSourceUrl(instruction.source));
  /** Local sources open in the file manager, GitHub ones in the browser. */
  const sourceHref = $derived.by(() => {
    if (sourceUrl) return sourceUrl;
    const source = instruction.source;
    return source.type === "file" || source.type === "agent" ? source.path : null;
  });
  const formatDate = (value: string) => {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
  };

  function openSource() {
    if (sourceUrl) void openExternal(sourceUrl);
    else if (sourceHref) openInFileManager(sourceHref).catch(console.error);
  }

  // Read the file when the content tab is shown, again whenever it changed on disk.
  $effect(() => {
    if (tab !== "content") return;
    const key = `${instruction.id}|${instruction.hubHash ?? instruction.hash}`;
    if (key === untrack(() => loadedKey)) return;
    loadedKey = key;
    contentLoading = true;
    contentError = "";
    readInstruction(instruction.id)
      .then((text) => (content = text))
      .catch((error) => (contentError = String(error)))
      .finally(() => (contentLoading = false));
  });

  /** Save the library file; unmodified copies follow, edited ones are listed. */
  async function save(text: string): Promise<string[]> {
    const result = await writeInstruction(instruction.id, text);
    const view = result.instruction;
    if (view) {
      // Keep the editor's text instead of re-reading the file it was just written to.
      loadedKey = `${view.id}|${view.hubHash ?? view.hash}`;
      content = text === "" || text.endsWith("\n") ? text : `${text}\n`;
      applyInstructionView(view);
    }
    await refreshInstructions();
    return result.blockers;
  }

  function startRename() {
    nameDraft = instruction.name;
    renameError = "";
    renaming = true;
  }

  async function commitRename() {
    const name = nameDraft.trim();
    if (!name || name === instruction.name) {
      renaming = false;
      return;
    }
    try {
      applyInstructionView(await renameInstruction(instruction.id, name));
      renaming = false;
    } catch (error) {
      renameError = String(error);
    }
  }

  function handleNameKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") void commitRename();
    if (event.key === "Escape") renaming = false;
  }
</script>

<div class="flex min-h-0 min-w-0 flex-col">
  <header
    class="border-base-300 flex h-12 flex-none items-center justify-between gap-3 border-b px-6"
    data-window-drag-region
  >
    <div class="flex min-w-0 flex-1 items-center gap-2">
      {#if renaming}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="border-base-300 bg-base-200 text-base-content focus:border-primary h-8 min-w-0 flex-1 rounded-lg border px-2 text-sm focus:outline-none"
          bind:value={nameDraft}
          onkeydown={handleNameKeydown}
          aria-label={$t("instructions.rename")}
          autofocus
        />
        <IconButton
          variant="outline"
          onclick={commitRename}
          title={$t("common.confirm")}
          class="h-8 w-8 p-0"
        >
          <Check size={14} />
        </IconButton>
        <IconButton
          variant="outline"
          onclick={() => (renaming = false)}
          title={$t("common.cancel")}
          class="h-8 w-8 p-0"
        >
          <X size={14} />
        </IconButton>
      {:else}
        <h2 class="text-base-content truncate text-[1.05rem] font-semibold tracking-[-0.02em]">
          {instruction.name}
        </h2>
        <button
          class="text-base-content-subtle hover:text-base-content shrink-0 transition"
          type="button"
          onclick={startRename}
          title={$t("instructions.rename")}
          aria-label={$t("instructions.rename")}
        >
          <Pencil size={13} />
        </button>
        {#if instruction.hasDrift}
          <span class="tag tag-warning">{$t("library.tag.changed")}</span>
        {/if}
      {/if}
    </div>
    <div class="flex shrink-0 items-center gap-1.5">
      <IconButton
        variant="outline"
        onclick={onOpenFile}
        title={$t("instructions.openFile")}
        class="h-8 w-8 p-0"
      >
        <FolderOpen size={15} />
      </IconButton>
      <IconButton
        variant="outline"
        onclick={onRemove}
        title={$t("detail.remove")}
        class="text-error h-8 w-8 p-0"
        disabled={busy}
      >
        <Trash2 size={15} />
      </IconButton>
      <PrimaryActionButton
        onclick={() => onInstall("user", null)}
        className="h-8 px-3 py-0 text-[13px]"
        disabled={busy}
      >
        {$t("detail.install")}
      </PrimaryActionButton>
    </div>
  </header>

  <div class="border-base-300 flex flex-none items-center border-b px-6 py-2">
    <SegmentedTabs
      items={[
        { value: "general", label: $t("instructions.tab.general") },
        { value: "content", label: $t("instructions.tab.content") },
      ]}
      value={tab}
      onChange={(value) => (tab = value as DetailTab)}
      className="text-xs"
    />
  </div>

  {#if tab === "general"}
    <div class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto max-w-4xl space-y-6 px-6 py-5">
        <dl
          class="text-base-content-muted grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-xs"
        >
          <dt>{$t("detail.source")}</dt>
          <dd class="flex min-w-0 items-center gap-2">
            <span class="tag tag-neutral shrink-0">
              {$t(`instructions.source.${instruction.source.type}`)}
            </span>
            {#if sourceHref}
              <button
                class="text-primary inline-flex min-w-0 items-center gap-1 truncate hover:underline"
                type="button"
                onclick={openSource}
                title={sourceHref}
              >
                <span class="truncate font-mono">{sourceLabel}</span>
                {#if sourceUrl}
                  <ExternalLink size={11} class="shrink-0" />
                {/if}
              </button>
            {/if}
          </dd>
          <dt>{$t("instructions.file")}</dt>
          <dd class="truncate font-mono" title={instruction.hubPath}>{instruction.hubPath}</dd>
          <dt>{$t("detail.hash")}</dt>
          <dd class="font-mono" title={instruction.hash}>
            {shortHash}
            {#if instruction.hubHash && instruction.hubHash !== instruction.hash}
              <span class="text-warning-content"> → {instruction.hubHash.slice(0, 10)}</span>
            {/if}
          </dd>
          <dt>{$t("detail.importedAt")}</dt>
          <dd>{formatDate(instruction.importedAt)}</dd>
          <dt>{$t("detail.updatedAt")}</dt>
          <dd>{formatDate(instruction.updatedAt)}</dd>
          <ItemScopeField itemKey={`instruction:${instruction.id}`} />
        </dl>

        {#if renameError}
          <p class="text-error text-sm whitespace-pre-wrap">{renameError}</p>
        {/if}
        {#if actionError}
          <p class="text-error text-sm whitespace-pre-wrap">{actionError}</p>
        {/if}

        {#if instruction.hasDrift}
          <section class="space-y-2">
            <h3 class="text-base-content text-sm font-medium">{$t("detail.drift")}</h3>
            <DriftPanel
              id={instruction.id}
              kind="instruction"
              hubState={instruction.hubState}
              installs={instruction.installs}
              {busy}
              {onSync}
              {onTargetAction}
            />
          </section>
        {/if}

        <section class="space-y-2">
          <div class="flex items-center justify-between gap-3">
            <h3 class="text-base-content text-sm font-medium">{$t("detail.installs")}</h3>
            <button
              class="border-base-300 text-base-content-muted hover:border-primary hover:text-primary flex h-7 items-center gap-1 rounded-lg border border-dashed px-2 text-[12px] transition disabled:opacity-50"
              type="button"
              onclick={onManageLocations}
              disabled={busy}
            >
              <Plus size={13} />
              <span>{$t("detail.installs.addLocation")}</span>
            </button>
          </div>
          <InstallTargets
            installs={instruction.installs}
            {projects}
            {agents}
            {homePath}
            {busy}
            onAdd={onInstall}
            onUninstall={onUninstallLocation}
          />
          {#if instruction.installs.length === 0}
            <p class="text-base-content-faint text-xs">{$t("instructions.installs.empty")}</p>
          {/if}
        </section>
      </div>
    </div>
  {:else}
    <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div class="mx-auto flex w-full max-w-4xl shrink-0 grow flex-col px-6 py-5">
        <MarkdownEditor
          {content}
          loading={contentLoading}
          error={contentError}
          disabled={busy}
          onSave={save}
        />
      </div>
    </div>
  {/if}
</div>
