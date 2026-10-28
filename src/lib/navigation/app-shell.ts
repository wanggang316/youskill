import type { ScopeRef } from "$lib/api/hub";

export type SidebarActiveKey = "library" | "instructions" | "projects" | "market" | "settings";

export type LibraryFilter = "all" | "changed" | "uninstalled";

/** Where library skills came from: GitHub, a local folder or archive, or nowhere recorded. */
export type LibrarySource = "all" | "local" | "github" | "none";

export type AppLocation = {
  activeKey: SidebarActiveKey;
  /** Selected skill in the library (`?skill=`). */
  skill: string | null;
  /** Selected template on the instructions page (`?id=`). */
  instruction: string | null;
  /** Selected agent file on the instructions page (`?file=`). */
  instructionFile: string | null;
  /** Selected install scope on the projects page (`?scope=user` or `?project=<path>`). */
  scope: ScopeRef | null;
  filter: LibraryFilter;
  /** Source filter of the library (`?source=`). */
  source: LibrarySource;
};

/** Panes of the settings page, in nav order (`/settings?section=`). */
export const SETTINGS_SECTIONS = [
  "general",
  "agents",
  "translation",
  "github",
  "backup",
  "shortcuts",
  "cli",
  "about",
] as const;

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

const LIBRARY_FILTERS: LibraryFilter[] = ["all", "changed", "uninstalled"];
const LIBRARY_SOURCES: LibrarySource[] = ["all", "local", "github", "none"];

const isSettingsPath = (pathname: string) =>
  pathname === "/settings" || pathname.startsWith("/settings/");

const isMarketPath = (pathname: string) => pathname === "/market";

const isProjectsPath = (pathname: string) => pathname.startsWith("/projects");

const isInstructionsPath = (pathname: string) => pathname.startsWith("/instructions");

export const parseLibraryFilter = (value: string | null): LibraryFilter =>
  LIBRARY_FILTERS.includes(value as LibraryFilter) ? (value as LibraryFilter) : "all";

export const parseLibrarySource = (value: string | null): LibrarySource =>
  LIBRARY_SOURCES.includes(value as LibrarySource) ? (value as LibrarySource) : "all";

const parseScope = (url: URL): ScopeRef | null => {
  const projectPath = url.searchParams.get("project");
  if (projectPath) return { scope: "project", projectPath };
  if (url.searchParams.get("scope") === "user") return { scope: "user", projectPath: null };
  return null;
};

export const getAppLocation = (url: URL): AppLocation => {
  const scope = parseScope(url);

  let activeKey: SidebarActiveKey = "library";
  if (isSettingsPath(url.pathname)) {
    activeKey = "settings";
  } else if (isMarketPath(url.pathname)) {
    activeKey = "market";
  } else if (isProjectsPath(url.pathname)) {
    activeKey = "projects";
  } else if (isInstructionsPath(url.pathname)) {
    activeKey = "instructions";
  }

  return {
    activeKey,
    skill: url.searchParams.get("skill") || null,
    instruction: url.searchParams.get("id") || null,
    instructionFile: url.searchParams.get("file") || null,
    scope,
    filter: parseLibraryFilter(url.searchParams.get("filter")),
    source: parseLibrarySource(url.searchParams.get("source")),
  };
};

export const buildLibraryHref = (
  options: { skill?: string | null; filter?: LibraryFilter; source?: LibrarySource } = {}
): string => {
  const params = new URLSearchParams();
  if (options.filter && options.filter !== "all") params.set("filter", options.filter);
  if (options.source && options.source !== "all") params.set("source", options.source);
  if (options.skill) params.set("skill", options.skill);
  const query = params.toString();
  return query ? `/?${query}` : "/";
};

/** The instructions page with a template (`id`) or an agent file (`file`) selected. */
export const buildInstructionsHref = (
  options: { id?: string | null; file?: string | null; filter?: LibraryFilter } = {}
): string => {
  const params = new URLSearchParams();
  if (options.filter && options.filter !== "all") params.set("filter", options.filter);
  if (options.id) params.set("id", options.id);
  else if (options.file) params.set("file", options.file);
  const query = params.toString();
  return query ? `/instructions?${query}` : "/instructions";
};

/** The projects page, optionally with one scope selected. */
export const buildScopeHref = (scope?: ScopeRef | null): string => {
  if (!scope) return "/projects";
  if (scope.scope === "user") return "/projects?scope=user";
  return `/projects?project=${encodeURIComponent(scope.projectPath ?? "")}`;
};

/** The market page, optionally with one marketplace skill open in the content panel. */
export const buildMarketHref = (skillId?: string | null): string =>
  skillId ? `/market?skill=${encodeURIComponent(skillId)}` : "/market";

export const parseSettingsSection = (value: string | null): SettingsSection =>
  SETTINGS_SECTIONS.includes(value as SettingsSection) ? (value as SettingsSection) : "general";

export const buildSettingsHref = (section: SettingsSection = "general"): string =>
  section === "general" ? "/settings" : `/settings?section=${section}`;
