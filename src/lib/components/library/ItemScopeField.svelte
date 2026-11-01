<script lang="ts">
  import { getItemScope, setItemScope, type ItemScope } from "$lib/api";
  import SegmentedTabs from "$lib/components/ui/SegmentedTabs.svelte";
  import { t } from "$lib/i18n";

  /**
   * The library sync scope of one item: every computer, or this computer only. Shown only
   * when the library is connected. `row` renders a `dt`/`dd` pair for a detail list.
   */
  let {
    itemKey,
    layout = "row",
  }: {
    /** `skill:<id>`, `instruction:<id>` or `agent:<id>`. */
    itemKey: string;
    layout?: "row" | "inline";
  } = $props();

  let scope = $state<ItemScope | null>(null);
  let saving = $state(false);
  let error = $state("");

  $effect(() => {
    const key = itemKey;
    scope = null;
    error = "";
    getItemScope(key)
      .then((value) => {
        if (key === itemKey) scope = value;
      })
      .catch(() => {
        if (key === itemKey) scope = null;
      });
  });

  async function change(value: string) {
    const next = value as ItemScope;
    if (saving || next === scope) return;
    const previous = scope;
    scope = next;
    saving = true;
    error = "";
    try {
      await setItemScope(itemKey, next);
    } catch (err) {
      scope = previous;
      error = err instanceof Error ? err.message : String(err);
    } finally {
      saving = false;
    }
  }
</script>

{#snippet control(current: ItemScope)}
  <SegmentedTabs
    items={[
      { value: "shared", label: $t("sync.scope.shared") },
      { value: "device", label: $t("sync.scope.device") },
    ]}
    value={current}
    onChange={change}
    className="text-xs"
  />
{/snippet}

{#if scope}
  {#if layout === "row"}
    <dt class="self-center">{$t("sync.scope.label")}</dt>
    <dd class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
      {@render control(scope)}
      <span class={error ? "text-error" : "text-base-content-subtle"}>
        {error || $t("sync.scope.hint")}
      </span>
    </dd>
  {:else}
    <div class="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]">
      <span class="text-base-content-muted">{$t("sync.scope.label")}</span>
      {@render control(scope)}
      {#if error}<span class="text-error">{error}</span>{/if}
    </div>
  {/if}
{/if}
