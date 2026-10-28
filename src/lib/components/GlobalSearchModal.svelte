<script lang="ts">
  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { Folder, Loader2, ScrollText, Search, User, Zap } from "@lucide/svelte";
  import type { Component } from "svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import {
    globalSearch,
    type CharRange,
    type SearchGroup,
    type SearchGroupKind,
    type SearchHit,
    type SearchItemKind,
  } from "$lib/api/search";
  import { t } from "$lib/i18n";
  import {
    buildInstructionsHref,
    buildLibraryHref,
    buildScopeHref,
  } from "$lib/navigation/app-shell";
  import { matchShortcut, shortcuts } from "$lib/shortcuts";
  import { skillLabel } from "$lib/stores/hub";
  import { globalSearchOpen } from "$lib/stores/modals";

  const DEBOUNCE_MS = 80;

  const GROUP_LABEL_KEYS: Record<SearchGroupKind, string> = {
    skills: "search.group.skills",
    instructions: "search.group.instructions",
    projects: "search.group.projects",
  };

  const ICONS: Record<SearchItemKind, Component<{ size?: number; strokeWidth?: number }>> = {
    skill: Zap,
    instruction: ScrollText,
    project: Folder,
    user: User,
  };

  let modal = $state<{ handleClose: () => void }>();
  let input = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLElement | null>(null);
  let query = $state("");
  let groups = $state<SearchGroup[]>([]);
  let loading = $state(false);
  let error = $state("");
  let selected = $state(0);
  /** Query whose results are shown; differs from `query` while a search is pending. */
  let shownQuery = $state("");

  let timer: ReturnType<typeof setTimeout> | undefined;
  /** Drops answers to queries that were replaced while they ran. */
  let requestSeq = 0;

  /** The hits belong to an older query until the search for this one returns. */
  const stale = $derived(loading || shownQuery !== query);

  /** Hits in display order, with their index into it. */
  const hits = $derived(groups.flatMap((group) => group.hits));
  const offsets = $derived(
    groups.reduce<number[]>((acc, group, index) => {
      acc.push(index === 0 ? 0 : acc[index - 1] + groups[index - 1].hits.length);
      return acc;
    }, [])
  );

  $effect(() => {
    if (!$globalSearchOpen) return;
    cancel();
    query = "";
    groups = [];
    shownQuery = "";
    error = "";
    selected = 0;
    // After the modal has focused its backdrop.
    tick().then(() => requestAnimationFrame(() => input?.focus()));
  });

  function schedule(text: string) {
    clearTimeout(timer);
    const seq = ++requestSeq;
    if (!text.trim()) {
      groups = [];
      shownQuery = "";
      loading = false;
      error = "";
      return;
    }
    loading = true;
    timer = setTimeout(async () => {
      try {
        const result = await globalSearch(text, $t("scope.user"));
        if (seq !== requestSeq || !$globalSearchOpen) return;
        groups = result;
        shownQuery = text;
        error = "";
        selected = 0;
      } catch (err) {
        if (seq !== requestSeq || !$globalSearchOpen) return;
        error = err instanceof Error ? err.message : String(err);
        groups = [];
      } finally {
        if (seq === requestSeq) loading = false;
      }
    }, DEBOUNCE_MS);
  }

  /** Stops the pending search, so that its answer is not shown. */
  function cancel() {
    clearTimeout(timer);
    requestSeq++;
    loading = false;
  }

  function close() {
    cancel();
    modal?.handleClose();
  }

  function hrefOf(hit: SearchHit): string {
    switch (hit.kind) {
      case "skill":
        return buildLibraryHref({ skill: hit.id });
      case "instruction":
        return buildInstructionsHref({ id: hit.id });
      case "project":
        return buildScopeHref({ scope: "project", projectPath: hit.id });
      case "user":
        return buildScopeHref({ scope: "user", projectPath: null });
    }
  }

  function openHit(hit: SearchHit | undefined) {
    if (!hit || stale) return;
    close();
    void goto(hrefOf(hit));
  }

  async function select(index: number) {
    if (hits.length === 0) return;
    selected = (index + hits.length) % hits.length;
    await tick();
    list
      ?.querySelector<HTMLElement>(`[data-hit-index="${selected}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (!event.repeat && matchShortcut(event, $shortcuts) === "search") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "Escape") {
      // Handled here instead of by the modal, so that the pending search is cancelled too.
      event.preventDefault();
      event.stopPropagation();
      close();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      void select(selected + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      void select(selected - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      openHit(hits[selected]);
    }
  }

  type Part = { text: string; marked: boolean };

  /** Split `text` into plain and marked parts; ranges count characters. */
  function parts(text: string, ranges: CharRange[]): Part[] {
    const chars = Array.from(text);
    const result: Part[] = [];
    let cursor = 0;
    for (const [start, end] of ranges) {
      if (start > cursor) result.push({ text: chars.slice(cursor, start).join(""), marked: false });
      result.push({ text: chars.slice(start, end).join(""), marked: true });
      cursor = end;
    }
    if (cursor < chars.length) result.push({ text: chars.slice(cursor).join(""), marked: false });
    return result;
  }

  /** The source a same-name skill gets in the library, after its name. */
  function nameSuffix(hit: SearchHit): string {
    if (hit.kind !== "skill") return "";
    const label = $skillLabel(hit.id);
    return label.startsWith(hit.name) ? label.slice(hit.name.length) : "";
  }
</script>

<Modal
  bind:this={modal}
  bind:open={$globalSearchOpen}
  showCloseButton={false}
  closeOnBackdropClick
  placement="top"
  containerClass="max-w-[640px]"
  onClose={() => {
    cancel();
    query = "";
    groups = [];
  }}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="flex max-h-[min(70vh,560px)] flex-col" onkeydown={handleKeydown}>
    <div class="border-base-200 flex items-center gap-2.5 border-b px-4">
      {#if loading}
        <Loader2 size={16} class="text-base-content-faint flex-none animate-spin" />
      {:else}
        <Search size={16} class="text-base-content-faint flex-none" />
      {/if}
      <input
        bind:this={input}
        bind:value={query}
        oninput={() => schedule(query)}
        class="text-base-content placeholder:text-base-content-faint h-12 min-w-0 flex-1 bg-transparent text-[14px] outline-none"
        type="text"
        placeholder={$t("search.placeholder")}
        aria-label={$t("search.title")}
        role="combobox"
        aria-expanded={hits.length > 0}
        aria-controls="global-search-results"
        aria-activedescendant={hits.length > 0 ? `global-search-hit-${selected}` : undefined}
        autocomplete="off"
        spellcheck="false"
      />
    </div>

    <div
      bind:this={list}
      id="global-search-results"
      class={`min-h-0 flex-1 overflow-y-auto p-2 transition-opacity ${stale && hits.length > 0 ? "opacity-60" : ""}`}
      role="listbox"
      aria-busy={stale}
      aria-label={$t("search.title")}
    >
      {#if error}
        <p class="text-error px-3 py-6 text-center text-[13px]">{error}</p>
      {:else if !query.trim()}
        <p class="text-base-content-muted px-3 py-6 text-center text-[13px] leading-relaxed">
          {$t("search.hint")}
        </p>
      {:else if hits.length === 0 && !loading && shownQuery}
        <p class="text-base-content-muted px-3 py-6 text-center text-[13px]">
          {$t("search.empty", { query: shownQuery })}
        </p>
      {:else}
        {#each groups as group, groupIndex (group.kind)}
          <div class="mb-1 last:mb-0" role="group" aria-label={$t(GROUP_LABEL_KEYS[group.kind])}>
            <div
              class="text-base-content-faint px-3 pt-2 pb-1 text-[11px] font-medium tracking-wide uppercase"
            >
              {$t(GROUP_LABEL_KEYS[group.kind])}
            </div>
            {#each group.hits as hit, hitIndex (`${hit.kind}:${hit.id}`)}
              {@const index = offsets[groupIndex] + hitIndex}
              {@const active = index === selected}
              {@const Icon = ICONS[hit.kind]}
              {@const suffix = nameSuffix(hit)}
              <button
                id={`global-search-hit-${index}`}
                data-hit-index={index}
                class={`flex w-full cursor-pointer items-start gap-3 rounded-lg px-3 py-2 text-left transition-colors ${
                  active ? "bg-base-content/[0.06]" : ""
                } ${hit.missing ? "opacity-60" : ""}`}
                type="button"
                role="option"
                aria-selected={active}
                tabindex="-1"
                onmousemove={() => (selected = index)}
                onclick={() => openHit(hit)}
              >
                <span class="text-base-content-muted mt-0.5 flex-none">
                  <Icon size={15} strokeWidth={1.8} />
                </span>
                <span class="min-w-0 flex-1">
                  <span class="flex min-w-0 items-center gap-2">
                    <span class="text-base-content truncate text-[13px]">
                      {#each parts(hit.name, hit.nameRanges) as part, partIndex (partIndex)}
                        {#if part.marked}<strong class="font-semibold">{part.text}</strong
                          >{:else}{part.text}{/if}
                      {/each}{#if suffix}<span class="text-base-content-muted">{suffix}</span>{/if}
                    </span>
                    {#if hit.missing}
                      <span class="text-error flex-none text-[11px]">{$t("search.missing")}</span>
                    {/if}
                  </span>
                  {#if hit.snippet}
                    <span class="text-base-content-muted mt-0.5 block truncate text-[12px]">
                      {#each parts( hit.snippet.text, [hit.snippet.range] ) as part, partIndex (partIndex)}
                        {#if part.marked}<mark class="bg-primary/15 text-base-content rounded-sm"
                            >{part.text}</mark
                          >{:else}{part.text}{/if}
                      {/each}
                    </span>
                  {:else if hit.detail}
                    <span class="text-base-content-muted mt-0.5 block truncate text-[12px]">
                      {hit.detail}
                    </span>
                  {/if}
                </span>
              </button>
            {/each}
          </div>
        {/each}
      {/if}
    </div>

    <div
      class="border-base-200 text-base-content-faint flex items-center gap-4 border-t px-4 py-2 text-[11px]"
    >
      <span class="flex items-center gap-1"><kbd>↑</kbd><kbd>↓</kbd> {$t("search.navigate")}</span>
      <span class="flex items-center gap-1"><kbd>↩</kbd> {$t("search.open")}</span>
      <span class="flex items-center gap-1"><kbd>Esc</kbd> {$t("search.close")}</span>
    </div>
  </div>
</Modal>
