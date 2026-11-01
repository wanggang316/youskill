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
