<script lang="ts">
  import { Info, X } from "@lucide/svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import SkillList from "$lib/components/library/SkillList.svelte";
  import SplitView from "$lib/components/SplitView.svelte";
  import SkillDetail from "$lib/components/library/SkillDetail.svelte";
  import type { TargetAction } from "$lib/components/library/DriftPanel.svelte";
  import RemoveSkillModal from "$lib/components/RemoveSkillModal.svelte";
  import { t } from "$lib/i18n";
  import {
    buildLibraryHref,
    getAppLocation,
    type LibraryFilter,
    type LibrarySource,
  } from "$lib/navigation/app-shell";
  import {
    installSkill,
    openInFileManager,
    removeHubSkill,
    syncSkill,
    uninstallSkill,
    type HubSkillView,
    type InstallScope,
    type InstallView,
    type SkillSource,
    type SyncAction,
  } from "$lib/api";
  import {
    agentsById,
    checkSourceUpdates,
    hubError,
    hubLoading,
    hubSkills,
    migrationReport,
    refreshHub,
    refreshHubAndSources,
    skillLabel,
    sourceChecking,
  } from "$lib/stores/hub";
  import { homePath } from "$lib/stores/env";
  import {
    openInstallModal,
    openLocationPickerModal,
    openScanRootsModal,
    performAction,
  } from "$lib/stores/modals";
  import { userProjects } from "$lib/stores/user-projects";
  import { baseName } from "$lib/scopes";

  let search = $state("");
  let busy = $state(false);
  let actionError = $state("");
  let removeModalOpen = $state(false);
  let migrationDismissed = $state(false);
  /** Skills picked with Shift+click for a batch install or removal. */
  let checkedIds = $state<string[]>([]);
  /** Skills the remove modal acts on: the selected one or the checked batch. */
  let removeIds = $state<string[]>([]);

  const location = $derived(getAppLocation(page.url));
  const selectedId = $derived(location.skill);
  const filter = $derived(location.filter);
  const source = $derived(location.source);

  const filteredSkills = $derived.by(() => {
    const needle = search.trim().toLowerCase();
    return $hubSkills.filter((skill) => {
      if (needle) {
        const haystack = `${skill.name} ${skill.description ?? ""}`.toLowerCase();
        if (!haystack.includes(needle)) return false;
      }
      if (source !== "all" && sourceKind(skill.source) !== source) return false;
      switch (filter) {
        case "changed":
          return skill.hasDrift;
        case "uninstalled":
          return skill.installs.length === 0;
        default:
          return true;
      }
    });
  });

  const selectedSkill = $derived<HubSkillView | null>(
    selectedId ? ($hubSkills.find((skill) => skill.id === selectedId) ?? null) : null
  );

  const removeInstallCount = $derived(
    $hubSkills
      .filter((skill) => removeIds.includes(skill.id))
      .reduce((total, skill) => total + skill.installs.length, 0)
  );

  const migrationNotice = $derived.by(() => {
    const report = $migrationReport;
    if (!report || migrationDismissed) return null;
    if (report.errors.length > 0) {
      return { kind: "errors" as const, count: report.errors.length, details: report.errors };
    }
    if (report.imported.length > 0 && isRecent(report.completedAt)) {
      return { kind: "imported" as const, count: report.imported.length, details: [] };
    }
    return null;
  });

  /** Folder and archive imports both come from this machine. */
  function sourceKind(value: SkillSource): LibrarySource {
    if (value.type === "github") return "github";
    return value.type === "none" ? "none" : "local";
  }

  function isRecent(value: string | null | undefined): boolean {
    if (!value) return false;
    const time = new Date(value).getTime();
    return Number.isFinite(time) && Date.now() - time < 24 * 60 * 60 * 1000;
  }

  function navigate(options: {
    skill?: string | null;
    filter?: LibraryFilter;
    source?: LibrarySource;
  }) {
    const href = buildLibraryHref({
      skill: options.skill === undefined ? selectedId : options.skill,
      filter: options.filter ?? filter,
      source: options.source ?? source,
    });
    goto(href, { replaceState: true, keepFocus: true, noScroll: true });
  }

  function handleSelect(id: string) {
    actionError = "";
    navigate({ skill: id });
  }

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

  function handleToggleChecked(id: string) {
    checkedIds = checkedIds.includes(id)
      ? checkedIds.filter((item) => item !== id)
      : [...checkedIds, id];
  }

  function handleBatchInstall() {
    const ids = checkedIds.filter((id) => $hubSkills.some((skill) => skill.id === id));
    checkedIds = [];
    openInstallModal(ids, { replace: true });
  }

  function openRemoveModal(ids: string[]) {
    if (ids.length === 0) return;
    removeIds = ids;
    removeModalOpen = true;
  }

  function handleBatchRemove() {
    openRemoveModal(checkedIds.filter((id) => $hubSkills.some((skill) => skill.id === id)));
  }

  function handleInstall(scope: InstallScope, projectPath: string | null, lockScope = false) {
    if (!selectedSkill) return;
    openInstallModal([selectedSkill.id], { scope, projectPath, lockScope });
  }

  /** Uninstall from every agent of one location, after confirming. */
  function handleUninstallLocation(
    scope: InstallScope,
    projectPath: string | null,
    installs: InstallView[]
  ) {
    const skill = selectedSkill;
    if (!skill || installs.length === 0) return;
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
    const skill = selectedSkill;
    if (!skill) return;
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
    const skill = selectedSkill;
    if (!skill) return;
    const name = skill.id;
    void runAction(async () => {
      const result = await performAction((force) => syncSkill(name, withForce(action, force)));
      if (!result.applied && result.blockers.length > 0) {
        actionError = result.blockers.join("\n");
      }
    });
  }

  function handleOpenDir() {
    if (!selectedSkill) return;
    openInFileManager(selectedSkill.hubPath).catch((error) => (actionError = String(error)));
  }

  function handleRefresh() {
    refreshHubAndSources().catch(console.error);
  }

  function handleCheckSource() {
    if (!selectedSkill) return;
    void checkSourceUpdates([selectedSkill.id], { manual: true }).then((updates) => {
      const failed = updates.find((update) => update.error);
      if (failed?.error) actionError = failed.error;
    });
  }

  /** Remove every target; on failure keep only the failed ones so a retry skips the rest. */
  async function handleRemove(removeInstalls: boolean) {
    const failures: { id: string; error: unknown }[] = [];
    for (const id of removeIds) {
      try {
        await removeHubSkill(id, removeInstalls);
      } catch (error) {
        failures.push({ id, error });
      }
    }
    if (failures.length === 0) return;
    const removed = removeIds.filter((id) => !failures.some((item) => item.id === id));
    if (removed.length > 0) {
      dropRemoved(removed);
      await refreshHub().catch(console.error);
    }
    removeIds = failures.map((item) => item.id);
    throw failures.length === 1 && removed.length === 0
      ? failures[0].error
      : failures.map((item) => `${$skillLabel(item.id)}: ${String(item.error)}`).join("\n");
  }

  function dropRemoved(removed: string[]) {
    checkedIds = checkedIds.filter((id) => !removed.includes(id));
    if (selectedId && removed.includes(selectedId)) navigate({ skill: null });
  }

  async function handleRemoved() {
    dropRemoved(removeIds);
    await refreshHub();
  }
</script>

<section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden">
  {#if migrationNotice}
    <div
      class="border-base-300 bg-base-200 text-base-content flex items-start gap-2 border-b px-4 py-2 text-xs"
    >
      <Info size={14} class="text-primary mt-0.5 shrink-0" />
      <div class="min-w-0 flex-1">
        {#if migrationNotice.kind === "errors"}
          <p>{$t("migration.errors", { count: migrationNotice.count })}</p>
          <ul class="text-base-content-muted mt-1 max-h-24 overflow-y-auto">
            {#each migrationNotice.details as detail}
              <li class="truncate" title={detail}>{detail}</li>
            {/each}
          </ul>
        {:else}
          <p>{$t("migration.notice", { count: migrationNotice.count })}</p>
        {/if}
      </div>
      <button
        class="text-base-content-muted hover:text-base-content shrink-0"
        type="button"
        onclick={() => (migrationDismissed = true)}
        title={$t("migration.dismiss")}
      >
        <X size={14} />
      </button>
    </div>
  {/if}

  <SplitView>
    <SkillList
      skills={filteredSkills}
      totalCount={$hubSkills.length}
      {selectedId}
      {checkedIds}
      loading={$hubLoading}
      checking={$sourceChecking}
      error={$hubError}
      bind:search
      {filter}
      {source}
      onSelect={handleSelect}
      onToggleChecked={handleToggleChecked}
      onBatchInstall={handleBatchInstall}
      onBatchRemove={handleBatchRemove}
      onClearChecked={() => (checkedIds = [])}
      onRefresh={handleRefresh}
      onScan={() => openScanRootsModal()}
      onFilterChange={(next) => navigate({ filter: next })}
      onSourceChange={(next) => navigate({ source: next })}
    />

    <div class="surface-panel grid min-h-0 min-w-0 grid-rows-[minmax(0,1fr)]">
      {#if selectedSkill}
        {#key selectedSkill.id}
          <SkillDetail
            skill={selectedSkill}
            projects={$userProjects}
            agents={$agentsById}
            homePath={$homePath}
            {busy}
            checkingSource={$sourceChecking}
            {actionError}
            onInstall={handleInstall}
            onUninstallLocation={handleUninstallLocation}
            onManageLocations={() => openLocationPickerModal(selectedSkill.id)}
            onTargetAction={handleTargetAction}
            onSync={handleSync}
            onRemove={() => openRemoveModal([selectedSkill.id])}
            onOpenDir={handleOpenDir}
            onCheckSource={handleCheckSource}
          />
        {/key}
      {:else}
        <div class="flex min-h-0 flex-col">
          <header class="h-12 flex-none" data-window-drag-region></header>
          <div
            class="text-base-content-muted flex flex-1 items-center justify-center p-8 text-center text-sm"
          >
            {$t("library.selectHint")}
          </div>
        </div>
      {/if}
    </div>
  </SplitView>
</section>

<RemoveSkillModal
  bind:open={removeModalOpen}
  name={removeIds.map((id) => $skillLabel(id)).join(", ")}
  title={removeIds.length > 1 ? $t("remove.batchTitle", { count: removeIds.length }) : ""}
  description={removeIds.length > 1
    ? $t("remove.batchDescription", { count: removeIds.length })
    : ""}
  installCount={removeInstallCount}
  remove={handleRemove}
  onRemoved={handleRemoved}
/>
