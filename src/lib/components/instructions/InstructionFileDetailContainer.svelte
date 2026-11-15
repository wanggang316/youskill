<script lang="ts">
  import InstructionFileDetail from "./InstructionFileDetail.svelte";
  import { t } from "$lib/i18n";
  import type { AgentFileView } from "$lib/api/instructions";
  import { openInFileManager } from "$lib/api/skills";
  import { agentsById } from "$lib/stores/hub";
  import { openImportInstructionModal } from "$lib/stores/modals";
  import { baseName } from "$lib/scopes";

  /**
   * The detail of one agent instruction file with its actions. The Instructions page and the
   * in-place detail of the Projects page both show it; the host decides where Back and
   * "open template" go.
   */
  let {
    file,
    onBack,
    onOpenTemplate,
  }: {
    file: AgentFileView;
    onBack?: () => void;
    onOpenTemplate: (id: string) => void;
  } = $props();

  let actionError = $state("");

  /** Suggested template name for a promoted file: the project (or the user) and the file. */
  function promoteName(): string {
    const where = file.scope === "user" ? $t("scope.user") : baseName(file.projectPath ?? "");
    return `${where} ${file.fileName.replace(/\.[^.]+$/, "")}`;
  }
</script>

<InstructionFileDetail
  {file}
  agents={$agentsById}
  {actionError}
  {onOpenTemplate}
  onPromote={() => openImportInstructionModal({ paths: [file.path], name: promoteName() })}
  onOpenFile={() => openInFileManager(file.path).catch((error) => (actionError = String(error)))}
  {onBack}
/>
