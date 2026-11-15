/**
 * Instructions API
 *
 * Two things live here. Templates are Markdown files kept once under
 * ~/.youskill/instructions/<id>.md, keyed by a generated id with a free-text name, and
 * installed to the file each agent reads. Agent files are those AGENTS.md / CLAUDE.md
 * files themselves, listed and edited in place; they only enter the library when promoted
 * to a template. Install records share the hub types, with the template id in `name`.
 */

import { apiCall } from "./index";
import type {
  HubState,
  InstallRequest,
  InstallScope,
  InstallView,
  SkillDiff,
  SyncAction,
  TargetState,
  UninstallRequest,
} from "./hub";

// ============ Types ============

/** Where a template came from; nothing is synced with it, it is shown for context. */
export type InstructionSource =
  | { type: "github"; repo: string; filePath: string; branch?: string | null }
  | { type: "file"; path: string }
  | { type: "agent"; path: string }
  | { type: "none" };

export interface InstructionView {
  id: string;
  name: string;
  hubPath: string;
  source: InstructionSource;
  /** First line of the file. */
  description?: string | null;
  hash: string;
  hubHash?: string | null;
  hubState: HubState;
  importedAt: string;
  updatedAt: string;
  installs: InstallView[];
  hasDrift: boolean;
}

export interface InstructionImportItem {
  name: string;
  /** Markdown file to copy into the library. */
  path: string;
  /** Where the file came from; derived from the path when absent. */
  source?: InstructionSource | null;
}

export interface InstructionImportOutcome {
  id: string;
  name: string;
  hubPath: string;
  hash: string;
}

export interface InstructionActionResult {
  applied: boolean;
  blockers: string[];
  instruction?: InstructionView | null;
}

/** A Markdown file found in a folder, among picked files, or in a downloaded repository. */
export interface DetectedInstruction {
  /** Suggested name, editable before importing. */
  name: string;
  /** Absolute path to copy from (a temp directory for GitHub sources). */
  path: string;
  /** Path relative to the folder or repository it was found in. */
  relPath: string;
  fileName: string;
  /** Set for downloaded files, whose temp path says nothing about the origin. */
  source?: InstructionSource | null;
}

/** The template an agent file belongs to and how the file compares to it. */
export interface AgentFileTemplate {
  id: string;
  name: string;
  state: TargetState;
}

/** An instruction file that exists at an agent location. */
export interface AgentFileView {
  path: string;
  fileName: string;
  scope: InstallScope;
  projectPath?: string | null;
  /** Agents that read this file. */
  agentIds: string[];
  hash?: string | null;
  template?: AgentFileTemplate | null;
  /** Where the file links to when it is a symlink, absent for a regular file. */
  linkTarget?: string | null;
  /** Files a pure reference file imports (`@AGENTS.md`), absent for any other file. */
  references?: string[];
}

export function instructionSourceLabel(source: InstructionSource): string {
  switch (source.type) {
    case "github":
      return `${source.repo}/${source.filePath}`;
    case "file":
    case "agent":
      return source.path;
    default:
      return "";
  }
}

export function instructionSourceUrl(source: InstructionSource): string | null {
  if (source.type !== "github") return null;
  return `https://github.com/${source.repo}/blob/${source.branch || "main"}/${source.filePath}`;
}

// ============ Templates ============

export async function listInstructions(): Promise<InstructionView[]> {
  return apiCall<InstructionView[]>("list_instructions");
}

export async function getInstruction(id: string): Promise<InstructionView | null> {
  return apiCall<InstructionView | null>("get_instruction", { id });
}

export async function readInstruction(id: string): Promise<string> {
  return apiCall<string>("read_instruction", { id });
}

/** Save edited content; unmodified copies are pushed, edited ones come back as blockers. */
export async function writeInstruction(
  id: string,
  content: string
): Promise<InstructionActionResult> {
  return apiCall<InstructionActionResult>("write_instruction", { id, content });
}

export async function renameInstruction(id: string, name: string): Promise<InstructionView> {
  return apiCall<InstructionView>("rename_instruction", { id, name });
}

export async function importInstructions(
  items: InstructionImportItem[]
): Promise<InstructionImportOutcome[]> {
  return apiCall<InstructionImportOutcome[]>("import_instructions", { items });
}

export async function createInstruction(
  name: string,
  content = ""
): Promise<InstructionImportOutcome> {
  return apiCall<InstructionImportOutcome>("create_instruction", { name, content });
}

/** `request.name` is the template id. */
export async function installInstruction(
  request: InstallRequest
): Promise<InstructionActionResult> {
  return apiCall<InstructionActionResult>("install_instruction", { request });
}

/** `request.name` is the template id. */
export async function uninstallInstruction(
  request: UninstallRequest
): Promise<InstructionActionResult> {
  return apiCall<InstructionActionResult>("uninstall_instruction", { request });
}

export async function removeInstruction(id: string, removeInstalls: boolean): Promise<void> {
  return apiCall<void>("remove_instruction", { id, removeInstalls });
}

export async function syncInstruction(
  id: string,
  action: SyncAction
): Promise<InstructionActionResult> {
  return apiCall<InstructionActionResult>("sync_instruction", { id, action });
}

export async function diffInstruction(id: string, path: string): Promise<SkillDiff> {
  return apiCall<SkillDiff>("diff_instruction", { id, path });
}

/** Markdown files in the given files and folders. */
export async function detectInstructionFiles(paths: string[]): Promise<DetectedInstruction[]> {
  return apiCall<DetectedInstruction[]>("detect_instruction_files", { paths });
}

/** Markdown files in a GitHub repository, folder or file URL. */
export async function detectInstructionGithub(githubPath: string): Promise<DetectedInstruction[]> {
  return apiCall<DetectedInstruction[]>("detect_instruction_github", { githubPath });
}

// ============ Agent files ============

export async function listInstructionFiles(): Promise<AgentFileView[]> {
  return apiCall<AgentFileView[]>("list_instruction_files");
}

/** The agent files of one scope, also of a project that is not registered. */
export async function listScopeInstructionFiles(
  scope: InstallScope,
  projectPath?: string | null
): Promise<AgentFileView[]> {
  return apiCall<AgentFileView[]>("list_scope_instruction_files", {
    scope,
    projectPath: projectPath ?? null,
  });
}

export async function readInstructionFile(path: string): Promise<string> {
  return apiCall<string>("read_instruction_file", { path });
}

/** Write an agent file in place; a symlink into the library edits the template instead. */
export async function writeInstructionFile(path: string, content: string): Promise<void> {
  return apiCall<void>("write_instruction_file", { path, content });
}
