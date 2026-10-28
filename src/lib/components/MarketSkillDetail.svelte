<script lang="ts">
  import { ChevronLeft, Loader2 } from "@lucide/svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import SkillFileViewer from "$lib/components/SkillFileViewer.svelte";
  import { t } from "$lib/i18n";
  import { fetchRemoteSkill, type RemoteSkill } from "$lib/api/skills";

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
        if (!cancelled) error = String(err);
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
    <h2 class="text-base-content truncate text-sm font-semibold">{skill?.name ?? ""}</h2>
  </header>

  {#if loading}
    <div class="flex flex-1 items-center justify-center py-12" role="status">
      <Loader2 size={24} class="text-base-content-muted animate-spin" />
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
