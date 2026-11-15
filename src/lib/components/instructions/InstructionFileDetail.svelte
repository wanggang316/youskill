<script lang="ts">
  import { untrack } from "svelte";
  import { ChevronLeft, FolderOpen, LayoutTemplate } from "@lucide/svelte";
  import AgentBadge from "$lib/components/AgentBadge.svelte";
  import ReferenceMarker from "$lib/components/ReferenceMarker.svelte";
  import SymlinkMarker from "$lib/components/SymlinkMarker.svelte";
  import MarkdownEditor from "$lib/components/MarkdownEditor.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import {
    readInstructionFile,
    writeInstructionFile,
    type AgentFileView,
  } from "$lib/api/instructions";
  import type { AgentInfo } from "$lib/api/skills";
  import { refreshInstructions } from "$lib/stores/instructions";
  import { baseName } from "$lib/scopes";

  let {
    file,
    agents,
    busy = false,
    actionError = "",
    onOpenTemplate,
    onPromote,
    onOpenFile,
    onBack,
  }: {
    file: AgentFileView;
    agents: Map<string, AgentInfo>;
    busy?: boolean;
    actionError?: string;
    onOpenTemplate: (id: string) => void;
    /** Import this file as a new template that it then belongs to. */
    onPromote: () => void;
    onOpenFile: () => void;
    /** Shown in an in-place detail: return to the page's own view. */
    onBack?: () => void;
  } = $props();

  let content = $state("");
  let contentError = $state("");
  let contentLoading = $state(false);
  let loadedKey = $state("");

  const location = $derived(
    file.scope === "user" ? $t("scope.user") : baseName(file.projectPath ?? "")
  );

  // Read the file on show, again whenever it changed on disk.
  $effect(() => {
    const key = `${file.path}|${file.hash ?? ""}`;
    if (key === untrack(() => loadedKey)) return;
    loadedKey = key;
    contentLoading = true;
    contentError = "";
    readInstructionFile(file.path)
      .then((text) => (content = text))
      .catch((error) => (contentError = String(error)))
      .finally(() => (contentLoading = false));
  });

  async function save(text: string) {
    await writeInstructionFile(file.path, text);
    content = text === "" || text.endsWith("\n") ? text : `${text}\n`;
    await refreshInstructions();
    // The refreshed hash matches what was just written; keep the editor's text.
    loadedKey = `${file.path}|${file.hash ?? ""}`;
  }
</script>

<div class="flex min-h-0 min-w-0 flex-col">
  <header
    class="border-base-300 flex h-12 flex-none items-center justify-between gap-3 border-b px-6"
    data-window-drag-region
  >
    <div class="flex min-w-0 items-center gap-2">
      {#if onBack}
        <IconButton
          variant="outline"
          onclick={onBack}
          title={$t("header.back")}
          class="h-8 w-8 shrink-0 p-0"
        >
          <ChevronLeft size={16} />
        </IconButton>
      {/if}
      <h2 class="text-base-content truncate font-mono text-[1rem] font-semibold tracking-[-0.02em]">
        {file.fileName}
      </h2>
      <SymlinkMarker target={file.linkTarget} size={13} />
      <ReferenceMarker references={file.references} size={13} />
      <span class="text-base-content-muted truncate text-xs">{location}</span>
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
      {#if !file.template}
        <PrimaryActionButton
          onclick={onPromote}
          className="h-8 px-3 py-0 text-[13px]"
          disabled={busy}
        >
          {$t("instructions.file.promote")}
        </PrimaryActionButton>
      {/if}
    </div>
  </header>

  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto">
    <div class="mx-auto flex w-full max-w-4xl shrink-0 grow flex-col gap-5 px-6 py-5">
      <dl
        class="text-base-content-muted grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-1.5 text-xs"
      >
        <dt>{$t("instructions.file.path")}</dt>
        <dd class="truncate font-mono" title={file.path}>{file.path}</dd>
        {#if file.linkTarget}
          <dt>{$t("symlink.label")}</dt>
          <dd class="truncate font-mono" title={file.linkTarget}>{file.linkTarget}</dd>
        {/if}
        {#if file.references?.length}
          <dt>{$t("reference.label")}</dt>
          <dd class="min-w-0 font-mono">
            {#each file.references as reference (reference)}
              <span class="block truncate" title={reference}>{reference}</span>
            {/each}
          </dd>
        {/if}
        <dt>{$t("instructions.file.agents")}</dt>
        <dd class="flex items-center">
          <AgentBadge agentIds={file.agentIds} {agents} path={file.path} />
        </dd>
        <dt>{$t("instructions.file.template")}</dt>
        <dd class="flex min-w-0 items-center gap-2">
          {#if file.template}
            <button
              class="text-primary inline-flex min-w-0 items-center gap-1 truncate hover:underline"
              type="button"
              onclick={() => onOpenTemplate(file.template?.id ?? "")}
            >
              <LayoutTemplate size={12} class="shrink-0" />
              <span class="truncate">{file.template.name}</span>
            </button>
            <span class={`state-dot state-${file.template.state} shrink-0`}></span>
            <span class="text-base-content shrink-0">
              {$t(`target.state.${file.template.state}`)}
            </span>
          {:else}
            <span>{$t("instructions.file.noTemplate")}</span>
          {/if}
        </dd>
      </dl>

      {#if actionError}
        <p class="text-error text-sm whitespace-pre-wrap">{actionError}</p>
      {/if}

      <MarkdownEditor
        {content}
        loading={contentLoading}
        error={contentError}
        disabled={busy}
        onSave={save}
      />
    </div>
  </div>
</div>
