<script lang="ts">
  import { confirm } from "@tauri-apps/plugin-dialog";
  import InstructionDetail from "./InstructionDetail.svelte";
  import type { TargetAction } from "$lib/components/library/DriftPanel.svelte";
  import RemoveSkillModal from "$lib/components/RemoveSkillModal.svelte";
  import { t } from "$lib/i18n";
  import type { InstallScope, InstallView, SyncAction } from "$lib/api/hub";
  import {
    installInstruction,
    removeInstruction,
    syncInstruction,
    uninstallInstruction,
    type InstructionView,
  } from "$lib/api/instructions";
  import { openInFileManager } from "$lib/api/skills";
  import { homePath } from "$lib/stores/env";
  import { agentsById } from "$lib/stores/hub";
  import { refreshInstructions } from "$lib/stores/instructions";
  import {
    openInstallModal,
    openLocationPickerModal,
    performInstructionAction,
  } from "$lib/stores/modals";
  import { userProjects } from "$lib/stores/user-projects";
  import { baseName } from "$lib/scopes";

  /**
   * The detail of one instruction template with its actions. The Instructions page and the
   * in-place detail of the Projects page both show it; the host decides where Back and a
   * removal go.
   */
  let {
    instruction,
    onBack,
    onRemoved,
  }: {
    instruction: InstructionView;
    onBack?: () => void;
    /** The template left the library. */
    onRemoved: () => void;
  } = $props();

  let busy = $state(false);
  let actionError = $state("");
  let removeModalOpen = $state(false);

  async function runAction(work: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    actionError = "";
    try {
      await work();
    } catch (error) {
      actionError = String(error);
    } finally {
      busy = false;
      await refreshInstructions().catch(console.error);
    }
  }

  function handleInstall(scope: InstallScope, projectPath: string | null, lockScope = false) {
    openInstallModal([instruction.id], { scope, projectPath, lockScope, kind: "instruction" });
  }

  /** Uninstall from every agent of one location, after confirming. */
  function handleUninstallLocation(
    scope: InstallScope,
    projectPath: string | null,
    installs: InstallView[]
  ) {
    if (installs.length === 0) return;
    const { id, name } = instruction;
    const paths = installs.map((install) => install.path);
    const location =
      scope === "user"
        ? $t("scope.user")
        : ($userProjects.find((project) => project.path === projectPath)?.name ??
          baseName(projectPath ?? ""));
    void runAction(async () => {
      const confirmed = await confirm($t("locationPicker.uninstallConfirm", { name, location }), {
        title: $t("scope.uninstall"),
        kind: "warning",
      });
      if (!confirmed) return;
      const result = await performInstructionAction((force) =>
        uninstallInstruction({ name: id, targets: [], paths, force })
      );
      if (!result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  function handleTargetAction(install: InstallView, action: TargetAction) {
    const name = instruction.id;
    void runAction(async () => {
      let result;
      switch (action) {
        case "reinstall":
          if (install.mode === "symlink") {
            const targets = install.agentIds.map((agentId) => ({
              scope: install.scope,
              projectPath: install.projectPath ?? null,
              agentId,
            }));
            result = await performInstructionAction((force) =>
              installInstruction({ name, targets, mode: "symlink", force })
            );
            break;
          }
          result = await performInstructionAction((force) =>
            syncInstruction(name, { kind: "push_targets", targets: [install.path], force })
          );
          break;
        case "push":
          result = await performInstructionAction((force) =>
            syncInstruction(name, { kind: "push_targets", targets: [install.path], force })
          );
          break;
        case "adopt":
          result = await performInstructionAction((force) =>
            syncInstruction(name, { kind: "adopt_target", path: install.path, force })
          );
          break;
      }
      if (result && !result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  function withForce(action: SyncAction, force: boolean): SyncAction {
    return action.kind === "accept_hub" ? action : { ...action, force };
  }

  function handleSync(action: SyncAction) {
    const id = instruction.id;
    void runAction(async () => {
      const result = await performInstructionAction((force) =>
        syncInstruction(id, withForce(action, force))
      );
      if (!result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  async function handleRemoved() {
    await refreshInstructions();
    onRemoved();
  }
</script>

<InstructionDetail
  {instruction}
  projects={$userProjects}
  agents={$agentsById}
  homePath={$homePath}
  {busy}
  {actionError}
  onInstall={handleInstall}
  onUninstallLocation={handleUninstallLocation}
  onManageLocations={() => openLocationPickerModal(instruction.id, "instruction")}
  onTargetAction={handleTargetAction}
  onSync={handleSync}
  onRemove={() => (removeModalOpen = true)}
  onOpenFile={() =>
    openInFileManager(instruction.hubPath).catch((error) => (actionError = String(error)))}
  {onBack}
/>

<RemoveSkillModal
  bind:open={removeModalOpen}
  name={instruction.name}
  installCount={instruction.installs.length}
  kind="instruction"
  remove={(removeInstalls) => removeInstruction(instruction.id, removeInstalls)}
  onRemoved={handleRemoved}
/>
