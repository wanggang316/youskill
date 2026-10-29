<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { browser } from "$app/environment";
  import { page } from "$app/state";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Tooltip } from "bits-ui";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import AppTopBar from "$lib/components/AppTopBar.svelte";
  import DiffModal from "$lib/components/DiffModal.svelte";
  import ForceConfirmModal from "$lib/components/ForceConfirmModal.svelte";
  import GlobalSearchModal from "$lib/components/GlobalSearchModal.svelte";
  import ImportInstructionModal from "$lib/components/ImportInstructionModal.svelte";
  import ImportSkillModal from "$lib/components/ImportSkillModal.svelte";
  import InstallSkillModal from "$lib/components/InstallSkillModal.svelte";
  import LocationPickerModal from "$lib/components/LocationPickerModal.svelte";
  import ScanRootsModal from "$lib/components/ScanRootsModal.svelte";
  import ScopeAgentModal from "$lib/components/ScopeAgentModal.svelte";
  import SkillPickerModal from "$lib/components/SkillPickerModal.svelte";
  import UpdateDialog from "$lib/components/UpdateDialog.svelte";
  import WorkspaceModal from "$lib/components/WorkspaceModal.svelte";
  import { getAppLocation } from "$lib/navigation/app-shell";
  import { goBack, goForward } from "$lib/navigation/history";
  import { loadHomePath } from "$lib/stores/env";
  import {
    checkAllSourceUpdates,
    loadAgents,
    loadMigrationReport,
    refreshHub,
  } from "$lib/stores/hub";
  import { refreshInstructions } from "$lib/stores/instructions";
  import { globalSearchOpen, openImportModal } from "$lib/stores/modals";
  import { loadSettings } from "$lib/stores/settings";
  import {
    ensureUpdateChecked,
    installAvailableUpdate,
    openUpdateDialog,
    startAutomaticUpdateChecks,
    updaterState,
  } from "$lib/stores/updater";
  import { refreshUserProjects, refreshWorkspaces } from "$lib/stores/user-projects";

  let { children } = $props();

  const location = $derived(getAppLocation(page.url));

  /** ⌘[ and ⌘] (Ctrl on other platforms) go back and forward, like a browser. */
  const handleHistoryKeys = (event: KeyboardEvent) => {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
    if (event.key === "[") {
      event.preventDefault();
      goBack();
    } else if (event.key === "]") {
      event.preventDefault();
      goForward();
    }
  };
  const dragExcludedSelector = [
    "a",
    "button",
    "input",
    "select",
    "textarea",
    "[contenteditable='true']",
    "[data-window-drag-exclude]",
  ].join(",");

  const handleWindowDrag = (event: MouseEvent) => {
    if (event.button !== 0 || event.buttons !== 1) return;

    const target = event.target;
    if (!(target instanceof Element)) return;
    if (!target.closest("[data-window-drag-region]")) return;
    if (target.closest(dragExcludedSelector)) return;

    // Without this WebKit starts a text selection and shows the I-beam for the whole drag.
    event.preventDefault();
    getCurrentWindow()
      .startDragging()
      .catch((error) => {
        console.error("Failed to start window dragging:", error);
      });
  };

  $effect(() => {
    const action = page.url.searchParams.get("action");
    if (action === "add") openImportModal();
  });

  onMount(() => {
    if (!browser) {
      return () => {};
    }

    // The app shell is painted now; drop the static placeholder from app.html.
    document.getElementById("boot-skeleton")?.remove();

    let unlistenOpenInstallModal: UnlistenFn | null = null;
    let unlistenCheckForUpdates: UnlistenFn | null = null;
    let unlistenFocus: UnlistenFn | null = null;
    let lastFocusRefresh = 0;

    // The CLI edits the same hub; pick up its changes when the window comes back.
    const refreshOnFocus = () => {
      const now = Date.now();
      if (now - lastFocusRefresh < 2000) return;
      lastFocusRefresh = now;
      refreshHub().catch(console.error);
      refreshInstructions().catch(console.error);
      refreshUserProjects().catch(console.error);
    };

    // Load shared application state without blocking the first render.
    loadSettings().catch(console.error);
    loadAgents().catch(console.error);
    refreshUserProjects().catch(console.error);
    refreshWorkspaces().catch(console.error);
    loadHomePath().catch(console.error);
    refreshHub()
      .then(() => {
        void checkAllSourceUpdates();
        return loadMigrationReport();
      })
      .catch(console.error);
    refreshInstructions().catch(console.error);
    ensureUpdateChecked().catch(console.error);
    const stopAutomaticUpdateChecks = startAutomaticUpdateChecks({
      onFocus: (listener) =>
        getCurrentWindow().onFocusChanged(({ payload: focused }) => {
          if (focused) listener();
        }),
    });
    listen("open-install-modal", () => {
      openImportModal();
    })
      .then((unlisten) => {
        unlistenOpenInstallModal = unlisten;
      })
      .catch(console.error);
    // Sent by the "Check for Updates…" item in the macOS app menu.
    listen("check-for-updates", () => {
      openUpdateDialog();
    })
      .then((unlisten) => {
        unlistenCheckForUpdates = unlisten;
      })
      .catch(console.error);
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) refreshOnFocus();
      })
      .then((unlisten) => {
        unlistenFocus = unlisten;
      })
      .catch(console.error);
    document.addEventListener("mousedown", handleWindowDrag);
    document.addEventListener("keydown", handleHistoryKeys);

    return () => {
      unlistenOpenInstallModal?.();
      unlistenCheckForUpdates?.();
      unlistenFocus?.();
      stopAutomaticUpdateChecks();
      document.removeEventListener("mousedown", handleWindowDrag);
      document.removeEventListener("keydown", handleHistoryKeys);
    };
  });
</script>

<Tooltip.Provider delayDuration={150} disableHoverableContent>
  <div class="bg-chrome text-base-content flex h-dvh min-h-0 flex-col overflow-hidden">
    <AppTopBar
      activeKey={location.activeKey}
      hasUpdate={$updaterState.hasUpdate}
      updateLoading={$updaterState.installing}
      latestVersion={$updaterState.latestVersion}
      onImport={() => openImportModal()}
      onOpenLibrary={() => void checkAllSourceUpdates()}
      onOpenSearch={() => globalSearchOpen.set(true)}
      onOpenUpdate={() => installAvailableUpdate().catch(console.error)}
    />

    <section class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden px-2">
      {@render children()}
    </section>
  </div>

  <ImportSkillModal />
  <ImportInstructionModal />
  <InstallSkillModal />
  <SkillPickerModal />
  <LocationPickerModal />
  <ScopeAgentModal />
  <ForceConfirmModal />
  <DiffModal />
  <WorkspaceModal />
  <ScanRootsModal />
  <GlobalSearchModal />
  <UpdateDialog />
</Tooltip.Provider>
