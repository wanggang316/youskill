<script lang="ts">
  import Icon from "@iconify/svelte";
  import { Grid2X2, GitFork, Tag } from "@lucide/svelte";

  let {
    icon,
    fallback = "category",
  }: {
    icon?: string | null;
    fallback?: "category" | "repository" | "all";
  } = $props();

  const name = $derived(icon?.trim() ?? "");
  const normalized = $derived(
    /^[A-Z][A-Za-z0-9]*$/.test(name)
      ? `lucide:${name
          .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
          .replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
          .toLowerCase()}`
      : name.toLowerCase()
  );
  const valid = $derived(/^[a-z0-9]+(?:-[a-z0-9]+)*:[a-z0-9]+(?:-[a-z0-9]+)*$/.test(normalized));
</script>

<span
  class="catalog-icon inline-flex size-4 shrink-0 items-center justify-center"
  aria-hidden="true"
>
  {#if valid}
    <Icon icon={normalized} width={16} height={16} />
  {/if}
  <span class="fallback inline-flex">
    {#if fallback === "all"}
      <Grid2X2 size={16} />
    {:else if fallback === "repository"}
      <GitFork size={16} />
    {:else}
      <Tag size={16} />
    {/if}
  </span>
</span>

<style>
  .catalog-icon:has(> :global(svg)) > .fallback {
    display: none;
  }
</style>
