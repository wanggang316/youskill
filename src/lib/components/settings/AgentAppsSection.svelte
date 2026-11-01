<script lang="ts">
  import { onMount } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { Loader2, Pencil, Plus, RefreshCw, Trash2 } from "@lucide/svelte";
  import { refreshAgentApps, removeAgentApp, type AgentApp } from "$lib/api";
  import AddAgentAppModal from "$lib/components/AddAgentAppModal.svelte";
  import AgentAppIcon from "$lib/components/AgentAppIcon.svelte";
  import ItemScopeField from "$lib/components/library/ItemScopeField.svelte";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import { t } from "$lib/i18n";
  import SettingsPane from "./SettingsPane.svelte";

  let internalApps = $state<AgentApp[]>([]);
  let userApps = $state<AgentApp[]>([]);
  let loading = $state(false);
  let error = $state("");
  let showAddModal = $state(false);
  let editingApp = $state<AgentApp | null>(null);

  onMount(() => {
    loadAgentApps();
  });

  async function loadAgentApps() {
    loading = true;
    error = "";
    try {
      const apps = await refreshAgentApps();
      internalApps = apps.filter((app) => !app.is_user_custom);
      userApps = apps.filter((app) => app.is_user_custom);
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  function openAddModal() {
    editingApp = null;
    showAddModal = true;
  }

  function openEditModal(app: AgentApp) {
    editingApp = app;
    showAddModal = true;
  }

  async function handleRemove(id: string, name: string) {
    const confirmed = await confirm($t("agentApps.removeConfirm") + ` (${name})`);
    if (!confirmed) return;
    try {
      await removeAgentApp(id);
      await loadAgentApps();
    } catch (err) {
      error = String(err);
    }
  }
</script>

{#snippet pathRows(app: AgentApp)}
  <dl
    class="text-base-content-muted mt-1 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-[10px] leading-4"
  >
    {#if app.global_path}
      <dt>{$t("agentApps.paths.userSkills")}</dt>
      <dd class="truncate" title={app.global_path}>{app.global_path}</dd>
    {/if}
    {#if app.global_profile_path}
      <dt>{$t("agentApps.paths.userMemory")}</dt>
      <dd class="truncate" title={app.global_profile_path}>{app.global_profile_path}</dd>
    {/if}
    {#if app.project_path}
      <dt>{$t("agentApps.paths.projectSkills")}</dt>
      <dd class="truncate" title={app.project_path}>{app.project_path}</dd>
    {/if}
    {#if app.profile_path}
      <dt>{$t("agentApps.paths.projectMemory")}</dt>
      <dd class="truncate" title={app.profile_path}>{app.profile_path}</dd>
    {/if}
  </dl>
{/snippet}

{#snippet appRow(app: AgentApp, editable: boolean)}
  <div class="bg-base-200 flex items-start gap-3 rounded-xl px-3 py-2.5">
    <AgentAppIcon agentId={app.id} name={app.display_name} />
    <div class="flex min-w-0 flex-1 flex-col">
      <span class="text-base-content text-[13px] font-medium">{app.display_name}</span>
      {@render pathRows(app)}
      {#if editable}
        <ItemScopeField itemKey={`agent:${app.id}`} layout="inline" />
      {/if}
    </div>
    {#if editable}
      <div class="flex shrink-0 gap-0.5">
        <button
          class="text-base-content-muted hover:bg-base-300 hover:text-base-content rounded-md p-1.5"
          onclick={() => openEditModal(app)}
          type="button"
          title={$t("agentApps.edit")}
          aria-label={$t("agentApps.edit")}
        >
          <Pencil size={13} />
        </button>
        <button
          class="text-error hover:bg-error/10 rounded-md p-1.5"
          onclick={() => handleRemove(app.id, app.display_name)}
          type="button"
          title={$t("agentApps.remove")}
          aria-label={$t("agentApps.remove")}
        >
          <Trash2 size={13} />
        </button>
      </div>
    {/if}
  </div>
{/snippet}

<SettingsPane title={$t("agentApps.title")} description={$t("agentApps.description")}>
  {#snippet actions()}
    <IconButton
      variant="outline"
      onclick={loadAgentApps}
      disabled={loading}
      title={$t("local.refresh")}
      class="h-8 w-8 p-0"
    >
      <RefreshCw size={15} class={loading ? "animate-spin" : ""} />
    </IconButton>
    <button class="settings-button" type="button" onclick={openAddModal}>
      <Plus size={14} />
      {$t("agentApps.add")}
    </button>
  {/snippet}

  {#if loading && internalApps.length === 0 && userApps.length === 0}
    <div class="text-base-content-muted flex items-center justify-center py-16">
      <Loader2 size={24} class="animate-spin" />
    </div>
  {:else if error}
    <p class="text-error text-[13px] break-all">{error}</p>
  {:else}
    {#if internalApps.length > 0}
      <div class="grid grid-cols-2 gap-2">
        {#each internalApps as app (app.id)}
          {@render appRow(app, false)}
        {/each}
      </div>
    {/if}

    {#if userApps.length > 0}
      <div class="space-y-2">
        <h3 class="text-base-content-subtle px-1 text-[11px] font-medium tracking-wide uppercase">
          {$t("agentApps.userSection")}
        </h3>
        <div class="grid grid-cols-2 gap-2">
          {#each userApps as app (app.id)}
            {@render appRow(app, true)}
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</SettingsPane>

<AddAgentAppModal bind:open={showAddModal} appToEdit={editingApp} onAppsChange={loadAgentApps} />
