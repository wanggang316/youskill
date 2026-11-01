/**
 * Library sync API
 *
 * Back up the library to a private GitHub repository and bring it to other computers.
 */

import { apiCall } from "./index";

/** `agent` is a custom agent; `device` is this computer's installs. */
export type SyncItemKind = "skill" | "instruction" | "agent" | "device";
export type SyncChangeKind = "added" | "modified" | "removed";

export interface LibraryChange {
  kind: SyncItemKind;
  id: string;
  name: string;
  change: SyncChangeKind;
}

export interface SyncStatus {
  connected: boolean;
  repo?: string;
  branch?: string;
  deviceName?: string;
  lastBackupAt?: string;
  lastPullAt?: string;
  /** This computer has backed up or pulled at least once. */
  hasSynced: boolean;
  /** The last sync was with a version 1 repository; the next backup or restore may ask to migrate it. */
  legacyBase: boolean;
  /** Changes on this computer since the last sync. */
  changes: LibraryChange[];
  /** Skills left out of the backup (a file over 50 MB, a missing hub copy). */
  skipped: string[];
  libraryEmpty: boolean;
  /** The checkpoint of the last restore, which `undoLibraryRestore` puts back. */
  checkpoint?: RestoreCheckpoint;
}

export interface RestoreCheckpoint {
  createdAt: string;
  /** The commit the restore applied. */
  target: string;
}

export type BackupResult =
  | "pushed"
  | "up_to_date"
  | "remote_ahead"
  | "diverged"
  | "remote_not_empty"
  | "migration_required";

export interface BackupOutcome {
  result: BackupResult;
  commit?: string;
  changes: LibraryChange[];
  skipped: string[];
  /** Changes of other computers the backup kept for the next restore. */
  incoming: number;
}

/** `pull` merges against the last synced commit; the other modes are for a first connect. */
export type RestoreMode = "pull" | "merge" | "replace_local" | "replace_remote";
export type FirstConnectMode = Exclude<RestoreMode, "pull">;
export type ItemChange = "unchanged" | "added" | "modified" | "removed";
export type Resolution = "keep_local" | "take_remote" | "conflict" | "removal_candidate";
export type MatchReason = "hash" | "source_key";
export type PlanState =
  | "ready"
  | "up_to_date"
  | "remote_empty"
  | "choose_mode"
  | "migration_required";

export interface RestoreItem {
  /** `skill:<id>`, `instruction:<id>` or `agent:<id>`. */
  key: string;
  kind: SyncItemKind;
  id: string;
  name: string;
  /** Local id of an item paired with a repository item of another id. */
  localId?: string;
  localName?: string;
  localChange: ItemChange;
  remoteChange: ItemChange;
  resolution: Resolution;
  matchReason?: MatchReason;
  /** Left out of an earlier restore; excluded unless included again. */
  deferred: boolean;
  /** Another device made this item its device-only item: removed by default. */
  madeDeviceOnly: boolean;
}

export interface RestorePlan {
  state: PlanState;
  mode: RestoreMode;
  target: string | null;
  targetCommit: string | null;
  remoteHead: string | null;
  fingerprint: string;
  items: RestoreItem[];
}

export type ConflictChoice = "keep_local" | "take_remote" | "keep_both";
export type RemovalChoice = "remove" | "keep";

export interface RestoreDecisions {
  conflicts: Record<string, ConflictChoice>;
  removals: Record<string, RemovalChoice>;
  /** Include (`true`) or exclude a `take_remote` item. */
  include: Record<string, boolean>;
}

export interface RestoreOptions {
  applyInstalls: boolean;
}

export interface RestoreOutcome {
  result: "applied" | "stale";
  commit?: string;
  pushed: boolean;
  localApplied: boolean;
  agentsChanged: boolean;
  installs?: ApplyPendingOutcome;
}

export interface RemoteCommit {
  id: string;
  tree: string;
  time: string;
  message: string;
}

export type PendingAction = "install" | "uninstall" | "update";

export interface PendingItem {
  kind: SyncItemKind;
  id: string;
  name: string;
  action: PendingAction;
  agentId: string;
  path?: string;
}

export interface PendingFailure {
  kind: SyncItemKind;
  name: string;
  reason: string;
}

export interface ApplyPendingOutcome {
  applied: number;
  failures: PendingFailure[];
}

export async function getLibrarySyncStatus(): Promise<SyncStatus> {
  return apiCall<SyncStatus>("library_sync_status");
}

/** `repo` is `name` or `owner/name`; empty uses `youskill-library`. */
export async function connectLibrarySync(repo: string): Promise<SyncStatus> {
  return apiCall<SyncStatus>("library_sync_connect", { repo: repo.trim() || null });
}

export async function disconnectLibrarySync(): Promise<void> {
  return apiCall<void>("library_sync_disconnect");
}

/** Move a version 1 repository to item scopes and device files, after confirmation. */
export async function migrateLibrarySync(): Promise<void> {
  return apiCall<void>("library_sync_migrate");
}

export async function backupLibrary(): Promise<BackupOutcome> {
  return apiCall<BackupOutcome>("library_sync_backup");
}

/** `target` is a commit from `listLibraryHistory`; `null` is the branch head. */
export async function planLibraryRestore(
  mode: RestoreMode,
  target: string | null = null
): Promise<RestorePlan> {
  return apiCall<RestorePlan>("library_sync_plan_restore", { mode, target });
}

export async function applyLibraryRestore(
  plan: RestorePlan,
  decisions: RestoreDecisions,
  options: RestoreOptions
): Promise<RestoreOutcome> {
  return apiCall<RestoreOutcome>("library_sync_apply_restore", { plan, decisions, options });
}

export async function undoLibraryRestore(): Promise<void> {
  return apiCall<void>("library_sync_undo_restore");
}

export async function listLibraryHistory(): Promise<RemoteCommit[]> {
  return apiCall<RemoteCommit[]>("library_sync_history");
}

export async function listLibraryPending(): Promise<PendingItem[]> {
  return apiCall<PendingItem[]>("library_sync_pending");
}

export async function applyLibraryPending(items: PendingItem[]): Promise<ApplyPendingOutcome> {
  return apiCall<ApplyPendingOutcome>("library_sync_apply_pending", { items });
}

/** `shared`: every device gets the item. `device`: only this device gets it. */
export type ItemScope = "shared" | "device";

export interface DeviceItem {
  /** `skill:<id>`, `instruction:<id>` or `agent:<id>`. */
  key: string;
  kind: SyncItemKind;
  id: string;
  name: string;
}

export interface DeviceInfo {
  id: string;
  name: string;
  lastBackupAt?: string;
  installCount: number;
  thisDevice: boolean;
  /** The repository has a device file for it. */
  hasFile: boolean;
  /** Its device-only items. */
  items: DeviceItem[];
}

/** What happens to a device-only item of a removed device. */
export type DeviceItemChoice =
  | { action: "share" }
  | { action: "give"; device: string }
  | { action: "delete" };

/** The scope of a library item; `null` when the library is not connected. */
export async function getItemScope(key: string): Promise<ItemScope | null> {
  return apiCall<ItemScope | null>("library_sync_item_scope", { key });
}

/** A local edit: the next backup writes it. */
export async function setItemScope(key: string, scope: ItemScope): Promise<void> {
  return apiCall<void>("library_sync_set_item_scope", { key, scope });
}

/** Copy a device-only item of another device as this device's item. Returns the copy's key. */
export async function copyDeviceItem(key: string): Promise<string> {
  return apiCall<string>("library_sync_copy_device_item", { key });
}

export async function listSyncDevices(): Promise<DeviceInfo[]> {
  return apiCall<DeviceInfo[]>("library_sync_devices");
}

export async function renameSyncDevice(name: string): Promise<SyncStatus> {
  return apiCall<SyncStatus>("library_sync_rename_device", { name });
}

/** First connect only: this computer becomes `device`. */
export async function takeOverSyncDevice(device: string): Promise<SyncStatus> {
  return apiCall<SyncStatus>("library_sync_take_over_device", { device });
}

/** First connect only: start from the installs of `from`, or of the latest backup. */
export async function copySyncDeviceInstalls(from: string | null): Promise<SyncStatus> {
  return apiCall<SyncStatus>("library_sync_copy_device_installs", { from });
}

export async function removeSyncDevice(
  device: string,
  choices: Record<string, DeviceItemChoice>
): Promise<void> {
  return apiCall<void>("library_sync_remove_device", { device, choices });
}
