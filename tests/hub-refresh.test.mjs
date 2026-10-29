import assert from "node:assert/strict";
import { after, beforeEach, test } from "node:test";
import { createServer } from "vite";

// Stand-in for the Tauri IPC bridge; each test sets the command handlers it needs.
let handlers = {};
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke: async (command) => {
      const handler = handlers[command];
      if (!handler) throw new Error(`unexpected command: ${command}`);
      return handler();
    },
  },
};

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const hub = await server.ssrLoadModule("/src/lib/stores/hub.ts");
const { get } = await server.ssrLoadModule("svelte/store");

const githubSkill = {
  id: "s1",
  name: "alpha",
  sourceKey: "github:owner/repo",
  source: { type: "github", owner: "owner", repo: "repo" },
};

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const originalConsoleError = console.error;
let logged = [];

beforeEach(() => {
  handlers = {};
  logged = [];
  console.error = (...args) => logged.push(args);
  hub.hubSkills.set([]);
  hub.hubLoaded.set(false);
  hub.hubLoading.set(false);
  hub.hubError.set("");
  hub.sourceChecking.set(false);
});
after(() => {
  console.error = originalConsoleError;
});

test("first load shows the loading state", async () => {
  const list = deferred();
  handlers.list_hub_skills = () => list.promise;
  const run = hub.refreshHub();
  assert.equal(get(hub.hubLoading), true);
  list.resolve([githubSkill]);
  await run;
  assert.equal(get(hub.hubLoading), false);
  assert.equal(get(hub.hubSkills).length, 1);
});

test("background refresh updates the list without the loading state", async () => {
  hub.hubLoaded.set(true);
  const list = deferred();
  handlers.list_hub_skills = () => list.promise;
  const run = hub.refreshHub();
  assert.equal(get(hub.hubLoading), false);
  list.resolve([githubSkill]);
  await run;
  assert.equal(get(hub.hubLoading), false);
  assert.deepEqual(
    get(hub.hubSkills).map((skill) => skill.id),
    ["s1"]
  );
});

test("failed background refresh logs and keeps the page error", async () => {
  hub.hubLoaded.set(true);
  hub.hubError.set("previous error");
  handlers.list_hub_skills = () => Promise.reject("boom");
  await hub.refreshHub();
  assert.equal(get(hub.hubError), "previous error");
  assert.equal(logged.length, 1);
});

test("failed manual refresh shows the error", async () => {
  hub.hubLoaded.set(true);
  handlers.list_hub_skills = () => Promise.reject("boom");
  await hub.refreshHub({ manual: true });
  assert.match(get(hub.hubError), /boom/);
  assert.equal(get(hub.hubLoading), false);
});

test("manual refresh that joins a background run shows the loading state", async () => {
  hub.hubLoaded.set(true);
  const list = deferred();
  handlers.list_hub_skills = () => list.promise;
  const background = hub.refreshHub();
  const manual = hub.refreshHub({ manual: true });
  assert.equal(get(hub.hubLoading), true);
  list.reject("boom");
  await Promise.all([background, manual]);
  assert.equal(get(hub.hubLoading), false);
  assert.match(get(hub.hubError), /boom/);
});

test("background source check does not show the checking state", async () => {
  hub.hubLoaded.set(true);
  hub.hubSkills.set([githubSkill]);
  const check = deferred();
  const update = { id: "s1", updateAvailable: true };
  handlers.check_source_updates = () => check.promise;
  handlers.list_hub_skills = () => [githubSkill];
  const run = hub.checkAllSourceUpdates({ force: true });
  assert.equal(get(hub.sourceChecking), false);
  assert.equal(get(hub.hubLoading), false);
  check.resolve([update]);
  assert.deepEqual(await run, [update]);
  assert.deepEqual(get(hub.lastSourceCheck), [update]);
  assert.equal(get(hub.sourceChecking), false);
});

test("manual source check shows the checking state until the hub refresh ends", async () => {
  hub.hubLoaded.set(true);
  hub.hubSkills.set([githubSkill]);
  const list = deferred();
  handlers.check_source_updates = () => [];
  handlers.list_hub_skills = () => list.promise;
  const run = hub.checkAllSourceUpdates({ force: true, manual: true });
  assert.equal(get(hub.sourceChecking), true);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(get(hub.sourceChecking), true);
  assert.equal(get(hub.hubLoading), true);
  list.resolve([githubSkill]);
  await run;
  assert.equal(get(hub.sourceChecking), false);
  assert.equal(get(hub.hubLoading), false);
});
