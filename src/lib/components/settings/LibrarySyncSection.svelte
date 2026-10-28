<script lang="ts">
  import { onMount } from "svelte";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { ExternalLink, Loader2, RefreshCw } from "@lucide/svelte";
  import {
    applyLibraryPending,
    backupLibrary,
    checkLibraryRemote,
    connectLibrarySync,
    disconnectLibrarySync,
    getGithubAuthStatus,
    getLibrarySyncStatus,
    listLibraryPending,
    pullLibrary,
    type GithubAuthStatus,
    type LibraryChange,
    type PendingAction,
    type PendingItem,
    type RemoteCheck,
    type SyncStatus,
  } from "$lib/api";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import { buildSettingsHref } from "$lib/navigation/app-shell";
  import { agentsById, loadAgents, refreshHub } from "$lib/stores/hub";
  import { refreshInstructions } from "$lib/stores/instructions";
  import SettingsPane from "./SettingsPane.svelte";
  import SettingsRow from "./SettingsRow.svelte";

  type Busy = "load" | "connect" | "backup" | "check" | "pull" | "apply" | "disconnect";
  type Notice = { tone: "success" | "info" | "error"; text: string; details?: string[] };

  const DEFAULT_REPO = "youskill-library";
  const ACTION_ORDER: PendingAction[] = ["install", "update", "uninstall"];

  let status = $state<SyncStatus | null>(null);
  let github = $state<GithubAuthStatus | null>(null);
  let remote = $state<RemoteCheck | null>(null);
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
      remote = await checkLibraryRemote();
      if (remote.remoteHasLibrary && status.libraryEmpty) {
        notice = { tone: "info", text: $t("settings.sync.connectedRestoreHint") };
      } else {
        notice = { tone: "success", text: $t("settings.sync.connected") };
      }
    });

  const disconnect = () =>
    run("disconnect", async () => {
      await disconnectLibrarySync();
      confirmDisconnect = false;
      remote = null;
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
      if (outcome.result === "remote_ahead") remote = await checkLibraryRemote();
      else if (outcome.result === "pushed") remote = null;
    });

  const check = () =>
    run("check", async () => {
      remote = await checkLibraryRemote();
      if (!remote.remoteHasLibrary) {
        notice = { tone: "info", text: $t("settings.sync.remoteEmpty") };
      }
    });

  const pull = () =>
    run("pull", async () => {
      const outcome = await pullLibrary();
      const count = changeCount(outcome.changes);
      notice =
        outcome.result === "pulled"
          ? {
              tone: outcome.errors.length ? "error" : "success",
              text: $t("settings.sync.pulled", { count }),
              details: outcome.errors,
            }
          : outcome.result === "up_to_date"
            ? { tone: "info", text: $t("settings.sync.pullUpToDate") }
            : outcome.result === "remote_empty"
              ? { tone: "info", text: $t("settings.sync.remoteEmpty") }
              : outcome.result === "diverged"
                ? { tone: "error", text: $t("settings.sync.diverged") }
                : { tone: "error", text: $t("settings.sync.pullLocalNotEmpty") };
      if (outcome.result === "pulled") {
        remote = null;
        await reloadLibrary(outcome.agentsChanged);
        await loadPending();
      }
      status = await getLibrarySyncStatus();
    });

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
    if (remote) {
      if (!remote.remoteHasLibrary) return $t("settings.sync.remoteEmpty");
      return remote.remoteAhead
        ? $t("settings.sync.remoteAhead", { count: remote.changes.length })
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
        <button class="settings-button" type="button" onclick={pull} disabled={busy !== null}>
          {#if busy === "pull"}<Loader2 size={13} class="animate-spin" />{/if}
          {$t("settings.sync.pull")}
        </button>
      </SettingsRow>
    </div>

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

    {#if remote?.remoteAhead && remote.changes.length > 0}
      <div class="space-y-2">
        <h3 class="text-base-content-subtle px-1 text-[11px] font-medium tracking-wide uppercase">
          {$t("settings.sync.remoteChangesTitle")}
        </h3>
        <ul class="settings-card px-4 py-2 text-[13px]">
          {#each remote.changes as change (`${change.kind}:${change.id}`)}
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
