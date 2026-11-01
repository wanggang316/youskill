<script lang="ts">
  import { onMount } from "svelte";
  import { AlertTriangle, Github, Loader2 } from "@lucide/svelte";
  import type { AccountProvider } from "$lib/api";
  import Modal from "$lib/components/ui/Modal.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import { t } from "$lib/i18n";
  import { account, accountState } from "$lib/stores/account";
  import SettingsPane from "./SettingsPane.svelte";
  import SettingsRow from "./SettingsRow.svelte";

  const providers: { key: AccountProvider; label: string }[] = $derived([
    { key: "github", label: $t("settings.account.signInWithGithub") },
    { key: "apple", label: $t("settings.account.signInWithApple") },
  ]);

  let confirmingDelete = $state(false);

  onMount(() => {
    void account.load();
  });

  async function confirmDelete() {
    await account.deleteAccount();
    // A failure shows on the page, where the person can try again.
    confirmingDelete = false;
  }
</script>

<SettingsPane title={$t("settings.account.title")} description={$t("settings.account.description")}>
  {#if !$accountState.loaded}
    <div class="text-base-content-muted flex items-center justify-center py-16">
      <Loader2 size={24} class="animate-spin" />
    </div>
  {:else if $accountState.identity}
    {@const identity = $accountState.identity}
    <div class="space-y-2">
      <div class="settings-card">
        <SettingsRow
          label={identity.displayName}
          description={$t("settings.account.signedInWith", {
            provider: $t(`settings.account.provider.${identity.provider}`),
          })}
        >
          <button
            class="settings-button"
            type="button"
            onclick={() => account.signOut()}
            disabled={$accountState.signingOut}
          >
            {#if $accountState.signingOut}
              <Loader2 size={13} class="animate-spin" />
              {$t("settings.account.signingOut")}
            {:else}
              {$t("settings.account.signOut")}
            {/if}
          </button>
        </SettingsRow>
      </div>
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.account.signOutHint")}</p>
      <div class="settings-card">
        <SettingsRow
          label={$t("settings.account.deleteAccount")}
          description={$t("settings.account.deleteAccountHint")}
        >
          <button
            class="settings-button text-error"
            type="button"
            onclick={() => (confirmingDelete = true)}
            disabled={$accountState.deleting || $accountState.signingOut}
          >
            {$t("settings.account.deleteAccount")}
          </button>
        </SettingsRow>
      </div>
    </div>
  {:else}
    <div class="settings-card">
      <SettingsRow
        label={$t("settings.account.signedOut")}
        description={$t("settings.account.signedOutHint")}
      />
      <div class="grid grid-cols-2 gap-3 px-4 pb-4">
        {#each providers as provider (provider.key)}
          <button
            class="settings-button h-10 justify-center"
            type="button"
            onclick={() => account.signIn(provider.key)}
          >
            {#if $accountState.signingIn === provider.key}
              <Loader2 size={15} class="animate-spin" />
            {:else if provider.key === "github"}
              <Github size={15} />
            {:else}
              <svg viewBox="0 0 24 24" class="size-[15px]" fill="currentColor" aria-hidden="true">
                <path
                  d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"
                />
              </svg>
            {/if}
            {provider.label}
          </button>
        {/each}
      </div>
    </div>
    {#if $accountState.signingIn}
      <p class="text-base-content-muted px-1 text-xs">{$t("settings.account.waiting")}</p>
    {/if}
  {/if}

  {#if $accountState.error}
    <p class="text-error text-[13px] break-all">{$accountState.error}</p>
  {/if}
</SettingsPane>

<Modal
  bind:open={confirmingDelete}
  title={$t("settings.account.deleteTitle")}
  containerClass="max-w-md"
>
  <div class="text-error flex items-start gap-2 px-6 pt-2 pb-6 text-sm">
    <AlertTriangle size={18} class="mt-0.5 shrink-0" />
    <p class="text-base-content">{$t("settings.account.deleteDescription")}</p>
  </div>
  {#snippet footer()}
    <button
      class="border-base-300 text-base-content hover:bg-base-200 rounded-xl border px-4 py-2 text-sm transition"
      type="button"
      onclick={() => (confirmingDelete = false)}
      disabled={$accountState.deleting}
    >
      {$t("common.cancel")}
    </button>
    <PrimaryActionButton onclick={confirmDelete} loading={$accountState.deleting} variant="danger">
      {$t("settings.account.deleteConfirm")}
    </PrimaryActionButton>
  {/snippet}
</Modal>
