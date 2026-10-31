<script lang="ts">
  import { TriangleAlert } from "@lucide/svelte";
  import type { PendingFailure } from "$lib/api";
  import { t } from "$lib/i18n";
  import { homePath } from "$lib/stores/env";

  /** Installs that were skipped, grouped by item, with the reason for each location. */
  let { failures }: { failures: PendingFailure[] } = $props();

  const groups = $derived.by(() => {
    const byName = new Map<string, string[]>();
    for (const failure of failures) {
      byName.set(failure.name, [...(byName.get(failure.name) ?? []), shortPath(failure.reason)]);
    }
    return [...byName].map(([name, reasons]) => ({ name, reasons }));
  });

  function shortPath(text: string): string {
    const home = $homePath;
    return home ? text.replaceAll(home, "~") : text;
  }
</script>

<div class="border-warning/40 bg-warning/10 rounded-xl border px-4 py-3">
  <div class="text-base-content flex items-start gap-2 text-[13px]">
    <TriangleAlert size={15} class="text-warning-content mt-0.5 shrink-0" />
    <div>
      <p class="font-medium">
        {$t("settings.sync.result.skippedTitle", { count: failures.length })}
      </p>
      <p class="text-base-content-muted mt-0.5 text-xs leading-relaxed">
        {$t("settings.sync.result.skippedHint")}
      </p>
    </div>
  </div>
  <ul class="mt-2 space-y-1.5 pl-6">
    {#each groups as group (group.name)}
      <li class="text-[13px]">
        <details>
          <summary class="text-base-content cursor-pointer">
            {group.name}
            <span class="text-base-content-muted text-xs">
              · {$t("settings.sync.result.locations", { count: group.reasons.length })}
            </span>
          </summary>
          <ul class="text-base-content-muted mt-1 space-y-1 pl-4 text-xs [overflow-wrap:anywhere]">
            {#each group.reasons as reason, index (index)}
              <li>{reason}</li>
            {/each}
          </ul>
        </details>
      </li>
    {/each}
  </ul>
</div>
