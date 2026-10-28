import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { createMarketplace } = await server.ssrLoadModule("/src/lib/stores/market.ts");
const result = (ids, more = false) => ({
  skills: ids.map((id) => ({ id, name: "review", source: "team/tools" })),
  total: more ? 40 : ids.length,
  has_more: more,
});
function setup(fetchSkills = async () => result([]), repos = [{ id: 1, name: "team/tools" }]) {
  const calls = [];
  const market = createMarketplace({
    fetchSkills: (params) => {
      calls.push(params);
      return fetchSkills(params);
    },
    fetchCategories: async () => [{ id: 7, name: "Development", skill_count: 5 }],
    fetchRepositories: async () => repos,
  });
  let state;
  market.state.subscribe((value) => {
    state = value;
  });
  return { market, calls, read: () => state };
}

test("tabs remember selections and clear search without combining filters", async () => {
  const { market, calls, read } = setup();
  await market.initialize();
  await market.selectCategory(7);
  await market.search("hooks");
  assert.equal(calls.at(-1).tagId, 7);
  assert.equal(calls.at(-1).search, "hooks");
  await market.selectTab("repositories");
  assert.equal(calls.at(-1).source, "team/tools");
  assert.equal(calls.at(-1).tagId, undefined);
  assert.equal(read().query, "");
  await market.selectTab("categories");
  assert.equal(read().categoryId, 7);
  assert.equal(calls.at(-1).source, undefined);
});

test("empty hot repositories do not fall back to all skills", async () => {
  const { market, calls, read } = setup(undefined, []);
  await market.initialize();
  const count = calls.length;
  await market.selectTab("repositories");
  assert.equal(calls.length, count);
  assert.deepEqual(read().skills, []);
  assert.equal(read().repository, null);
});

test("a late response cannot replace a newer selection", async () => {
  let resolveOld;
  const { market, read } = setup(({ search }) =>
    search === "old"
      ? new Promise((resolve) => {
          resolveOld = resolve;
        })
      : Promise.resolve(result(["new"]))
  );
  await market.initialize();
  const old = market.search("old");
  await market.selectCategory(7);
  resolveOld(result(["old"]));
  await old;
  assert.equal(read().skills[0].id, "new");
  assert.equal(read().loading, false);
});

test("failed pagination retries the same offset and preserves loaded rows", async () => {
  let fail = true;
  const { market, calls, read } = setup(async ({ skip }) => {
    if (skip && fail) {
      fail = false;
      throw new Error("Offline");
    }
    return result(
      Array.from({ length: 20 }, (_, i) => String(skip + i)),
      skip === 0
    );
  });
  await market.initialize();
  await market.loadMore();
  assert.equal(read().skills.length, 20);
  assert.match(read().error, /Offline/);
  await market.loadMore();
  assert.equal(calls.at(-1).skip, 20);
  assert.equal(read().skills.length, 40);
});

test("re-entering the page retains results, query, sort and scroll", async () => {
  const { market, calls, read } = setup(async () => result(["42"]));
  await market.initialize();
  await market.selectCategory(7);
  await market.search("review");
  await market.sort("star_count");
  market.saveScroll(280);
  const count = calls.length;
  await market.initialize();
  assert.equal(calls.length, count);
  assert.equal(read().query, "review");
  assert.equal(read().sortBy, "star_count");
  assert.equal(read().scrollTop, 280);
  assert.equal(read().categoryId, 7);
  assert.equal(read().skills[0].id, "42");
});

test("a failed category request leaves repository navigation usable", async () => {
  const market = createMarketplace({
    fetchSkills: async () => result(["42"]),
    fetchCategories: async () => {
      throw new Error("Tags unavailable");
    },
    fetchRepositories: async () => [{ id: 1, name: "team/tools", skill_count: 1 }],
  });
  let state;
  market.state.subscribe((value) => {
    state = value;
  });
  await market.initialize();
  assert.match(state.categoriesError, /Tags unavailable/);
  await market.selectTab("repositories");
  assert.equal(state.repository, "team/tools");
  assert.equal(state.skills[0].id, "42");
});

test("typing invalidates old results before the debounce request starts", async () => {
  let resolveOld;
  const { market, read } = setup(({ search }) =>
    search === "old"
      ? new Promise((resolve) => {
          resolveOld = resolve;
        })
      : Promise.resolve(result(["new"]))
  );
  await market.initialize();
  const old = market.search("old");
  market.setSearch("new");
  resolveOld(result(["old"]));
  await old;
  assert.deepEqual(read().skills, []);
  assert.equal(read().loaded, false);
  await market.initialize();
  assert.equal(read().skills[0].id, "new");
});
