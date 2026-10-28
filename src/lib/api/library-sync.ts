/**
 * Library sync API
 *
 * Back up the library to a private GitHub repository and bring it to other computers.
 */

import { apiCall } from "./index";

export type SyncItemKind = "skill" | "instruction" | "agents";
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
}

export type BackupResult =
  | "pushed"
  | "up_to_date"
  | "remote_ahead"
  | "diverged"
  | "remote_not_empty";

export interface BackupOutcome {
  result: BackupResult;
  commit?: string;
  changes: LibraryChange[];
  skipped: string[];
}

export type PullResult = "pulled" | "up_to_date" | "remote_empty" | "diverged" | "local_not_empty";

export interface PullOutcome {
  result: PullResult;
  commit?: string;
  changes: LibraryChange[];
  errors: string[];
  agentsChanged: boolean;
}

export interface RemoteCheck {
  remoteAhead: boolean;
  remoteHasLibrary: boolean;
  changes: LibraryChange[];
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

export interface ApplyPendingOutcome {
  applied: number;
  failures: string[];
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

export async function backupLibrary(): Promise<BackupOutcome> {
  return apiCall<BackupOutcome>("library_sync_backup");
}

export async function checkLibraryRemote(): Promise<RemoteCheck> {
  return apiCall<RemoteCheck>("library_sync_check_remote");
}

export async function pullLibrary(): Promise<PullOutcome> {
  return apiCall<PullOutcome>("library_sync_pull");
}

export async function listLibraryPending(): Promise<PendingItem[]> {
  return apiCall<PendingItem[]>("library_sync_pending");
}

export async function applyLibraryPending(items: PendingItem[]): Promise<ApplyPendingOutcome> {
  return apiCall<ApplyPendingOutcome>("library_sync_apply_pending", { items });
}
