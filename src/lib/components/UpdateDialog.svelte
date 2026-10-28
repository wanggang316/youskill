<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import appIcon from "$lib/assets/app-icon.png";
  import MarkdownPreview from "$lib/components/MarkdownPreview.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import { t } from "$lib/i18n";
  import { installAvailableUpdate, updateDialogOpen, updaterState } from "$lib/stores/updater";
  import { removeEmptySections, renderMarkdown } from "$lib/utils/markdown";

  /**
   * Follows the Sparkle windows on macOS: progress and update windows have a
   * "Software Update" title bar, results are shown as a narrow alert.
   */
  let open = $state(false);
  let modal = $state<ReturnType<typeof Modal>>();
  let currentVersion = $state("");

  type View = "checking" | "installing" | "available" | "error" | "upToDate";

  const view = $derived.by((): View => {
    const state = $updaterState;
    if (state.installing) return "installing";
    if (state.checking) return "checking";
    if (state.hasUpdate) return "available";
    if (state.error) return "error";
    return "upToDate";
  });
  const isAlert = $derived(view === "upToDate" || view === "error");
  // Published releases can keep changelog categories without entries.
  const releaseNotes = $derived(renderMarkdown(removeEmptySections($updaterState.releaseNotes)));
  const progress = $derived(
    $updaterState.contentLength > 0
      ? Math.min(100, ($updaterState.downloaded / $updaterState.contentLength) * 100)
      : null
  );
  const containerClass = $derived(
    `rounded-[20px]! border-base-content/10! ${
      view === "available" ? "max-w-[600px]" : isAlert ? "max-w-[280px]" : "max-w-[420px]"
    }`
  );

  const pillClass =
    "inline-flex h-7 cursor-pointer items-center justify-center rounded-full px-5 text-[13px] transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:outline-none";
  const secondaryClass = `${pillClass} bg-base-content/[0.07] text-base-content hover:bg-base-content/[0.11]`;
  const primaryClass = `${pillClass} bg-primary text-primary-content hover:bg-primary-hover font-medium`;

  $effect(() => {
    open = $updateDialogOpen;
  });

  onMount(async () => {
    try {
      currentVersion = await getVersion();
    } catch (error) {
      console.error("Failed to load app version:", error);
    }
  });

  function close() {
    if (modal) modal.handleClose();
    else updateDialogOpen.set(false);
  }

  function handleInstall() {
    // The store keeps the error; the dialog shows it in the "available" view.
    installAvailableUpdate().catch(console.error);
  }

  function formatMegabytes(bytes: number): string {
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<Modal
  bind:this={modal}
  bind:open
  showCloseButton={false}
  onClose={() => updateDialogOpen.set(false)}
  {containerClass}
>
  <div class="text-base-content text-[13px] leading-[1.45]" data-update-view={view}>
    {#if isAlert}
      <div class="flex flex-col px-5 pt-6 pb-5">
        <img class="size-14 select-none" src={appIcon} alt="" draggable="false" />
        <h3 class="mt-4 text-[14px] font-semibold">
          {view === "error" ? $t("updater.errorTitle") : $t("updater.upToDateTitle")}
        </h3>
        <p class="mt-1.5 break-words">
          {#if view === "error"}
            {$t("updater.checkFailed")}
            <span class="text-base-content-muted mt-1 block text-[12px]">
              {$updaterState.error}
            </span>
          {:else}
            {$t("updater.upToDateBody", { version: currentVersion })}
          {/if}
        </p>
        <button class={`${primaryClass} mt-5 w-full`} type="button" onclick={close}>
          {$t("updater.ok")}
        </button>
      </div>
    {:else}
      <div class="border-base-content/10 flex h-7.5 items-center border-b px-5">
        <span class="text-base-content-muted text-[13px] font-semibold">
          {$t("updater.windowTitle")}
        </span>
      </div>

      {#if view === "available"}
        <div class="px-5 pt-4 pb-5">
          <div class="flex items-center gap-4">
            <img class="size-14 flex-none select-none" src={appIcon} alt="" draggable="false" />
            <div class="min-w-0">
              <h3 class="text-[15px] font-semibold">{$t("updater.availableTitle")}</h3>
              <p class="text-base-content/80 mt-1">
                {$t("updater.availableBody", {
                  latest: $updaterState.latestVersion,
                  current: currentVersion,
                })}
              </p>
            </div>
          </div>
          <div
            class="update-notes border-base-content/10 bg-base-100 mt-4 h-60 overflow-y-auto rounded-[10px] border px-4 py-3"
          >
            {#if releaseNotes}
              <MarkdownPreview htmlContent={releaseNotes} onOpenExternalLink={openExternal} />
            {:else}
              <p class="text-base-content-muted">{$t("updater.noReleaseNotes")}</p>
            {/if}
          </div>
          {#if $updaterState.error}
            <p class="text-error mt-2 text-[12px] break-words">
              {$t("updater.installFailed")}
              {$updaterState.error}
            </p>
          {/if}
          <div class="mt-4 flex justify-end gap-2.5">
            <button class={secondaryClass} type="button" onclick={close}>
              {$t("updater.remindLater")}
            </button>
            <button class={`${primaryClass} min-w-28`} type="button" onclick={handleInstall}>
              {$t("updater.install")}
            </button>
          </div>
        </div>
      {:else}
        <div class="flex gap-4 px-5 pt-4 pb-5">
          <img class="size-14 flex-none select-none" src={appIcon} alt="" draggable="false" />
          <div class="flex min-w-0 flex-1 flex-col">
            <p class="mt-1 text-[14px] font-semibold">
              {view === "checking" ? $t("updater.checking") : $t("updater.downloading")}
            </p>
            <div
              class="bg-base-content/[0.07] relative mt-2.5 h-[5px] w-full overflow-hidden rounded-full"
            >
              {#if progress === null}
                <div
                  class="update-indeterminate bg-primary absolute inset-y-0 w-1/4 rounded-full"
                ></div>
              {:else}
                <div
                  class="bg-primary h-full rounded-full transition-[width] duration-200"
                  style={`width: ${progress}%`}
                ></div>
              {/if}
            </div>
            {#if view === "installing" && $updaterState.contentLength > 0}
              <p class="text-base-content-muted mt-1.5 text-[11px] tabular-nums">
                {formatMegabytes($updaterState.downloaded)} / {formatMegabytes(
                  $updaterState.contentLength
                )}
              </p>
            {/if}
            {#if view === "checking"}
              <div class="mt-4 flex justify-end">
                <button class={`${secondaryClass} min-w-24`} type="button" onclick={close}>
                  {$t("common.cancel")}
                </button>
              </div>
            {/if}
          </div>
        </div>
      {/if}
    {/if}
  </div>
</Modal>

<style>
  .update-notes :global(.markdown-content) {
    font-size: 13px;
    line-height: 1.5;
  }

  .update-notes :global(.markdown-content > :first-child) {
    margin-top: 0;
  }

  .update-indeterminate {
    animation: update-indeterminate 1.4s ease-in-out infinite;
  }

  @keyframes update-indeterminate {
    from {
      left: -25%;
    }
    to {
      left: 100%;
    }
  }
</style>
