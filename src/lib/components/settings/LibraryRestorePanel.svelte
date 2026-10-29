<script lang="ts">
  import type {
    ConflictChoice,
    FirstConnectMode,
    RestoreDecisions,
    RestoreItem,
    RestoreOptions,
    RestorePlan,
  } from "$lib/api";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";

  /**
   * Preview of a restore: the first-connect choice, or the items of a plan with the
   * decisions the user makes on them.
   */
  let {
    plan,
    busy,
    targetLabel = "",
    onChooseMode,
    onApply,
    onCancel,
  }: {
    plan: RestorePlan;
    busy: boolean;
    /** The earlier commit being restored, empty for the branch head. */
    targetLabel?: string;
    onChooseMode: (mode: FirstConnectMode) => void;
    onApply: (decisions: RestoreDecisions, options: RestoreOptions) => void;
    onCancel: () => void;
  } = $props();

  const FIRST_CONNECT_MODES: FirstConnectMode[] = ["merge", "replace_local", "replace_remote"];
  const CONFLICT_CHOICES: ConflictChoice[] = ["keep_local", "take_remote", "keep_both"];

  let firstConnectMode = $state<FirstConnectMode>("merge");
  let conflicts = $state<Record<string, ConflictChoice>>({});
  let removals = $state<Record<string, boolean>>({});
  let include = $state<Record<string, boolean>>({});
  let applyInstalls = $state(true);

  const conflictItems = $derived(plan.items.filter((item) => item.resolution === "conflict"));
  const removalItems = $derived(
    plan.items.filter((item) => item.resolution === "removal_candidate")
  );
  const incomingItems = $derived(plan.items.filter((item) => item.resolution === "take_remote"));
  const outgoingItems = $derived(plan.items.filter((item) => item.resolution === "keep_local"));
  const undecided = $derived(conflictItems.filter((item) => !conflicts[item.key]).length);

  const included = (item: RestoreItem) => include[item.key] ?? !item.deferred;

  function itemName(item: RestoreItem): string {
    return item.kind === "agents" ? $t("settings.sync.customAgents") : item.name || item.id;
  }

  function canKeepBoth(item: RestoreItem): boolean {
    return (
      item.kind !== "agents" && item.localChange !== "removed" && item.remoteChange !== "removed"
    );
  }

  function sides(item: RestoreItem): string {
    return $t("settings.sync.restore.sides", {
      local: $t(`settings.sync.restore.change.${item.localChange}`),
      remote: $t(`settings.sync.restore.change.${item.remoteChange}`),
    });
  }

  function note(item: RestoreItem): string {
    const notes: string[] = [];
    if (item.matchReason === "hash") notes.push($t("settings.sync.restore.matchedHash"));
    if (item.matchReason === "source_key") notes.push($t("settings.sync.restore.matchedSource"));
    if (item.localName) notes.push($t("settings.sync.restore.localName", { name: item.localName }));
    if (item.deferred) notes.push($t("settings.sync.restore.deferred"));
    return notes.join(" · ");
  }

  function apply() {
    onApply(
      {
        conflicts,
        removals: Object.fromEntries(
          removalItems.map((item) => [item.key, removals[item.key] ? "remove" : "keep"])
        ),
        include: Object.fromEntries(incomingItems.map((item) => [item.key, included(item)])),
      },
      { applyInstalls }
    );
  }
</script>

{#if plan.state === "choose_mode"}
  <div class="space-y-2">
    <h3 class="text-base-content-subtle px-1 text-[11px] font-medium tracking-wide uppercase">
      {$t("settings.sync.restore.chooseTitle")}
    </h3>
    <div class="settings-card px-4 py-3">
      <p class="text-base-content-muted mb-2 text-xs">{$t("settings.sync.restore.chooseHint")}</p>
      {#each FIRST_CONNECT_MODES as mode (mode)}
        <label class="text-base-content flex items-start gap-2.5 py-1.5 text-[13px]">
          <input
            type="radio"
            name="first-connect-mode"
            class="mt-0.5"
            value={mode}
            bind:group={firstConnectMode}
            disabled={busy}
          />
          <span>
            {$t(`settings.sync.restore.mode.${mode}`)}
            <span class="text-base-content-muted block text-xs">
              {$t(`settings.sync.restore.mode.${mode}Hint`)}
            </span>
          </span>
        </label>
      {/each}
      <div class="flex justify-end gap-2 pt-2">
        <button class="settings-button" type="button" onclick={onCancel} disabled={busy}>
          {$t("common.cancel")}
        </button>
        <PrimaryActionButton
          className="h-8 rounded-lg px-3 py-0 text-[13px]"
          onclick={() => onChooseMode(firstConnectMode)}
          disabled={busy}
        >
          {$t("settings.sync.restore.preview")}
        </PrimaryActionButton>
      </div>
    </div>
  </div>
{:else}
  <div class="space-y-2">
    <div class="flex items-baseline justify-between gap-4 px-1">
      <h3 class="text-base-content-subtle text-[11px] font-medium tracking-wide uppercase">
        {targetLabel
          ? $t("settings.sync.restore.titleAt", { target: targetLabel })
          : $t("settings.sync.restore.title")}
      </h3>
      {#if plan.items.length === 0}
        <span class="text-base-content-muted text-xs">{$t("settings.sync.restore.nothing")}</span>
      {/if}
    </div>
    <div class="settings-card">
      {#if conflictItems.length > 0}
        <div class="px-4 py-3">
          <div class="text-base-content-muted mb-1.5 text-xs">
            {$t("settings.sync.restore.conflicts")}
          </div>
          {#each conflictItems as item (item.key)}
            <div class="flex items-center justify-between gap-4 py-1.5 text-[13px]">
              <span class="min-w-0">
                <span class="text-base-content block truncate">{itemName(item)}</span>
                <span class="text-base-content-muted block text-xs">
                  {[sides(item), note(item)].filter(Boolean).join(" · ")}
                </span>
              </span>
              <select
                class="border-base-300 bg-base-100 text-base-content h-8 shrink-0 rounded-lg border px-2 text-[13px]"
                value={conflicts[item.key] ?? ""}
                onchange={(event) =>
                  (conflicts = {
                    ...conflicts,
                    [item.key]: (event.currentTarget as HTMLSelectElement).value as ConflictChoice,
                  })}
                disabled={busy}
              >
                <option value="" disabled>{$t("settings.sync.restore.choose")}</option>
                {#each CONFLICT_CHOICES as choice (choice)}
                  {#if choice !== "keep_both" || canKeepBoth(item)}
                    <option value={choice}>{$t(`settings.sync.restore.choice.${choice}`)}</option>
                  {/if}
                {/each}
              </select>
            </div>
          {/each}
        </div>
      {/if}

      {#if removalItems.length > 0}
        <div class="px-4 py-3">
          <div class="text-base-content-muted mb-1.5 text-xs">
            {$t("settings.sync.restore.removals")}
          </div>
          {#each removalItems as item (item.key)}
            <label class="text-base-content flex items-center gap-2.5 py-1 text-[13px]">
              <input
                type="checkbox"
                checked={removals[item.key] ?? false}
                onchange={() => (removals = { ...removals, [item.key]: !removals[item.key] })}
                disabled={busy}
              />
              <span class="min-w-0 truncate">{itemName(item)}</span>
            </label>
          {/each}
        </div>
      {/if}

      {#if incomingItems.length > 0}
        <div class="px-4 py-3">
          <div class="text-base-content-muted mb-1.5 text-xs">
            {$t("settings.sync.restore.incoming")}
          </div>
          {#each incomingItems as item (item.key)}
            <label class="text-base-content flex items-center gap-2.5 py-1 text-[13px]">
              <input
                type="checkbox"
                checked={included(item)}
                onchange={() => (include = { ...include, [item.key]: !included(item) })}
                disabled={busy}
              />
              <span class="min-w-0 truncate">
                {$t(`settings.sync.restore.change.${item.remoteChange}`)} · {itemName(item)}
                {#if note(item)}
                  <span class="text-base-content-muted">· {note(item)}</span>
                {/if}
              </span>
            </label>
          {/each}
        </div>
      {/if}

      {#if outgoingItems.length > 0}
        <div class="px-4 py-3">
          <div class="text-base-content-muted mb-1.5 text-xs">
            {$t("settings.sync.restore.outgoing")}
          </div>
          {#each outgoingItems as item (item.key)}
            <div class="text-base-content py-1 text-[13px]">
              {$t(`settings.sync.restore.change.${item.localChange}`)} · {itemName(item)}
            </div>
          {/each}
        </div>
      {/if}

      <div class="flex items-center justify-between gap-4 px-4 py-3">
        <label class="text-base-content flex items-center gap-2.5 text-[13px]">
          <input type="checkbox" bind:checked={applyInstalls} disabled={busy} />
          {$t("settings.sync.restore.applyInstalls")}
        </label>
        <div class="flex shrink-0 gap-2">
          <button class="settings-button" type="button" onclick={onCancel} disabled={busy}>
            {$t("common.cancel")}
          </button>
          <PrimaryActionButton
            className="h-8 rounded-lg px-3 py-0 text-[13px]"
            onclick={apply}
            disabled={undecided > 0 || busy}
            loading={busy}
            loadingText={$t("settings.sync.restoring")}
          >
            {undecided > 0
              ? $t("settings.sync.restore.undecided", { count: undecided })
              : $t("settings.sync.restore.apply")}
          </PrimaryActionButton>
        </div>
      </div>
    </div>
  </div>
{/if}
