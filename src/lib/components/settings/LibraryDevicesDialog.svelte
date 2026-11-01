<script lang="ts">
  import { untrack } from "svelte";
  import { Check, Copy, Loader2, Pencil, X } from "@lucide/svelte";
  import {
    copyDeviceItem,
    listSyncDevices,
    removeSyncDevice,
    renameSyncDevice,
    type DeviceInfo,
    type DeviceItem,
    type DeviceItemChoice,
    type SyncStatus,
  } from "$lib/api";
  import IconButton from "$lib/components/ui/IconButton.svelte";
  import Modal from "$lib/components/ui/Modal.svelte";
  import PrimaryActionButton from "$lib/components/ui/PrimaryActionButton.svelte";
  import SelectField from "$lib/components/ui/SelectField.svelte";
  import { t } from "$lib/i18n";

  /**
   * The devices of the sync repository: rename this computer, copy a device-only item of
   * another device, and remove a device with a choice for each of its device-only items.
   */
  let {
    open,
    onClose,
    onChanged,
  }: {
    open: boolean;
    onClose: () => void;
    /** Something changed: `status` after a rename, `agents` when a custom agent was added. */
    onChanged: (change: { status?: SyncStatus; agents?: boolean }) => void;
  } = $props();

  type Busy = "load" | "rename" | "remove" | `copy:${string}`;
  type Notice = { tone: "success" | "error"; text: string };

  let devices = $state<DeviceInfo[]>([]);
  let busy = $state<Busy | null>(null);
  let notice = $state<Notice | null>(null);
  let expanded = $state<string | null>(null);
  let renaming = $state(false);
  let nameDraft = $state("");
  /** The device being removed, and the choice for each of its items. */
  let removing = $state<DeviceInfo | null>(null);
  let choices = $state<Record<string, string>>({});

  /** Devices that can receive an item: this computer and those with a device file. */
  const receivers = $derived(
    devices.filter((device) => device.id !== removing?.id && (device.hasFile || device.thisDevice))
  );

  // Load on open only; `load` reads and writes `busy`, which must not rerun this effect.
  $effect(() => {
    if (open) untrack(() => void load());
  });

  const formatTime = (value?: string) => {
    if (!value) return "";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
  };

  const deviceName = (device: DeviceInfo) =>
    device.name || $t("settings.sync.device.unnamed", { id: device.id.slice(0, 8) });

  function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  async function run(kind: Busy, action: () => Promise<void>) {
    if (busy) return;
    busy = kind;
    notice = null;
    try {
      await action();
    } catch (error) {
      notice = { tone: "error", text: errorText(error) };
    } finally {
      busy = null;
    }
  }

  const load = () =>
    run("load", async () => {
      devices = await listSyncDevices();
    });

  function close() {
    removing = null;
    renaming = false;
    expanded = null;
    notice = null;
    onClose();
  }

  function startRename(device: DeviceInfo) {
    nameDraft = device.name;
    renaming = true;
  }

  const commitRename = () =>
    run("rename", async () => {
      const status = await renameSyncDevice(nameDraft);
      renaming = false;
      devices = await listSyncDevices();
      notice = { tone: "success", text: $t("settings.sync.device.renamed") };
      onChanged({ status });
    });

  function handleNameKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") void commitRename();
    if (event.key === "Escape") renaming = false;
  }

  const copy = (item: DeviceItem) =>
    run(`copy:${item.key}`, async () => {
      await copyDeviceItem(item.key);
      notice = { tone: "success", text: $t("settings.sync.device.copied", { name: item.name }) };
      onChanged({ agents: item.kind === "agent" });
    });

  function startRemove(device: DeviceInfo) {
    removing = device;
    notice = null;
    choices = Object.fromEntries(device.items.map((item) => [item.key, "share"]));
  }

  function choiceOf(value: string): DeviceItemChoice {
    if (value.startsWith("give:")) return { action: "give", device: value.slice(5) };
    return value === "delete" ? { action: "delete" } : { action: "share" };
  }

  const confirmRemove = () =>
    run("remove", async () => {
      const device = removing;
      if (!device) return;
      await removeSyncDevice(
        device.id,
        Object.fromEntries(device.items.map((item) => [item.key, choiceOf(choices[item.key])]))
      );
      removing = null;
      devices = await listSyncDevices();
      notice = {
        tone: "success",
        text: $t("settings.sync.device.removed", { name: deviceName(device) }),
      };
      onChanged({});
    });

  function summary(device: DeviceInfo): string {
    return [
      device.lastBackupAt
        ? $t("settings.sync.device.lastBackup", { time: formatTime(device.lastBackupAt) })
        : $t("settings.sync.device.neverBackedUp"),
      $t("settings.sync.device.installs", { count: device.installCount }),
      $t("settings.sync.device.items", { count: device.items.length }),
    ].join(" · ");
  }
</script>

{#snippet noticeView()}
  {#if notice}
    <p
      class={`rounded-xl px-4 py-3 text-[13px] [overflow-wrap:anywhere] ${
        notice.tone === "error" ? "text-error bg-error/10" : "text-success bg-base-200"
      }`}
    >
      {notice.text}
    </p>
  {/if}
{/snippet}

<Modal
  {open}
  title={removing
    ? $t("settings.sync.device.removeTitle", { name: deviceName(removing) })
    : $t("settings.sync.devicesTitle")}
  onClose={close}
  showCloseButton={busy === null}
  containerClass="max-w-lg"
>
  {#if removing}
    <div class="space-y-4 px-6 pt-4 pb-5 text-[13px]">
      <p class="text-base-content leading-relaxed">{$t("settings.sync.device.removeHint")}</p>
      {#if removing.items.length > 0}
        <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
          {#each removing.items as item (item.key)}
            <li class="flex items-center justify-between gap-3 px-3 py-2">
              <span class="text-base-content min-w-0 truncate">
                {item.name || item.id}
                <span class="text-base-content-muted text-xs">
                  · {$t(`settings.sync.device.kind.${item.kind}`)}
                </span>
              </span>
              <SelectField
                bind:value={choices[item.key]}
                disabled={busy !== null}
                fit
                className="shrink-0"
              >
                <option value="share">{$t("settings.sync.device.choice.share")}</option>
                {#each receivers as receiver (receiver.id)}
                  <option value={`give:${receiver.id}`}>
                    {$t("settings.sync.device.choice.give", { name: deviceName(receiver) })}
                  </option>
                {/each}
                <option value="delete">{$t("settings.sync.device.choice.delete")}</option>
              </SelectField>
            </li>
          {/each}
        </ul>
      {/if}
      {@render noticeView()}
    </div>
  {:else}
    <div class="space-y-3 px-6 pt-4 pb-5">
      <p class="text-base-content-muted text-xs leading-relaxed">
        {$t("settings.sync.devicesDialogHint")}
      </p>
      {@render noticeView()}
      {#if busy === "load" && devices.length === 0}
        <div class="text-base-content-muted flex items-center justify-center py-8">
          <Loader2 size={20} class="animate-spin" />
        </div>
      {:else}
        <ul class="border-base-300 divide-base-300 divide-y rounded-xl border">
          {#each devices as device (device.id)}
            <li class="space-y-2 px-3 py-2.5 text-[13px]">
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0 flex-1">
                  {#if device.thisDevice && renaming}
                    <div class="flex items-center gap-1.5">
                      <!-- svelte-ignore a11y_autofocus -->
                      <input
                        class="border-base-300 bg-base-100 text-base-content focus:border-primary h-8 min-w-0 flex-1 rounded-lg border px-2.5 text-[13px] outline-none"
                        bind:value={nameDraft}
                        onkeydown={handleNameKeydown}
                        aria-label={$t("settings.sync.device.rename")}
                        disabled={busy !== null}
                        autofocus
                      />
                      <IconButton
                        variant="outline"
                        onclick={commitRename}
                        title={$t("common.confirm")}
                        class="h-8 w-8 p-0"
                        disabled={busy !== null}
                      >
                        <Check size={14} />
                      </IconButton>
                      <IconButton
                        variant="outline"
                        onclick={() => (renaming = false)}
                        title={$t("common.cancel")}
                        class="h-8 w-8 p-0"
                      >
                        <X size={14} />
                      </IconButton>
                    </div>
                  {:else}
                    <span class="text-base-content flex min-w-0 items-center gap-2">
                      <span class="truncate font-medium">{deviceName(device)}</span>
                      {#if device.thisDevice}
                        <span class="tag tag-info shrink-0">
                          {$t("settings.sync.device.thisDevice")}
                        </span>
                      {/if}
                    </span>
                  {/if}
                  <span class="text-base-content-muted mt-0.5 block text-xs">{summary(device)}</span
                  >
                </div>
                <div class="flex shrink-0 items-center gap-1.5">
                  {#if device.thisDevice}
                    {#if !renaming}
                      <IconButton
                        variant="ghost"
                        onclick={() => startRename(device)}
                        title={$t("settings.sync.device.rename")}
                        class="h-8 w-8 p-0"
                        disabled={busy !== null}
                      >
                        <Pencil size={14} />
                      </IconButton>
                    {/if}
                  {:else}
                    {#if device.items.length > 0}
                      <button
                        class="settings-button"
                        type="button"
                        onclick={() => (expanded = expanded === device.id ? null : device.id)}
                        disabled={busy !== null}
                      >
                        {$t("settings.sync.device.showItems")}
                      </button>
                    {/if}
                    <button
                      class="settings-button text-error"
                      type="button"
                      onclick={() => startRemove(device)}
                      disabled={busy !== null}
                    >
                      {$t("settings.sync.device.remove")}
                    </button>
                  {/if}
                </div>
              </div>
              {#if expanded === device.id && !device.thisDevice}
                <ul class="bg-base-200 divide-base-300 divide-y rounded-lg">
                  {#each device.items as item (item.key)}
                    <li class="flex items-center justify-between gap-3 px-3 py-1.5">
                      <span class="text-base-content min-w-0 truncate">
                        {item.name || item.id}
                        <span class="text-base-content-muted text-xs">
                          · {$t(`settings.sync.device.kind.${item.kind}`)}
                        </span>
                      </span>
                      <button
                        class="settings-button shrink-0"
                        type="button"
                        onclick={() => copy(item)}
                        disabled={busy !== null}
                        title={$t("settings.sync.device.copyHint")}
                      >
                        {#if busy === `copy:${item.key}`}
                          <Loader2 size={13} class="animate-spin" />
                        {:else}
                          <Copy size={13} />
                        {/if}
                        {$t("settings.sync.device.copy")}
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    {#if removing}
      <button
        class="settings-button"
        type="button"
        onclick={() => (removing = null)}
        disabled={busy !== null}
      >
        {$t("common.cancel")}
      </button>
      <PrimaryActionButton
        className="h-8 rounded-lg px-3 py-0 text-[13px]"
        variant="danger"
        onclick={confirmRemove}
        disabled={busy !== null && busy !== "remove"}
        loading={busy === "remove"}
      >
        {$t("settings.sync.device.confirmRemove")}
      </PrimaryActionButton>
    {:else}
      <button class="settings-button" type="button" onclick={close} disabled={busy !== null}>
        {$t("common.close")}
      </button>
    {/if}
  {/snippet}
</Modal>
