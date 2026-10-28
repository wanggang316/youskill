<script lang="ts">
  import { onMount, type Component } from "svelte";

  export type SegmentedTabItem = {
    value: string;
    label: string;
    icon?: Component<{ size?: number }>;
  };

  let {
    items = [],
    value = "",
    onChange = () => {},
    fullWidth = false,
    surface = "panel",
    className = "",
  } = $props<{
    items?: SegmentedTabItem[];
    value?: string;
    onChange?: (value: string) => void;
    fullWidth?: boolean;
    surface?: "panel" | "sidebar";
    className?: string;
  }>();

  let track = $state<HTMLElement | null>(null);
  /** Position of the active pill; it slides between items instead of jumping. */
  let indicator = $state({ left: 0, width: 0 });
  let indicatorVisible = $state(false);
  /** No slide on first paint, only on later moves. */
  let animate = $state(false);

  function measure() {
    const active = track?.querySelector<HTMLElement>(`[data-tab-value="${CSS.escape(value)}"]`);
    // Only write here, never read: this runs inside an effect.
    indicatorVisible = Boolean(active);
    if (active) indicator = { left: active.offsetLeft, width: active.offsetWidth };
  }

  $effect(() => {
    // Re-measure when the selection or the labels (locale) change.
    void value;
    void items;
    measure();
  });

  onMount(() => {
    const observer = new ResizeObserver(measure);
    if (track) observer.observe(track);
    requestAnimationFrame(() => (animate = true));
    return () => observer.disconnect();
  });
</script>

<div
  bind:this={track}
  class={`${surface === "sidebar" ? "bg-base-content/[0.05]" : "bg-base-200"} relative flex gap-2 rounded-full p-1 ${className}`}
>
  <span
    class={`${surface === "sidebar" ? "bg-raised" : "bg-base-100"} absolute top-1 bottom-1 left-0 rounded-full shadow-sm ${
      animate
        ? "transition-[transform,width,opacity] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none"
        : ""
    }`}
    style={`transform: translateX(${indicator.left}px); width: ${indicator.width}px; opacity: ${
      indicatorVisible ? 1 : 0
    };`}
    aria-hidden="true"
  ></span>
  {#each items as item (item.value)}
    <button
      class={`hover:text-base-content relative z-10 rounded-full px-4 py-2 text-sm transition-colors duration-200 ${
        fullWidth ? "flex flex-1 items-center justify-center gap-2" : ""
      } ${value === item.value ? "text-base-content" : "text-base-content-muted"}`}
      onclick={() => onChange(item.value)}
      type="button"
      data-tab-value={item.value}
    >
      {#if item.icon}
        {@const Icon = item.icon}
        <Icon size={16} />
      {/if}
      {item.label}
    </button>
  {/each}
</div>
