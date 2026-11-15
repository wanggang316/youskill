<script lang="ts">
  import { confirm } from "@tauri-apps/plugin-dialog";
  import SkillDetail from "./SkillDetail.svelte";
  import type { TargetAction } from "./DriftPanel.svelte";
  import RemoveSkillModal from "$lib/components/RemoveSkillModal.svelte";
  import { t } from "$lib/i18n";
  import {
    installSkill,
    openInFileManager,
    removeHubSkill,
    syncSkill,
    uninstallSkill,
    type HubSkillView,
    type InstallScope,
    type InstallView,
    type SyncAction,
  } from "$lib/api";
  import {
    agentsById,
    checkSourceUpdates,
    refreshHub,
    skillLabel,
    sourceChecking,
  } from "$lib/stores/hub";
  import { homePath } from "$lib/stores/env";
  import { openInstallModal, openLocationPickerModal, performAction } from "$lib/stores/modals";
  import { userProjects } from "$lib/stores/user-projects";
  import { baseName } from "$lib/scopes";

  /**
   * The detail of one library skill with its actions. The Library page and the in-place
   * detail of the Projects page both show it; the host decides where Back and a removal go.
   */
  let {
    skill,
    onBack,
    onRemoved,
  }: {
    skill: HubSkillView;
    onBack?: () => void;
    /** The skill left the library. */
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
      await refreshHub().catch(console.error);
    }
  }

  function handleInstall(scope: InstallScope, projectPath: string | null, lockScope = false) {
    openInstallModal([skill.id], { scope, projectPath, lockScope });
  }

  /** Uninstall from every agent of one location, after confirming. */
  function handleUninstallLocation(
    scope: InstallScope,
    projectPath: string | null,
    installs: InstallView[]
  ) {
    if (installs.length === 0) return;
    const id = skill.id;
    const name = $skillLabel(id);
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
      const result = await performAction((force) =>
        uninstallSkill({ name: id, targets: [], paths, force })
      );
      if (!result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  function handleTargetAction(install: InstallView, action: TargetAction) {
    const name = skill.id;
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
            result = await performAction((force) =>
              installSkill({ name, targets, mode: "symlink", force })
            );
            break;
          }
          result = await performAction((force) =>
            syncSkill(name, { kind: "push_targets", targets: [install.path], force })
          );
          break;
        case "push":
          result = await performAction((force) =>
            syncSkill(name, { kind: "push_targets", targets: [install.path], force })
          );
          break;
        case "adopt":
          result = await performAction((force) =>
            syncSkill(name, { kind: "adopt_target", path: install.path, force })
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
    const name = skill.id;
    void runAction(async () => {
      const result = await performAction((force) => syncSkill(name, withForce(action, force)));
      if (!result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  function handleOpenDir() {
    openInFileManager(skill.hubPath).catch((error) => (actionError = String(error)));
  }

  function handleCheckSource() {
    void checkSourceUpdates([skill.id], { manual: true }).then((updates) => {
      const failed = updates.find((update) => update.error);
      if (failed?.error) actionError = failed.error;
    });
  }

  async function handleRemoved() {
    onRemoved();
    await refreshHub();
  }
</script>

<SkillDetail
  {skill}
  projects={$userProjects}
  agents={$agentsById}
  homePath={$homePath}
  {busy}
  checkingSource={$sourceChecking}
  {actionError}
  onInstall={handleInstall}
  onUninstallLocation={handleUninstallLocation}
  onManageLocations={() => openLocationPickerModal(skill.id)}
  onTargetAction={handleTargetAction}
  onSync={handleSync}
  onRemove={() => (removeModalOpen = true)}
  onOpenDir={handleOpenDir}
  onCheckSource={handleCheckSource}
  {onBack}
/>

<RemoveSkillModal
  bind:open={removeModalOpen}
  name={$skillLabel(skill.id)}
  installCount={skill.installs.length}
  remove={(removeInstalls) => removeHubSkill(skill.id, removeInstalls)}
  onRemoved={handleRemoved}
/>
