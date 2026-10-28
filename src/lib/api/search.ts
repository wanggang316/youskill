/**
 * Global Search API
 *
 * One query over hub skills, instruction templates and install scopes. Matching, ranking
 * and snippets run in the backend; positions count characters (code points), so slice
 * with `Array.from(text)`.
 */

import { apiCall } from "./index";

export type SearchGroupKind = "skills" | "instructions" | "projects";

export type SearchItemKind = "skill" | "instruction" | "project" | "user";

export type MatchField = "name" | "description" | "path" | "body";

/** `[start, end)` in characters. */
export type CharRange = [number, number];

export interface SearchSnippet {
  text: string;
  range: CharRange;
}

export interface SearchHit {
  kind: SearchItemKind;
  /** Skill id, template id or project path; empty for the user scope. */
  id: string;
  name: string;
  field: MatchField;
  nameRanges: CharRange[];
  /** Skill description or scope path. */
  detail?: string | null;
  snippet?: SearchSnippet | null;
  missing: boolean;
}

export interface SearchGroup {
  kind: SearchGroupKind;
  hits: SearchHit[];
}

/** `userName` is the user scope label in the app language. */
export async function globalSearch(query: string, userName: string): Promise<SearchGroup[]> {
  return apiCall<SearchGroup[]>("global_search", { query, userName });
}
