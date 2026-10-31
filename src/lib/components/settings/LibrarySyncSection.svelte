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
  import Modal from "$lib/components/ui/Modal.svelte";
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
  type Dialog = "restore" | "changes" | "pending" | "history" | "undo" | "disconnect";
  type Notice = { tone: "success" | "info" | "error"; text: string; details?: string[] };

  const DEFAULT_REPO = "youskill-library";
  const ACTION_ORDER: PendingAction[] = ["install", "update", "uninstall"];
  const CHANGE_TAG: Record<LibraryChange["change"], string> = {
    added: "tag-success",
    modified: "tag-info",
    removed: "tag-error",
  };

  let status = $state<SyncStatus | null>(null);
  let github = $state<GithubAuthStatus | null>(null);
  /** The last restore preview; kept after the dialog closes to summarize the repository. */
  let plan = $state<RestorePlan | null>(null);
  let history = $state<RemoteCommit[]>([]);
  let pending = $state<PendingItem[]>([]);
  let selected = $state<string[]>([]);
  let repoInput = $state(DEFAULT_REPO);
  let busy = $state<Busy | null>(null);
  let notice = $state<Notice | null>(null);
  /** The open dialog. A notice shows in it while it is open, else under the settings. */
  let dialog = $state<Dialog | null>(null);

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

  const shortId = (id: string) => id.slice(0, 7);

  function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  function openDialog(next: Dialog) {
    notice = null;
    dialog = next;
  }

  const closeDialog = () => {
    dialog = null;
  };

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
      await preview("pull");
      if (plan?.state === "choose_mode") {
        notice = { tone: "info", text: $t("settings.sync.connectedChooseHint") };
      } else if (plan?.state !== "remote_empty" && status.libraryEmpty) {
        notice = { tone: "info", text: $t("settings.sync.connectedRestoreHint") };
      } else if (dialog === null) {
        notice = { tone: "success", text: $t("settings.sync.connected") };
      }
    });

  const disconnect = () =>
    run("disconnect", async () => {
      await disconnectLibrarySync();
      dialog = null;
      plan = null;
      history = [];
      pending = [];
      status = await getLibrarySyncStatus();
    });

  const backup = () =>
    run("backup", async () => {
      const outcome = await backupLibrary();
      const count = outcome.changes.length;
      if (dialog === "changes") dialog = null;
      status = await getLibrarySyncStatus();
      if (outcome.result === "pushed") plan = null;
      else if (outcome.result !== "up_to_date") {
        await preview(outcome.result === "remote_not_empty" ? "merge" : "pull");
      }
      notice =
        outcome.result === "pushed"
          ? { tone: "success", text: $t("settings.sync.backupPushed", { count }) }
          : outcome.result === "up_to_date"
            ? { tone: "info", text: $t("settings.sync.backupUpToDate") }
            : outcome.result === "remote_ahead"
              ? { tone: "info", text: $t("settings.sync.backupRemoteAhead") }
              : outcome.result === "diverged"
                ? { tone: "info", text: $t("settings.sync.diverged") }
                : { tone: "info", text: $t("settings.sync.backupRemoteNotEmpty") };
    });

  /** Plan a restore and open it, or report that there is nothing to restore. */
  async function preview(mode: RestorePlan["mode"], target: string | null = null) {
    plan = await planLibraryRestore(mode, target);
    if (plan.state === "ready" || plan.state === "choose_mode") {
      dialog = "restore";
      return;
    }
    if (dialog === "restore") dialog = null;
    notice = {
      tone: "info",
      text:
        plan.state === "remote_empty"
          ? $t("settings.sync.remoteEmpty")
          : $t("settings.sync.pullUpToDate"),
    };
  }

  const restore = () => run("check", () => preview("pull"));

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
    let outcome: RestoreOutcome;
    try {
      outcome = await applyLibraryRestore(current, decisions, options);
    } catch (error) {
      // A restore that stopped part-way may have written items and left a checkpoint.
      await refreshAfterFailure();
      throw error;
    }
    if (outcome.result === "stale" && !outcome.localApplied) {
      await preview(current.mode, current.target);
      notice = { tone: "error", text: $t("settings.sync.restoreStale") };
    } else if (outcome.result === "stale") {
      dialog = null;
      notice = { tone: "error", text: $t("settings.sync.restorePushRejected") };
    } else {
      dialog = null;
      plan = null;
      notice = restoreNotice(outcome);
    }
    if (outcome.localApplied) {
      await reloadLibrary(outcome.agentsChanged);
      await loadPending();
    }
    status = await getLibrarySyncStatus();
  }

  async function refreshAfterFailure() {
    await reloadLibrary(true);
    status = await getLibrarySyncStatus();
  }

  function restoreNotice(outcome: RestoreOutcome): Notice {
    const details = outcome.installs?.failures ?? [];
    const parts = [
      outcome.pushed ? $t("settings.sync.restoredPushed") : $t("settings.sync.restored"),
    ];
    if (outcome.installs) {
      parts.push($t("settings.sync.pendingApplied", { count: outcome.installs.applied }));
    }
    return { tone: details.length ? "error" : "success", text: parts.join(" "), details };
  }

  const showHistory = () =>
    run("history", async () => {
      history = await listLibraryHistory();
      dialog = "history";
    });

  const previewCommit = (commit: RemoteCommit) => run("check", () => preview("pull", commit.id));

  const undo = () =>
    run("undo", async () => {
      try {
        await undoLibraryRestore();
      } catch (error) {
        await refreshAfterFailure();
        throw error;
      }
      dialog = null;
      plan = null;
      notice = { tone: "success", text: $t("settings.sync.undone") };
      await reloadLibrary(true);
      await loadPending();
      status = await getLibrarySyncStatus();
    });

  const applyPending = () =>
    run("apply", async () => {
      const items = pending.filter((item) => selected.includes(itemKey(item)));
      if (items.length === 0) return;
      const outcome = await applyLibraryPending(items);
      await reloadLibrary();
      await loadPending();
      status = await getLibrarySyncStatus();
      if (outcome.failures.length) {
        notice = {
          tone: "error",
          text: $t("settings.sync.pendingPartly", { count: outcome.applied }),
          details: outcome.failures,
        };
      } else {
        dialog = null;
        notice = {
          tone: "success",
          text: $t("settings.sync.pendingApplied", { count: outcome.applied }),
        };
      }
    });

  function toggle(item: PendingItem) {
    const key = itemKey(item);
    selected = selected.includes(key)
      ? selected.filter((value) => value !== key)
      : [...selected, key];
  }

  function changeName(change: LibraryChange): string {
    return change.kind === "agents" ? $t("settings.sync.customAgents") : change.name || change.id;
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

  function restoreTitle(current: RestorePlan | null): string {
    if (current?.state === "choose_mode") return $t("settings.sync.restore.chooseTitle");
    return current?.target
      ? $t("settings.sync.restore.titleAt", { target: shortId(current.target) })
      : $t("settings.sync.restore.title");
  }
</script>

{#snippet noticeView(boxed: boolean)}
  {#if notice}
    <div
      class={`text-[13px] [overflow-wrap:anywhere] ${boxed ? "rounded-xl px-4 py-3" : "px-1"} ${
        notice.tone === "error"
          ? `text-error ${boxed ? "bg-error/10" : ""}`
          : notice.tone === "success"
            ? "text-success"
            : `text-base-content-muted ${boxed ? "bg-base-200" : ""}`
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
{/snippet}

{#snippet restoreMessage()}
  {#if notice?.tone !== "error"}{@render noticeView(true)}{/if}
{/snippet}

{#snippet restoreAlert()}
  {#if notice?.tone === "error"}{@render noticeView(true)}{/if}
{/snippet}

{#snippet cancelButton()}
  <button class="settings-button" type="button" onclick={closeDialog} disabled={busy !== null}>
    {$t("common.cancel")}
  </button>
{/snippet}

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
        <button
          class="settings-button"
          type="button"
          onclick={() => openDialog("disconnect")}
          disabled={busy !== null}
        >
          {$t("settings.sync.disconnect")}
        </button>
      </SettingsRow>
      <SettingsRow label={$t("settings.sync.thisComputer")} description={localSummary(status)}>
        {#if status.changes.length > 0}
          <button
            class="settings-button"
            type="button"
            onclick={() => openDialog("changes")}
            disabled={busy !== null}
          >
            {$t("settings.sync.viewChanges")}
          </button>
        {/if}
        <PrimaryActionButton
          className="h-8 rounded-lg px-3 py-0 text-[13px]"
          onclick={backup}
          disabled={busy !== null && busy !== "backup"}
          loading={busy === "backup" && dialog === null}
          loadingText={$t("settings.sync.backingUp")}
        >
          {$t("settings.sync.backup")}
        </PrimaryActionButton>
      </SettingsRow>
      <SettingsRow label={$t("settings.sync.remote")} description={remoteSummary(status)}>
        <button class="settings-button" type="button" onclick={restore} disabled={busy !== null}>
          {#if busy === "check" && dialog === null}<Loader2 size={13} class="animate-spin" />{/if}
          {$t("settings.sync.pull")}
        </button>
      </SettingsRow>
      {#if pending.length > 0}
        <SettingsRow
          label={$t("settings.sync.pendingTitle")}
          description={$t("settings.sync.pendingSummary", { count: pending.length })}
        >
          <button
            class="settings-button"
            type="button"
            onclick={() => openDialog("pending")}
            disabled={busy !== null}
          >
            {$t("settings.sync.review")}
          </button>
        </SettingsRow>
      {/if}
      <SettingsRow
        label={$t("settings.sync.history")}
        description={status.checkpoint
          ? $t("settings.sync.checkpointAt", { time: formatTime(status.checkpoint.createdAt) })
          : $t("settings.sync.historyHint")}
      >
        {#if status.checkpoint}
          <button
            class="settings-button"
            type="button"
            onclick={() => openDialog("undo")}
            disabled={busy !== null}
          >
            {$t("settings.sync.undo")}
          </button>
        {/if}
        <button
          class="settings-button"
          type="button"
          onclick={showHistory}
          disabled={busy !== null}
        >
          {#if busy === "history"}<Loader2 size={13} class="animate-spin" />{/if}
          {$t("settings.sync.showHistory")}
        </button>
      </SettingsRow>
    </div>

    {#if status.libraryEmpty && !status.hasSynced}
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.sync.emptyLibraryHint")}</p>
    {/if}

    {#if status.skipped.length > 0}
      <p class="text-warning-content px-1 text-xs break-all">
        {$t("settings.sync.skipped", { names: status.skipped.join(", ") })}
      </p>
    {/if}
  {/if}

  {#if dialog === null}
    {@render noticeView(false)}
  {/if}
</SettingsPane>

<Modal
  open={dialog === "restore" && plan !== null}
  title={restoreTitle(plan)}
  onClose={closeDialog}
  showCloseButton={busy === null}
  containerClass="max-w-2xl"
>
  {#if plan}
    {#key `${plan.mode}:${plan.target ?? ""}:${plan.fingerprint}:${plan.remoteHead ?? ""}`}
      <LibraryRestorePanel
        {plan}
        busy={busy !== null}
        message={restoreMessage}
        alert={restoreAlert}
        onChooseMode={chooseMode}
        onApply={applyRestore}
        onCancel={closeDialog}
      />
    {/key}
  {/if}
</Modal>

<Modal
  open={dialog === "changes" && status !== null}
  title={$t("settings.sync.localChangesTitle")}
  onClose={closeDialog}
  containerClass="max-w-lg"
>
  <div class="space-y-3 px-6 pt-4 pb-5">
    <p class="text-base-content-muted text-xs leading-relaxed">
      {$t("settings.sync.localChangesHint")}
    </p>
    {@render noticeView(true)}
    <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
      {#each status?.changes ?? [] as change (`${change.kind}:${change.id}`)}
        <li class="flex items-center justify-between gap-4 px-3 py-2 text-[13px]">
          <span class="text-base-content min-w-0 truncate">{changeName(change)}</span>
          <span class={`tag shrink-0 ${CHANGE_TAG[change.change]}`}>
            {$t(`settings.sync.change.${change.change}`)}
          </span>
        </li>
      {/each}
    </ul>
  </div>
  {#snippet footer()}
    <button class="settings-button" type="button" onclick={closeDialog} disabled={busy !== null}>
      {$t("common.close")}
    </button>
    <PrimaryActionButton
      className="h-8 rounded-lg px-3 py-0 text-[13px]"
      onclick={backup}
      disabled={busy !== null && busy !== "backup"}
      loading={busy === "backup"}
      loadingText={$t("settings.sync.backingUp")}
    >
      {$t("settings.sync.backup")}
    </PrimaryActionButton>
  {/snippet}
</Modal>

<Modal
  open={dialog === "pending"}
  title={$t("settings.sync.pendingTitle")}
  onClose={closeDialog}
  containerClass="max-w-lg"
>
  <div class="space-y-4 px-6 pt-4 pb-5">
    <p class="text-base-content-muted text-xs leading-relaxed">
      {$t("settings.sync.pendingHint")}
    </p>
    {@render noticeView(true)}
    {#each pendingGroups as group (group.action)}
      <section>
        <h4 class="text-base-content mb-2 text-[13px] font-medium">
          {$t(`settings.sync.pending.${group.action}`)}
        </h4>
        <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
          {#each group.items as item (itemKey(item))}
            <li class="px-3 py-2">
              <label class="text-base-content flex items-center gap-2.5 text-[13px]">
                <input
                  type="checkbox"
                  checked={selected.includes(itemKey(item))}
                  onchange={() => toggle(item)}
                  disabled={busy !== null}
                />
                <span class="min-w-0 truncate">
                  {item.name}
                  <span class="text-base-content-muted text-xs">· {agentName(item.agentId)}</span>
                </span>
              </label>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
  {#snippet footer()}
    {@render cancelButton()}
    <PrimaryActionButton
      className="h-8 rounded-lg px-3 py-0 text-[13px]"
      onclick={applyPending}
      disabled={selected.length === 0 || (busy !== null && busy !== "apply")}
      loading={busy === "apply"}
      loadingText={$t("settings.sync.applying")}
    >
      {$t("settings.sync.applySelected", { count: selected.length })}
    </PrimaryActionButton>
  {/snippet}
</Modal>

<Modal
  open={dialog === "history"}
  title={$t("settings.sync.historyTitle")}
  onClose={closeDialog}
  containerClass="max-w-lg"
>
  <div class="space-y-3 px-6 pt-4 pb-5">
    <p class="text-base-content-muted text-xs leading-relaxed">
      {$t("settings.sync.historyDialogHint")}
    </p>
    {@render noticeView(true)}
    {#if history.length === 0}
      <p class="text-base-content-muted py-4 text-center text-[13px]">
        {$t("settings.sync.historyEmpty")}
      </p>
    {:else}
      <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
        {#each history as commit (commit.id)}
          <li class="flex items-center justify-between gap-4 px-3 py-2 text-[13px]">
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
    {/if}
  </div>
</Modal>

<Modal
  open={dialog === "undo"}
  title={$t("settings.sync.undoTitle")}
  onClose={closeDialog}
  containerClass="max-w-md"
>
  <div class="space-y-3 px-6 pt-4 pb-5 text-[13px]">
    <p class="text-base-content leading-relaxed">{$t("settings.sync.undoHint")}</p>
    {#if status?.checkpoint}
      <p class="text-base-content-muted text-xs">
        {$t("settings.sync.checkpointAt", { time: formatTime(status.checkpoint.createdAt) })}
      </p>
    {/if}
    {@render noticeView(true)}
  </div>
  {#snippet footer()}
    {@render cancelButton()}
    <PrimaryActionButton
      className="h-8 rounded-lg px-3 py-0 text-[13px]"
      variant="danger"
      onclick={undo}
      disabled={busy !== null && busy !== "undo"}
      loading={busy === "undo"}
    >
      {$t("settings.sync.confirmUndo")}
    </PrimaryActionButton>
  {/snippet}
</Modal>

<Modal
  open={dialog === "disconnect"}
  title={$t("settings.sync.disconnectTitle")}
  onClose={closeDialog}
  containerClass="max-w-md"
>
  <div class="space-y-3 px-6 pt-4 pb-5 text-[13px]">
    <p class="text-base-content leading-relaxed">{$t("settings.sync.disconnectHint")}</p>
    {@render noticeView(true)}
  </div>
  {#snippet footer()}
    {@render cancelButton()}
    <PrimaryActionButton
      className="h-8 rounded-lg px-3 py-0 text-[13px]"
      variant="danger"
      onclick={disconnect}
      disabled={busy !== null && busy !== "disconnect"}
      loading={busy === "disconnect"}
    >
      {$t("settings.sync.confirmDisconnect")}
    </PrimaryActionButton>
  {/snippet}
</Modal>
