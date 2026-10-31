<script lang="ts">
  import { ChevronLeft, Heart, Loader2 } from "@lucide/svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import SkillFileViewer from "$lib/components/SkillFileViewer.svelte";
  import { t } from "$lib/i18n";
  import { fetchRemoteSkill, type RemoteSkill } from "$lib/api/skills";
  import { favorites, favoritesState, isMissingSkillError } from "$lib/stores/favorites";

  /** Detail of one marketplace skill, shown in the content panel of the market page. */
  let {
    id,
    onBack,
  }: {
    id: string;
    onBack: () => void;
  } = $props();

  let loading = $state(true);
  let error = $state("");
  let skill = $state<RemoteSkill | null>(null);
  const favorite = $derived($favoritesState.items.find((item) => item.skillId === id));
  /** The skill left the marketplace; the favorite shows its saved snapshot. */
  const missing = $derived(Boolean(error) && isMissingSkillError(error));

  $effect(() => {
    const current = id;
    let cancelled = false;
    loading = true;
    error = "";
    skill = null;
    fetchRemoteSkill(current)
      .then((result) => {
        if (!cancelled) skill = result;
      })
      .catch((err) => {
        if (cancelled) return;
        error = String(err);
        if (isMissingSkillError(err)) favorites.markUnavailable(current);
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="surface-panel flex min-h-0 min-w-0 flex-col">
  <header
    class="border-base-300 flex h-12 shrink-0 items-center gap-3 border-b px-6"
    data-window-drag-region
  >
    <IconButton variant="outline" onclick={onBack} title={$t("header.back")} class="h-8 w-8 p-0">
      <ChevronLeft size={16} />
    </IconButton>
    <h2 class="text-base-content min-w-0 flex-1 truncate text-sm font-semibold">
      {skill?.name ?? favorite?.snapshot.name ?? ""}
    </h2>
    {#if skill || favorite}
      <IconButton
        onclick={() => (skill ? favorites.toggle(skill) : favorites.remove(id))}
        title={$t(favorite ? "market.unfavorite" : "market.favorite")}
        class={`h-8 w-8 shrink-0 p-0 ${favorite ? "text-secondary" : "text-base-content-subtle"}`}
      >
        <Heart size={16} fill={favorite ? "currentColor" : "none"} />
      </IconButton>
    {/if}
  </header>

  {#if loading}
    <div class="flex flex-1 items-center justify-center py-12" role="status">
      <Loader2 size={24} class="text-base-content-muted animate-spin" />
    </div>
  {:else if missing && favorite}
    <div class="p-6">
      <div class="border-base-300 bg-base-100 rounded-2xl border p-6">
        <p class="text-warning text-sm font-medium">{$t("market.unavailable.detail")}</p>
        <p class="text-base-content mt-4 text-sm font-semibold">{favorite.snapshot.name}</p>
        <p class="text-base-content-subtle mt-1 text-xs">{favorite.snapshot.source}</p>
        {#if favorite.snapshot.description}
          <p class="text-base-content-muted mt-3 text-xs leading-relaxed">
            {favorite.snapshot.description}
          </p>
        {/if}
        <button type="button" class="settings-button mt-4" onclick={() => favorites.remove(id)}
          >{$t("market.unfavorite")}</button
        >
      </div>
    </div>
  {:else if error || !skill}
    <div class="p-6">
      <div class="border-error-border bg-base-100 rounded-2xl border p-6 text-center">
        <p class="text-error">{error || $t("detail.notFound")}</p>
      </div>
    </div>
  {:else}
    {#key skill.id}
      <SkillFileViewer source={{ kind: "remote", name: skill.name, skill }} dense />
    {/key}
  {/if}
</div>
