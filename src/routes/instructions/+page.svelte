<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import InstructionList from "$lib/components/instructions/InstructionList.svelte";
  import SplitView from "$lib/components/SplitView.svelte";
  import InstructionDetailContainer from "$lib/components/instructions/InstructionDetailContainer.svelte";
  import InstructionFileDetailContainer from "$lib/components/instructions/InstructionFileDetailContainer.svelte";
  import { t } from "$lib/i18n";
  import {
    buildInstructionsHref,
    getAppLocation,
    type LibraryFilter,
  } from "$lib/navigation/app-shell";
  import type { AgentFileView, InstructionView } from "$lib/api/instructions";
  import {
    instructionFiles,
    instructions,
    instructionsError,
    instructionsLoading,
    refreshInstructions,
  } from "$lib/stores/instructions";
  import { openImportInstructionModal } from "$lib/stores/modals";

  let search = $state("");

  const location = $derived(getAppLocation(page.url));
  const selectedId = $derived(location.instruction);
  const selectedFilePath = $derived(location.instructionFile);
  const filter = $derived(location.filter);
  const needle = $derived(search.trim().toLowerCase());

  const filteredTemplates = $derived.by(() =>
    $instructions.filter((item) => {
      if (needle) {
        const haystack = `${item.name} ${item.description ?? ""}`.toLowerCase();
        if (!haystack.includes(needle)) return false;
      }
      switch (filter) {
        case "changed":
          return item.hasDrift;
        case "uninstalled":
          return item.installs.length === 0;
        default:
          return true;
      }
    })
  );

  const filteredFiles = $derived.by(() =>
    $instructionFiles.filter((file) => {
      if (needle) {
        const haystack = `${file.fileName} ${file.path} ${file.template?.name ?? ""}`.toLowerCase();
        if (!haystack.includes(needle)) return false;
      }
      switch (filter) {
        case "changed":
          return Boolean(file.template && file.template.state !== "in_sync");
        case "uninstalled":
          return !file.template;
        default:
          return true;
      }
    })
  );

  const selected = $derived<InstructionView | null>(
    selectedId ? ($instructions.find((item) => item.id === selectedId) ?? null) : null
  );
  const selectedFile = $derived<AgentFileView | null>(
    selectedFilePath
      ? ($instructionFiles.find((file) => file.path === selectedFilePath) ?? null)
      : null
  );

  onMount(() => {
    refreshInstructions().catch(console.error);
  });

  function navigate(options: { id?: string | null; file?: string | null; filter?: LibraryFilter }) {
    const keepSelection = options.id === undefined && options.file === undefined;
    const href = buildInstructionsHref({
      id: keepSelection ? selectedId : options.id,
      file: keepSelection ? selectedFilePath : options.file,
      filter: options.filter ?? filter,
    });
    goto(href, { replaceState: true, keepFocus: true, noScroll: true });
  }

  function selectTemplate(id: string) {
    navigate({ id, file: null });
  }

  function selectFile(path: string) {
    navigate({ id: null, file: path });
  }
</script>

<section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden">
  <SplitView>
    <InstructionList
      templates={filteredTemplates}
      files={filteredFiles}
      totalTemplates={$instructions.length}
      {selectedId}
      selectedFile={selectedFilePath}
      loading={$instructionsLoading}
      error={$instructionsError}
      bind:search
      {filter}
      onSelectTemplate={selectTemplate}
      onSelectFile={selectFile}
      onRefresh={() => refreshInstructions().catch(console.error)}
      onImport={() => openImportInstructionModal()}
      onFilterChange={(next) => navigate({ filter: next })}
    />

    <div class="surface-panel grid min-h-0 min-w-0 grid-rows-[minmax(0,1fr)]">
      {#if selected}
        {#key selected.id}
          <InstructionDetailContainer
            instruction={selected}
            onRemoved={() => navigate({ id: null, file: null })}
          />
        {/key}
      {:else if selectedFile}
        {#key selectedFile.path}
          <InstructionFileDetailContainer file={selectedFile} onOpenTemplate={selectTemplate} />
        {/key}
      {:else}
        <div class="flex min-h-0 flex-col">
          <header class="h-12 flex-none" data-window-drag-region></header>
          <div
            class="text-base-content-muted flex flex-1 items-center justify-center p-8 text-center text-sm"
          >
            {$t("instructions.selectHint")}
          </div>
        </div>
      {/if}
    </div>
  </SplitView>
</section>
