<script module lang="ts">
  import type { RestoreOutcome } from "$lib/api";

  /** What the confirmed restore was asked to do, by item name. */
  export type RestoreSummary = {
    incoming: string[];
    both: string[];
    keptLocal: string[];
    removed: string[];
    left: string[];
  };

  export type RestoreResult =
    | { state: "done"; outcome: RestoreOutcome; summary: RestoreSummary }
    | { state: "failed"; message: string; canUndo: boolean };
</script>

<script lang="ts">
  import { CircleCheck, CircleX, TriangleAlert } from "@lucide/svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import LibraryInstallFailures from "./LibraryInstallFailures.svelte";

  /** The last step of the restore dialog: what the restore did, or why it stopped. */
  let {
    result,
    busy,
    onDone,
    onUndo,
    onRetry,
    onReviewPending,
  }: {
    result: RestoreResult;
    busy: boolean;
    onDone: () => void;
    onUndo: () => void;
    /** Plan the restore again. */
    onRetry: () => void;
    onReviewPending: () => void;
  } = $props();

  const DETAIL_KEYS = ["incoming", "both", "keptLocal", "removed", "left"] as const;

  const done = $derived(result.state === "done" ? result : null);
  const failed = $derived(result.state === "failed" ? result : null);
  const pushRejected = $derived(done?.outcome.result === "stale");
  const failures = $derived(done?.outcome.installs?.failures ?? []);

  const tiles = $derived.by(() => {
    if (!done) return [];
    const { summary, outcome } = done;
    const list = [
      { key: "incoming", count: summary.incoming.length + summary.both.length },
      { key: "keptLocal", count: summary.keptLocal.length },
      { key: "removed", count: summary.removed.length },
    ];
    if (outcome.installs) list.push({ key: "installed", count: outcome.installs.applied });
    return list;
  });

  /** The first line of an error is the sentence; the rest names the paths. */
  const errorLines = $derived(failed ? failed.message.split("\n") : []);
</script>

<div class="flex min-h-full flex-col">
  <div class="flex-1 space-y-5 px-6 pt-6 pb-5">
    {#if done}
      <div class="flex flex-col items-center gap-2 text-center">
        {#if pushRejected}
          <TriangleAlert size={36} class="text-warning-content" />
        {:else}
          <CircleCheck size={36} class="text-success" />
        {/if}
        <h4 class="text-base-content text-base font-medium">
          {pushRejected
            ? $t("settings.sync.result.pushRejectedTitle")
            : $t("settings.sync.result.doneTitle")}
        </h4>
        <p class="text-base-content-muted max-w-md text-[13px] leading-relaxed">
          {pushRejected
            ? $t("settings.sync.restorePushRejected")
            : done.outcome.pushed
              ? $t("settings.sync.result.donePushed")
              : $t("settings.sync.result.doneLocal")}
        </p>
      </div>

      <ul class="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {#each tiles as tile (tile.key)}
          <li class="bg-base-200 rounded-xl px-3 py-2.5 text-center">
            <div class="text-base-content text-xl font-medium tabular-nums">{tile.count}</div>
            <div class="text-base-content-muted mt-0.5 text-xs">
              {$t(`settings.sync.result.tile.${tile.key}`)}
            </div>
          </li>
        {/each}
      </ul>

      {#if failures.length > 0}
        <LibraryInstallFailures {failures} />
      {/if}

      <details class="border-base-300 rounded-xl border px-4 py-3 text-[13px]">
        <summary class="text-base-content cursor-pointer">
          {$t("settings.sync.result.details")}
        </summary>
        <dl class="mt-3 space-y-3">
          {#each DETAIL_KEYS as key (key)}
            {#if done.summary[key].length > 0}
              <div>
                <dt class="text-base-content-muted text-xs">
                  {$t(`settings.sync.result.detail.${key}`, {
                    count: done.summary[key].length,
                  })}
                </dt>
                <dd class="text-base-content mt-1 flex flex-wrap gap-1.5">
                  {#each done.summary[key] as name, index (index)}
                    <span class="tag tag-neutral text-xs">{name}</span>
                  {/each}
                </dd>
              </div>
            {/if}
          {/each}
        </dl>
      </details>
    {:else if failed}
      <div class="flex flex-col items-center gap-2 text-center">
        <CircleX size={36} class="text-error" />
        <h4 class="text-base-content text-base font-medium">
          {$t("settings.sync.result.failedTitle")}
        </h4>
        <p class="text-base-content-muted max-w-md text-[13px] leading-relaxed">
          {failed.canUndo
            ? $t("settings.sync.result.failedCanUndo")
            : $t("settings.sync.result.failedNothing")}
        </p>
      </div>
      <div class="bg-error/10 text-error rounded-xl px-4 py-3 text-[13px] [overflow-wrap:anywhere]">
        <p>{errorLines[0]}</p>
        {#if errorLines.length > 1}
          <ul class="mt-1.5 space-y-1 text-xs">
            {#each errorLines.slice(1) as line, index (index)}
              <li>{line}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>

  <div
    class="bg-base-100 border-base-200 sticky bottom-0 flex items-center justify-end gap-2 border-t px-6 py-3"
  >
    {#if done}
      {#if failures.length > 0}
        <button class="settings-button" type="button" onclick={onReviewPending} disabled={busy}>
          {$t("settings.sync.result.reviewPending")}
        </button>
      {/if}
      {#if pushRejected}
        <button class="settings-button" type="button" onclick={onDone} disabled={busy}>
          {$t("common.close")}
        </button>
        <PrimaryActionButton
          className="h-8 rounded-lg px-3 py-0 text-[13px]"
          onclick={onRetry}
          loading={busy}
        >
          {$t("settings.sync.result.restoreAgain")}
        </PrimaryActionButton>
      {:else}
        <PrimaryActionButton
          className="h-8 rounded-lg px-4 py-0 text-[13px]"
          onclick={onDone}
          disabled={busy}
        >
          {$t("settings.sync.result.finish")}
        </PrimaryActionButton>
      {/if}
    {:else if failed}
      <button class="settings-button" type="button" onclick={onDone} disabled={busy}>
        {$t("common.close")}
      </button>
      {#if failed.canUndo}
        <button class="settings-button" type="button" onclick={onUndo} disabled={busy}>
          {$t("settings.sync.undo")}
        </button>
      {/if}
      <PrimaryActionButton
        className="h-8 rounded-lg px-3 py-0 text-[13px]"
        onclick={onRetry}
        loading={busy}
      >
        {$t("settings.sync.result.retry")}
      </PrimaryActionButton>
    {/if}
  </div>
</div>
