<script lang="ts">
  import { Check, Funnel } from "@lucide/svelte";
  import { DropdownMenu } from "bits-ui";
  import { menuContentClass } from "$lib/components/ui/menu";
  import { t } from "$lib/i18n";
  import {
    LIBRARY_FILTERS,
    LIBRARY_SOURCES,
    isLibraryFilterActive,
    type LibraryFilter,
    type LibrarySource,
  } from "$lib/navigation/app-shell";

  /** Status and source filters of the Library list in one menu. Each group is single-select. */
  let {
    filter = "all",
    source = "all",
    onFilterChange,
    onSourceChange,
  }: {
    filter?: LibraryFilter;
    source?: LibrarySource;
    onFilterChange: (filter: LibraryFilter) => void;
    onSourceChange: (source: LibrarySource) => void;
  } = $props();

  const active = $derived(isLibraryFilterActive(filter, source));

  const triggerClass = $derived(
    `focus-visible:ring-primary/40 flex size-8 shrink-0 items-center justify-center rounded-xl border transition outline-none focus-visible:ring-2 ${
      active
        ? "border-primary/40 bg-primary/10 text-primary hover:bg-primary/15"
        : "border-base-300 bg-base-100 text-base-content-muted hover:bg-base-200 hover:text-base-content data-[state=open]:bg-base-200 data-[state=open]:text-base-content"
    }`
  );

  const headingClass = "text-base-content-subtle px-2.5 pt-1.5 pb-1 text-[11px] font-medium";
  const itemClass =
    "text-base-content data-highlighted:bg-base-200 flex w-full cursor-default items-center gap-3 rounded-lg px-2.5 py-1.5 text-left text-[13px] whitespace-nowrap outline-none transition select-none";
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger
    class={triggerClass}
    title={$t("library.filterMenu")}
    aria-label={$t("library.filterMenu")}
    data-active={active ? "" : undefined}
  >
    <Funnel size={14} />
  </DropdownMenu.Trigger>
  <DropdownMenu.Portal>
    <DropdownMenu.Content
      class={`${menuContentClass} min-w-44`}
      align="end"
      sideOffset={4}
      collisionPadding={8}
    >
      <DropdownMenu.RadioGroup
        value={filter}
        onValueChange={(value) => onFilterChange(value as LibraryFilter)}
      >
        <DropdownMenu.GroupHeading class={headingClass}>
          {$t("library.filterMenu.status")}
        </DropdownMenu.GroupHeading>
        {#each LIBRARY_FILTERS as item (item)}
          <DropdownMenu.RadioItem class={itemClass} value={item} closeOnSelect={false}>
            {#snippet children({ checked })}
              <span class="min-w-0 flex-1">{$t(`library.filter.${item}`)}</span>
              <Check size={14} class={`text-primary shrink-0 ${checked ? "" : "invisible"}`} />
            {/snippet}
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
      <DropdownMenu.Separator class="bg-base-300 -mx-1 my-1 h-px" />
      <DropdownMenu.RadioGroup
        value={source}
        onValueChange={(value) => onSourceChange(value as LibrarySource)}
      >
        <DropdownMenu.GroupHeading class={headingClass}>
          {$t("library.filterMenu.source")}
        </DropdownMenu.GroupHeading>
        {#each LIBRARY_SOURCES as item (item)}
          <DropdownMenu.RadioItem class={itemClass} value={item} closeOnSelect={false}>
            {#snippet children({ checked })}
              <span class="min-w-0 flex-1">{$t(`library.source.${item}`)}</span>
              <Check size={14} class={`text-primary shrink-0 ${checked ? "" : "invisible"}`} />
            {/snippet}
          </DropdownMenu.RadioItem>
        {/each}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Content>
  </DropdownMenu.Portal>
</DropdownMenu.Root>
