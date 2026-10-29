<script lang="ts">
  import { onMount } from "svelte";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { ExternalLink, Loader2, RefreshCw } from "@lucide/svelte";
  import {
    applyLibraryPending,
    applyLibraryRestore,
    backupLibrary,
    connectLibrarySync,
    disconnectLibrarySync,
    getGithubAuthStatus,
    getLibrarySyncStatus,
    listLibraryHistory,
    listLibraryPending,
    planLibraryRestore,
    undoLibraryRestore,
    type FirstConnectMode,
    type GithubAuthStatus,
    type LibraryChange,
    type PendingAction,
    type PendingItem,
    type RemoteCommit,
    type RestoreDecisions,
    type RestoreOptions,
    type RestoreOutcome,
    type RestorePlan,
    type SyncStatus,
  } from "$lib/api";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import { buildSettingsHref } from "$lib/navigation/app-shell";
  import { agentsById, loadAgents, refreshHub } from "$lib/stores/hub";
  import { refreshInstructions } from "$lib/stores/instructions";
  import LibraryRestorePanel from "./LibraryRestorePanel.svelte";
  import SettingsPane from "./SettingsPane.svelte";
  import SettingsRow from "./SettingsRow.svelte";

  type Busy =
    | "load"
    | "connect"
    | "backup"
    | "check"
    | "restore"
    | "apply"
    | "history"
    | "undo"
    | "disconnect";
  type Notice = { tone: "success" | "info" | "error"; text: string; details?: string[] };

  const DEFAULT_REPO = "youskill-library";
  const ACTION_ORDER: PendingAction[] = ["install", "update", "uninstall"];

  let status = $state<SyncStatus | null>(null);
  let github = $state<GithubAuthStatus | null>(null);
  /** The restore preview on screen, or the last check result when it has nothing to show. */
  let plan = $state<RestorePlan | null>(null);
  let showPlan = $state(false);
  let history = $state<RemoteCommit[] | null>(null);
  let confirmUndo = $state(false);
  let pending = $state<PendingItem[]>([]);
  let selected = $state<string[]>([]);
  let repoInput = $state(DEFAULT_REPO);
  let busy = $state<Busy | null>(null);
  let notice = $state<Notice | null>(null);
  let confirmDisconnect = $state(false);

  const githubReady = $derived(Boolean(github?.gh_installed && github?.selected_login));
  const pendingGroups = $derived(
    ACTION_ORDER.map((action) => ({
      action,
      items: pending.filter((item) => item.action === action),
    })).filter((group) => group.items.length > 0)
  );

  onMount(() => {
    load();
  });

  const itemKey = (item: PendingItem) =>
    `${item.kind}:${item.id}:${item.action}:${item.agentId}:${item.path ?? ""}`;

  const formatTime = (value?: string) => {
    if (!value) return "";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
  };

  const agentName = (id: string) => $agentsById.get(id)?.display_name ?? id;

  function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  async function load() {
    busy = "load";
    try {
      const [syncStatus, auth] = await Promise.all([getLibrarySyncStatus(), getGithubAuthStatus()]);
      status = syncStatus;
      github = auth;
      if (syncStatus.connected && syncStatus.hasSynced) await loadPending();
    } catch (error) {
      notice = { tone: "error", text: errorText(error) };
    } finally {
      busy = null;
    }
  }

  async function loadPending() {
    pending = await listLibraryPending();
    selected = pending.filter((item) => item.action !== "uninstall").map(itemKey);
  }

  async function reloadLibrary(agentsChanged = false) {
    if (agentsChanged) await loadAgents();
    await Promise.all([refreshHub(), refreshInstructions()]);
  }

  function changeCount(changes: LibraryChange[]): number {
    return changes.length;
  }

  async function run(kind: Busy, action: () => Promise<void>) {
    if (busy) return;
    busy = kind;
    notice = null;
    try {
      await action();
    } catch (error) {
      notice = { tone: "error", text: errorText(error) };
    } finally {
      busy = null;
    }
  }

  const connect = () =>
    run("connect", async () => {
      status = await connectLibrarySync(repoInput);
      plan = await planLibraryRestore("pull");
      if (plan.state === "choose_mode") {
        showPlan = true;
        notice = { tone: "info", text: $t("settings.sync.connectedChooseHint") };
      } else if (plan.state !== "remote_empty" && status.libraryEmpty) {
        notice = { tone: "info", text: $t("settings.sync.connectedRestoreHint") };
      } else {
        notice = { tone: "success", text: $t("settings.sync.connected") };
      }
    });

  const disconnect = () =>
    run("disconnect", async () => {
      await disconnectLibrarySync();
      confirmDisconnect = false;
      plan = null;
      showPlan = false;
      history = null;
      pending = [];
      status = await getLibrarySyncStatus();
    });

  const backup = () =>
    run("backup", async () => {
      const outcome = await backupLibrary();
      const count = changeCount(outcome.changes);
      notice =
        outcome.result === "pushed"
          ? { tone: "success", text: $t("settings.sync.backupPushed", { count }) }
          : outcome.result === "up_to_date"
            ? { tone: "info", text: $t("settings.sync.backupUpToDate") }
            : outcome.result === "remote_ahead"
              ? { tone: "info", text: $t("settings.sync.backupRemoteAhead") }
              : outcome.result === "diverged"
                ? { tone: "error", text: $t("settings.sync.diverged") }
                : { tone: "error", text: $t("settings.sync.backupRemoteNotEmpty") };
      status = await getLibrarySyncStatus();
      if (outcome.result === "pushed") {
        plan = null;
        showPlan = false;
      } else if (outcome.result !== "up_to_date") {
        await preview(outcome.result === "remote_not_empty" ? "merge" : "pull");
      }
    });

  /** Plan a restore and show it, or report that there is nothing to restore. */
  async function preview(mode: RestorePlan["mode"], target: string | null = null) {
    plan = await planLibraryRestore(mode, target);
    showPlan = plan.state === "ready" || plan.state === "choose_mode";
    if (plan.state === "up_to_date") {
      notice = { tone: "info", text: $t("settings.sync.pullUpToDate") };
    } else if (plan.state === "remote_empty") {
      notice = { tone: "info", text: $t("settings.sync.remoteEmpty") };
    }
  }

  /** A plan the user has nothing to decide on. */
  function isSimple(current: RestorePlan): boolean {
    return (
      current.state === "ready" &&
      current.target === null &&
      current.items.every((item) => item.resolution !== "conflict") &&
      current.items.every((item) => item.resolution !== "removal_candidate") &&
      current.items.every((item) => !item.deferred)
    );
  }

  const check = () => run("check", () => preview("pull"));

  const restore = () =>
    run("restore", async () => {
      await preview("pull");
      if (plan && isSimple(plan)) {
        await applyPlan(
          plan,
          { conflicts: {}, removals: {}, include: {} },
          { applyInstalls: false }
        );
      }
    });

  const chooseMode = (mode: FirstConnectMode) =>
    run("check", () => preview(mode, plan?.target ?? null));

  const applyRestore = (decisions: RestoreDecisions, options: RestoreOptions) =>
    run("restore", async () => {
      if (plan) await applyPlan(plan, decisions, options);
    });

  async function applyPlan(
    current: RestorePlan,
    decisions: RestoreDecisions,
    options: RestoreOptions
  ) {
    const outcome = await applyLibraryRestore(current, decisions, options);
    if (outcome.result === "stale") {
      notice = {
        tone: "error",
        text: outcome.localApplied
          ? $t("settings.sync.restorePushRejected")
          : $t("settings.sync.restoreStale"),
      };
      if (!outcome.localApplied) await preview(current.mode, current.target);
    } else {
      notice = restoreNotice(outcome);
      plan = null;
      showPlan = false;
      history = null;
    }
    if (outcome.localApplied) {
      await reloadLibrary(outcome.agentsChanged);
      await loadPending();
    }
    status = await getLibrarySyncStatus();
  }

  function restoreNotice(outcome: RestoreOutcome): Notice {
    const details = [...outcome.errors, ...(outcome.installs?.failures ?? [])];
    const parts = [
      outcome.pushed ? $t("settings.sync.restoredPushed") : $t("settings.sync.restored"),
    ];
    if (outcome.installs) {
      parts.push($t("settings.sync.pendingApplied", { count: outcome.installs.applied }));
    }
    return { tone: details.length ? "error" : "success", text: parts.join(" "), details };
  }

  const toggleHistory = () =>
    run("history", async () => {
      history = history ? null : await listLibraryHistory();
    });

  const previewCommit = (commit: RemoteCommit) => run("check", () => preview("pull", commit.id));

  const undo = () =>
    run("undo", async () => {
      await undoLibraryRestore();
      confirmUndo = false;
      plan = null;
      showPlan = false;
      notice = { tone: "success", text: $t("settings.sync.undone") };
      await reloadLibrary(true);
      await loadPending();
      status = await getLibrarySyncStatus();
    });

  const shortId = (id: string) => id.slice(0, 7);

  const applyPending = () =>
    run("apply", async () => {
      const items = pending.filter((item) => selected.includes(itemKey(item)));
      if (items.length === 0) return;
      const outcome = await applyLibraryPending(items);
      notice = outcome.failures.length
        ? {
            tone: "error",
            text: $t("settings.sync.pendingPartly", { count: outcome.applied }),
            details: outcome.failures,
          }
        : { tone: "success", text: $t("settings.sync.pendingApplied", { count: outcome.applied }) };
      await reloadLibrary();
      await loadPending();
      status = await getLibrarySyncStatus();
    });

  function toggle(item: PendingItem) {
    const key = itemKey(item);
    selected = selected.includes(key)
      ? selected.filter((value) => value !== key)
      : [...selected, key];
  }

  function changeLabel(change: LibraryChange): string {
    const name =
      change.kind === "agents" ? $t("settings.sync.customAgents") : change.name || change.id;
    return `${$t(`settings.sync.change.${change.change}`)} · ${name}`;
  }

  function localSummary(current: SyncStatus): string {
    if (current.changes.length > 0) {
      return $t("settings.sync.localChanges", { count: current.changes.length });
    }
    if (!current.hasSynced) return $t("settings.sync.notBackedUp");
    return current.lastBackupAt
      ? $t("settings.sync.allBackedUpAt", { time: formatTime(current.lastBackupAt) })
      : $t("settings.sync.allBackedUp");
  }

  function remoteSummary(current: SyncStatus): string {
    if (plan) {
      if (plan.state === "remote_empty") return $t("settings.sync.remoteEmpty");
      if (plan.state === "choose_mode") return $t("settings.sync.remoteHasLibrary");
      const incoming = plan.items.filter((item) => item.resolution !== "keep_local").length;
      return incoming > 0
        ? $t("settings.sync.remoteAhead", { count: incoming })
        : $t("settings.sync.remoteUpToDate");
    }
    return current.lastPullAt
      ? $t("settings.sync.lastPull", { time: formatTime(current.lastPullAt) })
      : $t("settings.sync.notChecked");
  }
</script>

<SettingsPane title={$t("settings.section.backup")} description={$t("settings.sync.description")}>
  {#snippet actions()}
    <IconButton
      variant="outline"
      onclick={load}
      disabled={busy !== null}
      title={$t("local.refresh")}
      class="h-8 w-8 p-0"
    >
      <RefreshCw size={15} class={busy === "load" ? "animate-spin" : ""} />
    </IconButton>
  {/snippet}

  {#if !status || !github}
    <div class="text-base-content-muted flex items-center justify-center py-16">
      <Loader2 size={24} class="animate-spin" />
    </div>
  {:else if !status.connected}
    <div class="settings-card">
      {#if !githubReady}
        <SettingsRow
          label={$t("settings.sync.needGithub")}
          description={$t("settings.sync.needGithubHint")}
        >
          <a class="settings-button" href={buildSettingsHref("github")}>
            {$t("settings.sync.openGithubSettings")}
          </a>
        </SettingsRow>
      {:else}
        <SettingsRow
          label={$t("settings.sync.repository")}
          description={$t("settings.sync.repositoryHint", { login: github.selected_login ?? "" })}
        >
          <input
            class="border-base-300 bg-base-100 text-base-content focus:border-primary h-8 w-48 rounded-lg border px-2.5 text-[13px] outline-none"
            bind:value={repoInput}
            placeholder={DEFAULT_REPO}
            spellcheck="false"
            disabled={busy !== null}
          />
          <PrimaryActionButton
            className="h-8 rounded-lg px-3 py-0 text-[13px]"
            onclick={connect}
            loading={busy === "connect"}
            loadingText={$t("settings.sync.connecting")}
          >
            {$t("settings.sync.connect")}
          </PrimaryActionButton>
        </SettingsRow>
      {/if}
    </div>
    <p class="text-base-content-muted px-1 text-xs leading-relaxed">
      {$t("settings.sync.scopeHint")}
    </p>
  {:else}
    <div class="settings-card">
      <SettingsRow
        label={$t("settings.sync.repository")}
        description={`${status.repo ?? ""}${status.deviceName ? ` · ${status.deviceName}` : ""}`}
      >
        <IconButton
          variant="ghost"
          onclick={() => openExternal(`https://github.com/${status?.repo ?? ""}`)}
          title={$t("settings.sync.openOnGithub")}
          class="h-8 w-8 p-0"
        >
          <ExternalLink size={15} />
        </IconButton>
        {#if confirmDisconnect}
          <button
            class="settings-button text-error"
            type="button"
            onclick={disconnect}
            disabled={busy !== null}
          >
            {$t("settings.sync.confirmDisconnect")}
          </button>
        {:else}
          <button
            class="settings-button"
            type="button"
            onclick={() => (confirmDisconnect = true)}
            disabled={busy !== null}
          >
            {$t("settings.sync.disconnect")}
          </button>
        {/if}
      </SettingsRow>
      <SettingsRow label={$t("settings.sync.thisComputer")} description={localSummary(status)}>
        <PrimaryActionButton
          className="h-8 rounded-lg px-3 py-0 text-[13px]"
          onclick={backup}
          disabled={busy !== null && busy !== "backup"}
          loading={busy === "backup"}
          loadingText={$t("settings.sync.backingUp")}
        >
          {$t("settings.sync.backup")}
        </PrimaryActionButton>
      </SettingsRow>
      <SettingsRow label={$t("settings.sync.remote")} description={remoteSummary(status)}>
        <button class="settings-button" type="button" onclick={check} disabled={busy !== null}>
          {#if busy === "check"}<Loader2 size={13} class="animate-spin" />{/if}
          {$t("settings.sync.check")}
        </button>
        <button class="settings-button" type="button" onclick={restore} disabled={busy !== null}>
          {#if busy === "restore" && !showPlan}<Loader2 size={13} class="animate-spin" />{/if}
          {$t("settings.sync.pull")}
        </button>
      </SettingsRow>
      <SettingsRow
        label={$t("settings.sync.history")}
        description={status.checkpoint
          ? $t("settings.sync.checkpointAt", { time: formatTime(status.checkpoint.createdAt) })
          : $t("settings.sync.historyHint")}
      >
        {#if status.checkpoint}
          {#if confirmUndo}
            <button
              class="settings-button text-error"
              type="button"
              onclick={undo}
              disabled={busy !== null}
            >
              {#if busy === "undo"}<Loader2 size={13} class="animate-spin" />{/if}
              {$t("settings.sync.confirmUndo")}
            </button>
          {:else}
            <button
              class="settings-button"
              type="button"
              onclick={() => (confirmUndo = true)}
              disabled={busy !== null}
            >
              {$t("settings.sync.undo")}
            </button>
          {/if}
        {/if}
        <button
          class="settings-button"
          type="button"
          onclick={toggleHistory}
          disabled={busy !== null}
        >
          {#if busy === "history"}<Loader2 size={13} class="animate-spin" />{/if}
          {history ? $t("settings.sync.hideHistory") : $t("settings.sync.showHistory")}
        </button>
      </SettingsRow>
    </div>

    {#if confirmUndo}
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.sync.undoHint")}</p>
    {/if}

    {#if history}
      <div class="space-y-2">
        <h3 class="text-base-content-subtle px-1 text-[11px] font-medium tracking-wide uppercase">
          {$t("settings.sync.historyTitle")}
        </h3>
        <ul class="settings-card px-4 py-2 text-[13px]">
          {#each history as commit (commit.id)}
            <li class="flex items-center justify-between gap-4 py-1.5">
              <span class="min-w-0">
                <span class="text-base-content block truncate">{commit.message}</span>
                <span class="text-base-content-muted block text-xs">
                  {formatTime(commit.time)} · {shortId(commit.id)}
                </span>
              </span>
              <button
                class="settings-button shrink-0"
                type="button"
                onclick={() => previewCommit(commit)}
                disabled={busy !== null}
              >
                {$t("settings.sync.restoreThis")}
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if plan && showPlan}
      {#key `${plan.mode}:${plan.target ?? ""}:${plan.fingerprint}:${plan.remoteHead ?? ""}`}
        <LibraryRestorePanel
          {plan}
          busy={busy !== null}
          targetLabel={plan.target ? shortId(plan.target) : ""}
          onChooseMode={chooseMode}
          onApply={applyRestore}
          onCancel={() => (showPlan = false)}
        />
      {/key}
    {/if}

    {#if confirmDisconnect}
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.sync.disconnectHint")}</p>
    {/if}

    {#if status.libraryEmpty && !status.hasSynced}
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.sync.emptyLibraryHint")}</p>
    {/if}

    {#if status.changes.length > 0}
      <div class="space-y-2">
        <h3 class="text-base-content-subtle px-1 text-[11px] font-medium tracking-wide uppercase">
          {$t("settings.sync.localChangesTitle")}
        </h3>
        <ul class="settings-card px-4 py-2 text-[13px]">
          {#each status.changes as change (`${change.kind}:${change.id}`)}
            <li class="text-base-content py-1">{changeLabel(change)}</li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if status.skipped.length > 0}
      <p class="text-warning-content px-1 text-xs break-all">
        {$t("settings.sync.skipped", { names: status.skipped.join(", ") })}
      </p>
    {/if}

    {#if pendingGroups.length > 0}
      <div class="space-y-2">
        <div class="flex items-baseline justify-between gap-4 px-1">
          <h3 class="text-base-content-subtle text-[11px] font-medium tracking-wide uppercase">
            {$t("settings.sync.pendingTitle")}
          </h3>
          <span class="text-base-content-muted text-xs">{$t("settings.sync.pendingHint")}</span>
        </div>
        <div class="settings-card">
          {#each pendingGroups as group (group.action)}
            <div class="px-4 py-3">
              <div class="text-base-content-muted mb-1.5 text-xs">
                {$t(`settings.sync.pending.${group.action}`)}
              </div>
              {#each group.items as item (itemKey(item))}
                <label class="text-base-content flex items-center gap-2.5 py-1 text-[13px]">
                  <input
                    type="checkbox"
                    checked={selected.includes(itemKey(item))}
                    onchange={() => toggle(item)}
                    disabled={busy !== null}
                  />
                  <span class="min-w-0 truncate">
                    {item.name}
                    <span class="text-base-content-muted">→ {agentName(item.agentId)}</span>
                  </span>
                </label>
              {/each}
            </div>
          {/each}
          <div class="flex justify-end px-4 py-3">
            <PrimaryActionButton
              className="h-8 rounded-lg px-3 py-0 text-[13px]"
              onclick={applyPending}
              disabled={selected.length === 0 || (busy !== null && busy !== "apply")}
              loading={busy === "apply"}
              loadingText={$t("settings.sync.applying")}
            >
              {$t("settings.sync.applySelected", { count: selected.length })}
            </PrimaryActionButton>
          </div>
        </div>
      </div>
    {/if}
  {/if}

  {#if notice}
    <div
      class={`px-1 text-[13px] break-all ${
        notice.tone === "error"
          ? "text-error"
          : notice.tone === "success"
            ? "text-success"
            : "text-base-content-muted"
      }`}
    >
      <p>{notice.text}</p>
      {#if notice.details?.length}
        <ul class="mt-1 list-disc pl-5 text-xs">
          {#each notice.details as detail, index (index)}
            <li>{detail}</li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</SettingsPane>
