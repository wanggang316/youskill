<script lang="ts">
  import type { Snippet } from "svelte";
  import { ArrowDownToLine, ArrowUpFromLine, CircleAlert, Trash2 } from "@lucide/svelte";
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
  import type { RestoreSummary } from "./LibraryRestoreResult.svelte";

  /**
   * Body of the restore dialog: the first-connect choice, or the items of a plan with the
   * decisions the user makes on them. The footer stays at the bottom while the list scrolls.
   */
  let {
    plan,
    busy,
    message,
    alert,
    onChooseMode,
    onApply,
    onCancel,
  }: {
    plan: RestorePlan;
    busy: boolean;
    /** Shown above the content, such as why the dialog opened or why the last step failed. */
    message?: Snippet;
    /** Shown above the footer, next to the action that failed. */
    alert?: Snippet;
    onChooseMode: (mode: FirstConnectMode) => void;
    onApply: (
      decisions: RestoreDecisions,
      options: RestoreOptions,
      summary: RestoreSummary
    ) => void;
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
  const includedCount = $derived(incomingItems.filter((item) => included(item)).length);

  const stats = $derived(
    [
      {
        icon: CircleAlert,
        tone: "text-warning-content",
        count: conflictItems.length,
        key: "statConflicts",
      },
      {
        icon: ArrowDownToLine,
        tone: "text-info-content",
        count: incomingItems.length,
        key: "statIncoming",
      },
      {
        icon: Trash2,
        tone: "text-error",
        count: removalItems.length,
        key: "statRemovals",
      },
      {
        icon: ArrowUpFromLine,
        tone: "text-success",
        count: outgoingItems.length,
        key: "statOutgoing",
      },
    ].filter((stat) => stat.count > 0)
  );

  function included(item: RestoreItem): boolean {
    return include[item.key] ?? !item.deferred;
  }

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

  function setAllIncluded(value: boolean) {
    include = Object.fromEntries(incomingItems.map((item) => [item.key, value]));
  }

  function summarize(): RestoreSummary {
    const summary: RestoreSummary = {
      incoming: [],
      both: [],
      keptLocal: outgoingItems.map(itemName),
      removed: [],
      left: [],
    };
    for (const item of incomingItems) {
      (included(item) ? summary.incoming : summary.left).push(itemName(item));
    }
    for (const item of removalItems) {
      (removals[item.key] ? summary.removed : summary.left).push(itemName(item));
    }
    for (const item of conflictItems) {
      const choice = conflicts[item.key];
      if (choice === "keep_both") summary.both.push(itemName(item));
      else if (choice === "keep_local") summary.keptLocal.push(itemName(item));
      else if (item.remoteChange === "removed") summary.removed.push(itemName(item));
      else summary.incoming.push(itemName(item));
    }
    return summary;
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
      { applyInstalls },
      summarize()
    );
  }
</script>

{#snippet section(title: string, hint: string, count: number)}
  <div class="flex items-baseline justify-between gap-4">
    <h4 class="text-base-content text-[13px] font-medium">{title}</h4>
    <span class="text-base-content-muted text-xs">{count}</span>
  </div>
  <p class="text-base-content-muted mt-0.5 mb-2 text-xs leading-relaxed">{hint}</p>
{/snippet}

<div class="flex min-h-full flex-col">
  <div class="flex-1 space-y-5 px-6 pt-4 pb-5">
    {@render message?.()}

    {#if plan.state === "choose_mode"}
      <p class="text-base-content-muted text-[13px]">{$t("settings.sync.restore.chooseHint")}</p>
      <div class="space-y-2">
        {#each FIRST_CONNECT_MODES as mode (mode)}
          <label
            class={`flex cursor-pointer items-start gap-3 rounded-xl border px-4 py-3 text-[13px] transition ${
              firstConnectMode === mode
                ? "border-primary bg-base-200"
                : "border-base-300 hover:bg-base-200"
            }`}
          >
            <input
              type="radio"
              name="first-connect-mode"
              class="mt-0.5"
              value={mode}
              bind:group={firstConnectMode}
              disabled={busy}
            />
            <span>
              <span class="text-base-content font-medium">
                {$t(`settings.sync.restore.mode.${mode}`)}
              </span>
              <span class="text-base-content-muted mt-0.5 block text-xs leading-relaxed">
                {$t(`settings.sync.restore.mode.${mode}Hint`)}
              </span>
            </span>
          </label>
        {/each}
      </div>
    {:else if plan.items.length === 0}
      <p class="text-base-content-muted py-6 text-center text-[13px]">
        {$t("settings.sync.restore.nothing")}
      </p>
    {:else}
      <div class="bg-base-200 rounded-xl px-4 py-3">
        <p class="text-base-content text-[13px]">{$t("settings.sync.restore.summary")}</p>
        <ul class="mt-2 grid grid-cols-2 gap-x-4 gap-y-1.5 text-[13px]">
          {#each stats as stat (stat.key)}
            <li class="text-base-content flex items-center gap-2">
              <stat.icon size={14} class={`shrink-0 ${stat.tone}`} />
              {$t(`settings.sync.restore.${stat.key}`, { count: stat.count })}
            </li>
          {/each}
        </ul>
      </div>

      {#if conflictItems.length > 0}
        <section>
          {@render section(
            $t("settings.sync.restore.conflicts"),
            $t("settings.sync.restore.conflictsHint"),
            conflictItems.length
          )}
          <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
            {#each conflictItems as item (item.key)}
              <li class="flex items-center justify-between gap-4 px-3 py-2.5 text-[13px]">
                <span class="min-w-0">
                  <span class="text-base-content block truncate">{itemName(item)}</span>
                  <span class="text-base-content-muted block text-xs">
                    {[sides(item), note(item)].filter(Boolean).join(" · ")}
                  </span>
                </span>
                <div
                  class="border-base-300 bg-base-200 inline-flex shrink-0 gap-0.5 rounded-lg border p-0.5"
                  role="radiogroup"
                  aria-label={itemName(item)}
                >
                  {#each CONFLICT_CHOICES as choice (choice)}
                    {#if choice !== "keep_both" || canKeepBoth(item)}
                      <button
                        type="button"
                        role="radio"
                        aria-checked={conflicts[item.key] === choice}
                        class={`rounded-md px-2.5 py-1 text-xs transition ${
                          conflicts[item.key] === choice
                            ? "bg-primary text-primary-content"
                            : "text-base-content-muted hover:text-base-content"
                        }`}
                        onclick={() => (conflicts = { ...conflicts, [item.key]: choice })}
                        disabled={busy}
                      >
                        {$t(`settings.sync.restore.choice.${choice}`)}
                      </button>
                    {/if}
                  {/each}
                </div>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if incomingItems.length > 0}
        <section>
          {@render section(
            $t("settings.sync.restore.incoming"),
            $t("settings.sync.restore.incomingHint"),
            incomingItems.length
          )}
          <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
            {#if incomingItems.length > 1}
              <li class="px-3 py-2">
                <label class="text-base-content-muted flex items-center gap-2.5 text-xs">
                  <input
                    type="checkbox"
                    checked={includedCount === incomingItems.length}
                    indeterminate={includedCount > 0 && includedCount < incomingItems.length}
                    onchange={() => setAllIncluded(includedCount < incomingItems.length)}
                    disabled={busy}
                  />
                  {$t("settings.sync.restore.selectAll")}
                </label>
              </li>
            {/if}
            {#each incomingItems as item (item.key)}
              <li class="px-3 py-2">
                <label class="text-base-content flex items-center gap-2.5 text-[13px]">
                  <input
                    type="checkbox"
                    checked={included(item)}
                    onchange={() => (include = { ...include, [item.key]: !included(item) })}
                    disabled={busy}
                  />
                  <span class="min-w-0 truncate">
                    {itemName(item)}
                    <span class="text-base-content-muted text-xs">
                      · {[$t(`settings.sync.restore.change.${item.remoteChange}`), note(item)]
                        .filter(Boolean)
                        .join(" · ")}
                    </span>
                  </span>
                </label>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if removalItems.length > 0}
        <section>
          {@render section(
            $t("settings.sync.restore.removals"),
            $t("settings.sync.restore.removalsHint"),
            removalItems.length
          )}
          <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
            {#each removalItems as item (item.key)}
              <li class="px-3 py-2">
                <label class="text-base-content flex items-center gap-2.5 text-[13px]">
                  <input
                    type="checkbox"
                    checked={removals[item.key] ?? false}
                    onchange={() => (removals = { ...removals, [item.key]: !removals[item.key] })}
                    disabled={busy}
                  />
                  <span class="min-w-0 truncate">{itemName(item)}</span>
                </label>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if outgoingItems.length > 0}
        <section>
          {@render section(
            $t("settings.sync.restore.outgoing"),
            $t("settings.sync.restore.outgoingHint"),
            outgoingItems.length
          )}
          <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
            {#each outgoingItems as item (item.key)}
              <li class="text-base-content px-3 py-2 text-[13px]">
                {itemName(item)}
                <span class="text-base-content-muted text-xs">
                  · {$t(`settings.sync.restore.change.${item.localChange}`)}
                </span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}
  </div>

  <div class="bg-base-100 border-base-200 sticky bottom-0 space-y-3 border-t px-6 py-3">
    {@render alert?.()}
    <div class="flex items-center justify-between gap-4">
      {#if plan.state === "choose_mode"}
        <span></span>
      {:else}
        <label class="text-base-content flex items-center gap-2.5 text-[13px]">
          <input type="checkbox" bind:checked={applyInstalls} disabled={busy} />
          {$t("settings.sync.restore.applyInstalls")}
        </label>
      {/if}
      <div class="flex shrink-0 gap-2">
        <button class="settings-button" type="button" onclick={onCancel} disabled={busy}>
          {$t("common.cancel")}
        </button>
        {#if plan.state === "choose_mode"}
          <PrimaryActionButton
            className="h-8 rounded-lg px-3 py-0 text-[13px]"
            onclick={() => onChooseMode(firstConnectMode)}
            disabled={busy}
            loading={busy}
          >
            {$t("settings.sync.restore.next")}
          </PrimaryActionButton>
        {:else}
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
        {/if}
      </div>
    </div>
  </div>
</div>
