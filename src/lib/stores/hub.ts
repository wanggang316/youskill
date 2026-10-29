import { derived, get, writable } from "svelte/store";
import { listLocalAgentApps } from "../api/agent-apps";
import {
  checkSourceUpdates as checkSourceUpdatesApi,
  listHubSkills,
  migrationStatus,
  sourceLabel,
  type HubSkillView,
  type MigrationReport,
  type SourceUpdate,
} from "../api/hub";
import type { AgentInfo } from "../api/skills";

export const agents = writable<AgentInfo[]>([]);

export const hubSkills = writable<HubSkillView[]>([]);
export const hubLoading = writable(false);
export const hubError = writable("");
export const hubLoaded = writable(false);

export const sourceChecking = writable(false);
export const lastSourceCheck = writable<SourceUpdate[]>([]);

export const migrationReport = writable<MigrationReport | null>(null);

export const hubSkillsById = derived(
  hubSkills,
  ($skills) => new Map($skills.map((skill) => [skill.id, skill]))
);

/** Names that more than one skill has; their rows also show the source. */
export const sharedSkillNames = derived(hubSkills, ($skills) => {
  const seen = new Set<string>();
  const shared = new Set<string>();
  for (const skill of $skills) {
    const name = skill.name.toLowerCase();
    if (seen.has(name)) shared.add(name);
    seen.add(name);
  }
  return shared;
});

/** Display name of a skill id: the name, plus the source when another skill has the name. */
export const skillLabel = derived([hubSkillsById, sharedSkillNames], ([$byId, $shared]) => {
  return (id: string): string => {
    const skill = $byId.get(id);
    if (!skill) return id;
    const source = sourceHint(skill);
    return $shared.has(skill.name.toLowerCase()) ? `${skill.name} (${source})` : skill.name;
  };
});

/** Short source of a skill: `owner/repo@branch`, or the folder or archive name. */
function sourceHint(skill: HubSkillView): string {
  const { source } = skill;
  if (source.type === "folder" || source.type === "zip") {
    return source.path.split(/[/\\]/).filter(Boolean).pop() ?? source.path;
  }
  return sourceLabel(source) || skill.sourceKey;
}

function compareSkills(a: HubSkillView, b: HubSkillView): number {
  return a.name.localeCompare(b.name) || a.sourceKey.localeCompare(b.sourceKey);
}

export const agentsById = derived(
  agents,
  ($agents) => new Map($agents.map((agent) => [agent.id, agent]))
);

export async function loadAgents(): Promise<void> {
  try {
    agents.set(await listLocalAgentApps());
  } catch (error) {
    console.error(error);
  }
}

/**
 * A manual refresh, or the first load while the list has no data, shows the loading state and
 * its error. Other refreshes run in the background: they update the list and only log errors.
 */
export interface RefreshOptions {
  manual?: boolean;
}

/** An in-flight run that every caller shares; any foreground caller makes it foreground. */
interface SharedRun<T> {
  promise: Promise<T>;
  foreground: boolean;
}

let hubRun: SharedRun<void> | null = null;

/** `hubLoading` stays set while any foreground hub run or manual refresh holds it. */
let hubLoadingHolds = 0;

function holdHubLoading(): void {
  hubLoadingHolds += 1;
  hubLoading.set(true);
}

function releaseHubLoading(): void {
  hubLoadingHolds -= 1;
  if (hubLoadingHolds === 0) hubLoading.set(false);
}

export function refreshHub(options: RefreshOptions = {}): Promise<void> {
  const foreground = options.manual === true || !get(hubLoaded);
  if (foreground) hubError.set("");
  if (hubRun) {
    if (foreground && !hubRun.foreground) {
      hubRun.foreground = true;
      holdHubLoading();
    }
    return hubRun.promise;
  }
  if (foreground) holdHubLoading();
  const run: SharedRun<void> = { promise: Promise.resolve(), foreground };
  hubRun = run;
  run.promise = (async () => {
    try {
      const skills = await listHubSkills();
      skills.sort(compareSkills);
      hubSkills.set(skills);
      hubLoaded.set(true);
      if (run.foreground) hubError.set("");
    } catch (error) {
      if (run.foreground) {
        hubError.set(String(error));
      } else {
        console.error("Background hub refresh failed:", error);
      }
    } finally {
      if (run.foreground) releaseHubLoading();
      hubRun = null;
    }
  })();
  return run.promise;
}

export function applySkillView(view: HubSkillView | null | undefined): void {
  if (!view) return;
  hubSkills.update((current) => {
    const index = current.findIndex((skill) => skill.id === view.id);
    if (index === -1) {
      return [...current, view].sort(compareSkills);
    }
    const next = [...current];
    next[index] = view;
    return next;
  });
}

/** Automatic checks run at most this often; the GitHub API allows 60 anonymous requests an hour. */
const AUTO_SOURCE_CHECK_INTERVAL_MS = 5 * 60 * 1000;

let sourceRun: SharedRun<SourceUpdate[]> | null = null;
let lastFullSourceCheckAt = 0;

/** Check the sources of `ids`, or of every skill; only a manual check shows the checking state. */
export function checkSourceUpdates(
  ids?: string[],
  options: RefreshOptions = {}
): Promise<SourceUpdate[]> {
  const foreground = options.manual === true;
  if (foreground) sourceChecking.set(true);
  if (sourceRun) {
    sourceRun.foreground ||= foreground;
    return sourceRun.promise;
  }
  const run: SharedRun<SourceUpdate[]> = { promise: Promise.resolve([]), foreground };
  sourceRun = run;
  run.promise = (async () => {
    try {
      const result = await checkSourceUpdatesApi(ids ?? null);
      lastSourceCheck.set(result);
      await refreshHub({ manual: run.foreground });
      return result;
    } catch (error) {
      console.error("Failed to check source updates:", error);
      return [];
    } finally {
      if (run.foreground) sourceChecking.set(false);
      sourceRun = null;
    }
  })();
  return run.promise;
}

/**
 * Check every GitHub-sourced skill in one batched call. Automatic triggers (startup, opening
 * the library) are throttled and run in the background; the refresh button passes `force` and
 * `manual`.
 */
export async function checkAllSourceUpdates(
  options: { force?: boolean } & RefreshOptions = {}
): Promise<SourceUpdate[]> {
  if (!get(hubSkills).some((skill) => skill.source.type === "github")) return [];
  const now = Date.now();
  if (!options.force && now - lastFullSourceCheckAt < AUTO_SOURCE_CHECK_INTERVAL_MS) {
    return get(lastSourceCheck);
  }
  lastFullSourceCheckAt = now;
  return checkSourceUpdates(undefined, { manual: options.manual });
}

/** The refresh button: reload the hub, then force a source check; the loading state covers both. */
export async function refreshHubAndSources(): Promise<void> {
  holdHubLoading();
  try {
    await refreshHub({ manual: true });
    await checkAllSourceUpdates({ force: true, manual: true });
  } finally {
    releaseHubLoading();
  }
}

export async function loadMigrationReport(): Promise<void> {
  try {
    migrationReport.set(await migrationStatus());
  } catch (error) {
    console.error("Failed to load migration status:", error);
  }
}
