<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import {
    FolderTree,
    LibraryBig,
    Loader2,
    Import,
    ScrollText,
    Search,
    Settings,
    Store,
  } from "@lucide/svelte";
  import { mergeProps, Tooltip } from "bits-ui";
  import type { Component, Snippet } from "svelte";
  import { BRAND_BOLT_PATH, BRAND_BOLT_VIEW_BOX } from "$lib/brand";
  import ShortcutKeys from "$lib/components/ui/ShortcutKeys.svelte";
  import { t } from "$lib/i18n";
  import {
    buildInstructionsHref,
    buildLibraryHref,
    buildMarketHref,
    buildScopeHref,
    type SidebarActiveKey,
  } from "$lib/navigation/app-shell";
  import {
    ariaKeyShortcut,
    HINT_KEY,
    isBareBinding,
    matchShortcut,
    shortcuts,
    type ShortcutAction,
  } from "$lib/shortcuts";

  let {
    activeKey,
    hasUpdate,
    updateLoading,
    latestVersion = "",
    onImport,
    onOpenLibrary,
    onOpenSearch,
    onOpenUpdate,
  }: {
    activeKey: SidebarActiveKey;
    hasUpdate: boolean;
    updateLoading: boolean;
    latestVersion?: string;
    onImport: () => void;
    onOpenLibrary: () => void;
    onOpenSearch: () => void;
    onOpenUpdate: () => void;
  } = $props();

  type NavItem = {
    key: SidebarActiveKey & ShortcutAction;
    href: string;
    label: string;
    icon: Component<{ size?: number; strokeWidth?: number }>;
    onclick?: () => void;
  };

  /** Local content; the marketplace sits apart as an icon button. */
  const items: NavItem[] = $derived([
    {
      key: "library",
      href: buildLibraryHref(),
      label: $t("sidebar.library"),
      icon: LibraryBig,
      onclick: onOpenLibrary,
    },
    {
      key: "instructions",
      href: buildInstructionsHref(),
      label: $t("sidebar.instructions"),
      icon: ScrollText,
    },
    { key: "projects", href: buildScopeHref(), label: $t("sidebar.projects"), icon: FolderTree },
  ]);

  const marketActive = $derived(activeKey === "market");
  const settingsActive = $derived(activeKey === "settings");

  /** Last page outside settings; the settings button toggles back to it. */
  let lastOutsideSettings = $state("/");
  $effect(() => {
    if (!settingsActive) lastOutsideSettings = `${page.url.pathname}${page.url.search}`;
  });
  /** Recessed track shared by the nav and the icon buttons; the active item floats above it. */
  const trackClass =
    "bg-base-content/[0.055] shadow-[inset_0_0_0_0.5px_color-mix(in_oklch,var(--color-base-content)_4%,transparent)]";
  const raisedClass =
    "bg-raised shadow-[0_1px_2px_0_color-mix(in_oklch,var(--color-base-content)_10%,transparent),0_0_0_0.5px_color-mix(in_oklch,var(--color-base-content)_8%,transparent)]";

  let nav = $state<HTMLElement | null>(null);
  /** Position of the active pill; it slides between items instead of jumping. */
  let indicator = $state({ left: 0, width: 0 });
  /** Hidden on pages outside the tabs (market, settings); keeps its last spot to fade out in place. */
  let indicatorVisible = $state(false);
  /** No slide on first paint, only on later moves. */
  let animate = $state(false);

  function measure() {
    const active = nav?.querySelector<HTMLElement>(`[data-nav-key="${activeKey}"]`);
    // Only write here, never read: this runs inside an effect.
    indicatorVisible = Boolean(active);
    if (active) indicator = { left: active.offsetLeft, width: active.offsetWidth };
  }

  $effect(() => {
    // Re-measure when the route or the labels (locale) change.
    void activeKey;
    void items;
    measure();
  });

  let header = $state<HTMLElement | null>(null);
  /** Shortcut keys replace the icons while the hint key (⌘) is held. */
  let hintsVisible = $state(false);
  let hintTimer: ReturnType<typeof setTimeout> | undefined;

  function hideHints() {
    clearTimeout(hintTimer);
    hintTimer = undefined;
    hintsVisible = false;
  }

  const isEditable = (target: EventTarget | null) =>
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      Boolean(target.closest("input, textarea, select, [contenteditable='true']")));

  /** Modals and menus own the keyboard while they are open. */
  const overlayOpen = () => Boolean(document.querySelector("[aria-modal='true'], [role='menu']"));

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === HINT_KEY) {
      const otherModifier =
        event.altKey || event.shiftKey || (HINT_KEY === "Meta" ? event.ctrlKey : event.metaKey);
      if (!event.repeat && !otherModifier && !hintTimer) {
        hintTimer = setTimeout(() => (hintsVisible = true), 500);
      }
      return;
    }
    hideHints();
    if (event.repeat || event.isComposing || event.defaultPrevented) return;

    const action = matchShortcut(event, $shortcuts);
    if (!action || overlayOpen()) return;
    if (isBareBinding($shortcuts[action]) && isEditable(event.target)) return;
    const target = header?.querySelector<HTMLElement>(`[data-shortcut="${action}"]`);
    if (!target) return;
    event.preventDefault();
    target.click();
  }

  function handleKeyup(event: KeyboardEvent) {
    if (event.key === HINT_KEY) hideHints();
  }

  onMount(() => {
    const observer = new ResizeObserver(measure);
    if (nav) observer.observe(nav);
    requestAnimationFrame(() => (animate = true));
    return () => {
      observer.disconnect();
      clearTimeout(hintTimer);
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} onkeyup={handleKeyup} onblur={hideHints} />

{#snippet shortcutSlot(action: ShortcutAction, icon: Snippet)}
  <span class="shortcut-slot relative inline-flex flex-none">
    <span class="shortcut-icon inline-flex">{@render icon()}</span>
    <span class="shortcut-hint absolute inset-0 flex items-center justify-center">
      <ShortcutKeys binding={$shortcuts[action]} />
    </span>
  </span>
{/snippet}

{#snippet tooltipContent(label: string, action: ShortcutAction)}
  <Tooltip.Portal>
    <Tooltip.Content
      class="bg-base-100 border-base-300 text-base-content pointer-events-none z-(--z-popover) flex items-center gap-2 rounded-lg border px-2 py-1 text-[12px] shadow-lg"
      side="bottom"
      sideOffset={8}
      collisionPadding={8}
    >
      <span class="whitespace-nowrap">{label}</span>
      <ShortcutKeys binding={$shortcuts[action]} />
    </Tooltip.Content>
  </Tooltip.Portal>
{/snippet}

<!-- The window controls sit at the left of this row (centred by `trafficLightPosition`). -->
<header
  bind:this={header}
  data-shortcut-hints={hintsVisible}
  class="grid h-15 flex-none grid-cols-[1fr_auto_1fr] items-center gap-4 pr-4 pl-[5.75rem]"
  data-window-drag-region
>
  <a
    class="group focus-visible:ring-primary/50 flex w-fit min-w-0 items-center gap-1.5 rounded-lg focus-visible:ring-2 focus-visible:outline-none"
    href={buildLibraryHref()}
    onclick={onOpenLibrary}
    data-icon="logo"
    aria-label="YouSkill"
  >
    <svg
      class="text-base-content h-[25px] w-auto flex-none"
      viewBox={BRAND_BOLT_VIEW_BOX}
      fill="currentColor"
      aria-hidden="true"
    >
      <defs>
        <clipPath id="brand-bolt-clip">
          <path d={BRAND_BOLT_PATH} />
        </clipPath>
        <linearGradient id="brand-bolt-sheen" x1="0" x2="1" y1="0" y2="0">
          <!-- Bell-shaped falloff so the band fades out softly at its edges. -->
          <stop offset="0" class="sheen-stop" stop-opacity="0" />
          <stop offset="0.25" class="sheen-stop" stop-opacity="0.1" />
          <stop offset="0.5" class="sheen-stop" stop-opacity="0.4" />
          <stop offset="0.75" class="sheen-stop" stop-opacity="0.1" />
          <stop offset="1" class="sheen-stop" stop-opacity="0" />
        </linearGradient>
      </defs>
      <path d={BRAND_BOLT_PATH} />
      <g clip-path="url(#brand-bolt-clip)">
        <rect class="sheen" width="340" height="455" fill="url(#brand-bolt-sheen)" />
      </g>
    </svg>
    <span class="text-base-content truncate pr-1 text-[16px] font-medium tracking-[-0.03em] italic">
      YouSkill
    </span>
  </a>

  <div class="flex items-center gap-2">
    <nav
      bind:this={nav}
      class={`${trackClass} relative flex h-9 items-center gap-0.5 rounded-full p-[3px]`}
      aria-label={$t("sidebar.navigation")}
    >
      <span
        class={`${raisedClass} absolute top-[3px] bottom-[3px] left-0 rounded-full ${
          animate
            ? "transition-[transform,width,opacity] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)]"
            : ""
        }`}
        style={`transform: translateX(${indicator.left}px); width: ${indicator.width}px; opacity: ${
          indicatorVisible ? 1 : 0
        };`}
        aria-hidden="true"
      ></span>
      {#each items as item (item.key)}
        {@const active = activeKey === item.key}
        {@const Icon = item.icon}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <a
                {...mergeProps(props, { onclick: item.onclick })}
                class={`focus-visible:ring-primary/50 relative z-10 flex h-full items-center gap-1.5 rounded-full px-3 text-[13px] whitespace-nowrap transition-colors duration-200 focus-visible:ring-2 focus-visible:outline-none ${
                  active
                    ? "text-base-content font-medium"
                    : "text-base-content-muted hover:text-base-content"
                }`}
                href={item.href}
                data-nav-key={item.key}
                data-icon={item.key}
                data-shortcut={item.key}
                aria-current={active ? "page" : undefined}
                aria-keyshortcuts={ariaKeyShortcut($shortcuts[item.key])}
              >
                {#snippet icon()}<Icon size={15} strokeWidth={1.8} />{/snippet}
                {@render shortcutSlot(item.key, icon)}
                {item.label}
              </a>
            {/snippet}
          </Tooltip.Trigger>
          {@render tooltipContent(item.label, item.key)}
        </Tooltip.Root>
      {/each}
    </nav>

    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <a
            {...props}
            class={`${trackClass} focus-visible:ring-primary/50 flex size-9 items-center justify-center rounded-full p-[3px] transition-colors duration-200 focus-visible:ring-2 focus-visible:outline-none ${
              marketActive ? "text-base-content" : "text-base-content-muted hover:text-base-content"
            }`}
            href={buildMarketHref()}
            data-icon="market"
            data-shortcut="market"
            aria-current={marketActive ? "page" : undefined}
            aria-label={$t("sidebar.market")}
            aria-keyshortcuts={ariaKeyShortcut($shortcuts.market)}
          >
            <span
              class={`flex size-full items-center justify-center rounded-full transition-[background-color,box-shadow] duration-200 ${
                marketActive ? raisedClass : ""
              }`}
            >
              {#snippet icon()}<Store size={16} strokeWidth={1.8} />{/snippet}
              {@render shortcutSlot("market", icon)}
            </span>
          </a>
        {/snippet}
      </Tooltip.Trigger>
      {@render tooltipContent($t("sidebar.market"), "market")}
    </Tooltip.Root>

    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            {...mergeProps(props, { onclick: onImport })}
            class={`${trackClass} focus-visible:ring-primary/50 text-base-content-muted hover:text-base-content flex size-9 cursor-pointer items-center justify-center rounded-full transition-[color,transform] duration-200 focus-visible:ring-2 focus-visible:outline-none active:scale-95`}
            type="button"
            data-icon="import"
            data-shortcut="import"
            aria-label={$t("import.title")}
            aria-keyshortcuts={ariaKeyShortcut($shortcuts.import)}
          >
            {#snippet icon()}<Import size={16} strokeWidth={1.8} />{/snippet}
            {@render shortcutSlot("import", icon)}
          </button>
        {/snippet}
      </Tooltip.Trigger>
      {@render tooltipContent($t("import.title"), "import")}
    </Tooltip.Root>
  </div>

  <div class="flex min-w-0 items-center justify-end gap-1.5">
    {#if hasUpdate}
      <button
        class="border-primary/25 bg-primary/8 text-primary hover:bg-primary/14 flex h-8 min-w-0 cursor-pointer items-center gap-1.5 rounded-full border px-2.5 text-[12px] font-medium transition-colors duration-150 disabled:cursor-default disabled:opacity-60"
        type="button"
        onclick={onOpenUpdate}
        disabled={updateLoading}
        title={$t("header.updateAvailable")}
      >
        {#if updateLoading}
          <Loader2 size={12} class="animate-spin" />
        {:else}
          <span class="relative flex size-1.5 flex-none" aria-hidden="true">
            <span class="bg-primary absolute inset-0 animate-ping rounded-full opacity-60"></span>
            <span class="bg-primary relative size-1.5 rounded-full"></span>
          </span>
        {/if}
        <span class="whitespace-nowrap">{$t("header.update")}</span>
        {#if latestVersion}
          <span class="whitespace-nowrap opacity-70 max-[1040px]:hidden">v{latestVersion}</span>
        {/if}
      </button>
    {/if}
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            {...mergeProps(props, { onclick: onOpenSearch })}
            class="text-base-content-muted hover:bg-base-content/[0.06] hover:text-base-content flex size-9 cursor-pointer items-center justify-center rounded-full transition-[background-color,color,transform] duration-200 active:scale-95"
            type="button"
            data-icon="search"
            data-shortcut="search"
            aria-label={$t("search.title")}
            aria-keyshortcuts={ariaKeyShortcut($shortcuts.search)}
          >
            {#snippet icon()}<Search size={17} strokeWidth={1.7} />{/snippet}
            {@render shortcutSlot("search", icon)}
          </button>
        {/snippet}
      </Tooltip.Trigger>
      {@render tooltipContent($t("search.title"), "search")}
    </Tooltip.Root>
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <a
            {...props}
            class={`flex size-9 items-center justify-center rounded-full transition-[background-color,color,transform] duration-200 active:scale-95 ${
              settingsActive
                ? "bg-base-content/[0.07] text-base-content"
                : "text-base-content-muted hover:bg-base-content/[0.06] hover:text-base-content"
            }`}
            href={settingsActive ? lastOutsideSettings : "/settings"}
            data-icon="settings"
            data-shortcut="settings"
            aria-current={settingsActive ? "page" : undefined}
            aria-label={$t("header.settings")}
            aria-keyshortcuts={ariaKeyShortcut($shortcuts.settings)}
          >
            {#snippet icon()}<Settings size={17} strokeWidth={1.7} />{/snippet}
            {@render shortcutSlot("settings", icon)}
          </a>
        {/snippet}
      </Tooltip.Trigger>
      {@render tooltipContent($t("header.settings"), "settings")}
    </Tooltip.Root>
  </div>
</header>

<!-- Hover motion follows what each icon shows. Lucide draws the parts as direct children of the svg. -->
<style>
  /* The import arrow rests a little higher than Lucide draws it, clear of the tray. */
  [data-icon="import"] :global(svg) {
    overflow: visible;
  }
  [data-icon="import"] :global(svg > :nth-child(-n + 2)) {
    transform: translateY(-3px);
  }

  /* Holding ⌘ cross-fades each icon into its shortcut keys. */
  .shortcut-icon,
  .shortcut-hint {
    transition: opacity 200ms ease;
  }
  .shortcut-hint {
    opacity: 0;
    pointer-events: none;
  }
  [data-shortcut-hints="true"] .shortcut-icon {
    opacity: 0;
  }
  [data-shortcut-hints="true"] .shortcut-hint {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    [data-icon] :global(svg),
    [data-icon] :global(svg > *) {
      transform-box: view-box;
      transition: transform 450ms cubic-bezier(0.34, 1.56, 0.64, 1);
    }

    /* Logo: a band of light passes over the bolt once. */
    .sheen {
      transform: translateX(-420px) skewX(-20deg);
    }
    [data-icon="logo"]:hover .sheen {
      animation: nav-sheen 1100ms ease-in-out;
    }

    /* Library: the leaning book stands up and lifts off the shelf. */
    [data-icon="library"] :global(svg > :nth-child(3)) {
      transform-origin: 18.5px 20.5px;
    }
    [data-icon="library"]:hover :global(svg > :nth-child(3)) {
      transform: translateY(-1.5px) rotate(17deg);
    }

    /* Instructions: the text lines on the scroll are written again, top line first. */
    [data-icon="instructions"]:hover :global(svg > :nth-child(-n + 2)) {
      stroke-dasharray: 5;
      animation: nav-write 360ms ease-out both;
    }
    [data-icon="instructions"]:hover :global(svg > :nth-child(1)) {
      animation-delay: 120ms;
    }

    /* Projects: the folders slide out along their branches. */
    [data-icon="projects"]:hover :global(svg > :nth-child(-n + 2)) {
      transform: translateX(1.5px);
    }
    [data-icon="projects"]:hover :global(svg > :nth-child(2)) {
      transition-delay: 60ms;
    }

    /* Market: the awning swings in the wind. */
    [data-icon="market"] :global(svg > :nth-child(2)) {
      transform-origin: 12px 2px;
    }
    [data-icon="market"]:hover :global(svg > :nth-child(2)) {
      animation: nav-swing 700ms ease-in-out;
    }

    /* Import: the arrow keeps dropping into the tray while hovered. */
    [data-icon="import"]:hover :global(svg > :nth-child(-n + 2)) {
      animation: nav-drop 900ms ease-in-out infinite;
    }

    /* Search: the lens tilts as if it looks closer. */
    [data-icon="search"] :global(svg) {
      transform-origin: 11px 11px;
    }
    [data-icon="search"]:hover :global(svg) {
      transform: rotate(-12deg) scale(1.06);
    }

    /* Settings: the gear turns. */
    [data-icon="settings"] :global(svg) {
      transition-duration: 600ms;
      transition-timing-function: cubic-bezier(0.22, 1, 0.36, 1);
    }
    [data-icon="settings"]:hover :global(svg) {
      transform: rotate(90deg);
    }
  }

  @keyframes nav-write {
    from {
      stroke-dashoffset: -5;
    }
    to {
      stroke-dashoffset: 0;
    }
  }

  /* Light on the dark bolt, shade on the light bolt of the dark theme. */
  .sheen-stop {
    stop-color: var(--color-base-100);
  }

  @keyframes nav-sheen {
    from {
      transform: translateX(-420px) skewX(-20deg);
    }
    to {
      transform: translateX(640px) skewX(-20deg);
    }
  }

  @keyframes nav-drop {
    0%,
    100% {
      transform: translateY(-3px);
    }
    50% {
      transform: translateY(-1.5px);
    }
  }

  @keyframes nav-swing {
    0%,
    100% {
      transform: rotate(0);
    }
    25% {
      transform: rotate(-7deg);
    }
    55% {
      transform: rotate(5deg);
    }
    80% {
      transform: rotate(-2deg);
    }
  }
</style>
