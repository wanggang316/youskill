/**
 * Skills API
 *
 * Marketplace lookups, source detection (staging into temp dirs) and skill file access.
 * Hub operations live in ./hub.
 */

import { apiCall } from "./index";

// ============ Types ============

export interface RemoteSkill {
  id: string;
  skill_id: string;
  name: string;
  description?: string | null;
  star_count: number;
  heat_score: number;
  install_count: number;
  source: string;
  url?: string | null;
  path?: string | null;
  /** GitHub tree SHA for the skill folder (for update detection) */
  skill_path_sha?: string | null;
  /** Git branch name (e.g., "main", "master") */
  branch?: string | null;
}

export interface RemoteSkillsResponse {
  skills: RemoteSkill[];
  total: number;
  has_more: boolean;
}

export interface AgentInfo {
  id: string;
  display_name: string;
  project_path?: string | null;
  global_path?: string | null;
  /** Directory whose presence marks the app as installed (when global_path is shared). */
  detect_path?: string | null;
  /** Instruction file read inside a project (AGENTS.md, CLAUDE.md). */
  profile_path?: string | null;
  /** User-level instruction file (~/.claude/CLAUDE.md, ~/.codex/AGENTS.md). */
  global_profile_path?: string | null;
  is_user_custom: boolean;
}

export interface DetectedSkill {
  name: string;
  tmp_path: string;
  /** Path of SKILL.md relative to the detected root (repo root for GitHub sources). */
  skill_path: string;
  /** Branch the repository archive was downloaded from (GitHub only). */
  branch?: string | null;
}

export interface SkillDirectoryEntry {
  path: string;
  is_directory: boolean;
  /** Absolute target when the entry is a symlink, absent for a regular file or directory. */
  link_target?: string | null;
  /** A symlink whose target does not exist. */
  link_broken?: boolean;
}

function normalizeDetectedSkills(
  value: DetectedSkill | DetectedSkill[] | null | undefined
): DetectedSkill[] {
  if (!value) return [];
  return Array.isArray(value) ? value : [value];
}

// ============ Marketplace ============

export async function fetchRemoteSkills(params?: {
  skip?: number;
  limit?: number;
  search?: string;
  sortBy?: string;
  sortOrder?: string;
  tagId?: number;
  source?: string;
}): Promise<RemoteSkillsResponse> {
  return apiCall<RemoteSkillsResponse>("fetch_remote_skills", params || {});
}

export async function fetchSkillsByNames(names: string[]): Promise<RemoteSkill[]> {
  return apiCall<RemoteSkill[]>("fetch_skills_by_names", { names });
}

export async function recordInstall(skillId: string): Promise<void> {
  return apiCall<void>("record_skill_install", { skillId });
}

// ============ Detection ============

export async function detectGithubManual(githubPath: string): Promise<DetectedSkill[]> {
  return apiCall<DetectedSkill[]>("detect_github_manual", { githubPath });
}

export async function detectZip(zipPath: string): Promise<DetectedSkill[]> {
  const result = await apiCall<DetectedSkill | DetectedSkill[]>("detect_zip", { zipPath });
  return normalizeDetectedSkills(result);
}

export async function detectFolder(folderPath: string): Promise<DetectedSkill[]> {
  const result = await apiCall<DetectedSkill | DetectedSkill[]>("detect_folder", { folderPath });
  return normalizeDetectedSkills(result);
}

export async function detectGithubAuto(
  githubPath: string,
  skillName: string
): Promise<DetectedSkill> {
  return apiCall<DetectedSkill>("detect_github_auto", { githubPath, skillName });
}

// ============ Skill files ============

export async function readSkillFile(skillPath: string): Promise<string> {
  return apiCall<string>("read_skill_file", { skillPath });
}

export async function listSkillDirectory(skillPath: string): Promise<SkillDirectoryEntry[]> {
  return apiCall<SkillDirectoryEntry[]>("list_skill_directory", { skillPath });
}

export async function readSkillRelativeFile(
  skillPath: string,
  relativePath: string
): Promise<string> {
  return apiCall<string>("read_skill_relative_file", { skillPath, relativePath });
}

export async function readSkillRelativeFileBytes(
  skillPath: string,
  relativePath: string
): Promise<number[]> {
  return apiCall<number[]>("read_skill_relative_file_bytes", { skillPath, relativePath });
}

export async function openInFileManager(filePath: string): Promise<void> {
  return apiCall<void>("open_in_file_manager", { filePath });
}

export async function getCachedSkillTranslation(markdown: string): Promise<string | null> {
  return apiCall<string | null>("get_cached_skill_translation", { markdown });
}

/** Translates `markdown`. Set `force` to skip the cached translation and replace it. */
export async function translateSkillMarkdown(markdown: string, force = false): Promise<string> {
  return apiCall<string>("translate_skill_markdown", { markdown, force });
}

export interface MarketCategory {
  icon?: string | null;
  id: number;
  name: string;
  skill_count: number;
}

export interface MarketRepository extends MarketCategory {
  star_count?: number | null;
  enabled: boolean;
  type: string;
}

export function fetchMarketCategories(): Promise<MarketCategory[]> {
  return apiCall("fetch_market_categories");
}

export function fetchHotRepositories(): Promise<MarketRepository[]> {
  return apiCall("fetch_hot_repositories");
}

export function fetchRemoteSkill(skillId: string): Promise<RemoteSkill> {
  return apiCall("fetch_remote_skill", { skillId });
}
