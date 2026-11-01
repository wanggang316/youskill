import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { createFavorites, isMissingSkillError, filterFavorites } = await server.ssrLoadModule(
  "/src/lib/stores/favorites.ts"
);
const { createMarketplace } = await server.ssrLoadModule("/src/lib/stores/market.ts");

const skill = (id, name = "review") => ({
  id,
  skill_id: id,
  name,
  source: "team/tools",
  description: "Reviews code",
});

/** A fake backend that keeps records like the Rust store: newest first, removals hidden. */
function setup() {
  let records = [];
  let clock = 0;
  const calls = [];
  const favorites = createFavorites({
    list: async () => records.filter((item) => item.favorited),
    set: async (skillId, favorited, snapshot) => {
      calls.push({ skillId, favorited, snapshot });
      records = records.filter((item) => item.skillId !== skillId);
      records.unshift({
        skillId,
        favorited,
        updatedAt: ++clock,
        snapshot: snapshot ?? { name: skillId, source: "" },
        pending: true,
      });
      return records.filter((item) => item.favorited);
    },
  });
  let state;
  favorites.state.subscribe((value) => {
    state = value;
  });
  return { favorites, calls, read: () => state };
}

test("toggle favorites a skill with a snapshot and toggles it back", async () => {
  const { favorites, calls, read } = setup();
  await favorites.load();
  await favorites.toggle(skill("42"));
  assert.deepEqual(calls[0], {
    skillId: "42",
    favorited: true,
    snapshot: { name: "review", source: "team/tools", description: "Reviews code" },
  });
  assert.equal(favorites.isFavorite("42"), true);
  assert.deepEqual(
    read().items.map((item) => item.skillId),
    ["42"]
  );

  await favorites.toggle(skill("42"));
  assert.equal(calls[1].favorited, false);
  assert.equal(favorites.isFavorite("42"), false);
  assert.deepEqual(read().items, []);
});

test("list and detail share one favorite state", async () => {
  const { favorites } = setup();
  await favorites.load();
  // Favorite from the list, then remove from the detail page by id.
  await favorites.toggle(skill("7"));
  await favorites.remove("7");
  assert.equal(favorites.isFavorite("7"), false);
});

test("a second toggle while one runs is ignored", async () => {
  const { favorites, calls } = setup();
  await favorites.load();
  await Promise.all([favorites.toggle(skill("1")), favorites.toggle(skill("1"))]);
  assert.equal(calls.length, 1);
  assert.equal(favorites.isFavorite("1"), true);
});

test("a failed change keeps the previous state and shows the error", async () => {
  const favorites = createFavorites({
    list: async () => [],
    set: async () => {
      throw new Error("Disk full");
    },
  });
  let state;
  favorites.state.subscribe((value) => (state = value));
  await favorites.load();
  await favorites.toggle(skill("1"));
  assert.equal(favorites.isFavorite("1"), false);
  assert.match(state.error, /Disk full/);
});

test("an unavailable favorite keeps its snapshot and can be removed", async () => {
  const { favorites, read } = setup();
  await favorites.load();
  await favorites.toggle(skill("9", "gone"));
  favorites.markUnavailable("9");
  assert.deepEqual(read().unavailable, ["9"]);
  assert.equal(read().items[0].snapshot.name, "gone");
  await favorites.remove("9");
  assert.deepEqual(read().items, []);
  assert.deepEqual(read().unavailable, []);
});

test("only a not-found error marks a skill as unavailable", () => {
  assert.equal(
    isMissingSkillError(
      "Catalog request failed: HTTP status client error (404 Not Found) for url (https://api/x)"
    ),
    true
  );
  assert.equal(isMissingSkillError("Catalog request failed: 500 Internal Server Error"), false);
  assert.equal(isMissingSkillError("Network error for /api/public/skills/404"), false);
});

test("the Favorites view filters by name, source and description", () => {
  const items = [
    { skillId: "1", snapshot: { name: "review", source: "team/tools", description: "Code" } },
    { skillId: "2", snapshot: { name: "deploy", source: "ops/kit", description: null } },
  ];
  assert.deepEqual(
    filterFavorites(items, "OPS").map((item) => item.skillId),
    ["2"]
  );
  assert.deepEqual(
    filterFavorites(items, "code").map((item) => item.skillId),
    ["1"]
  );
  assert.equal(filterFavorites(items, "  ").length, 2);
});

test("the Favorites view does not fetch and keeps the browse selection", async () => {
  const calls = [];
  const market = createMarketplace({
    fetchSkills: async (params) => {
      calls.push(params);
      return { skills: [skill("1")], total: 1, has_more: false };
    },
    fetchCategories: async () => [{ id: 7, name: "Development", skill_count: 5 }],
    fetchRepositories: async () => [],
  });
  let state;
  market.state.subscribe((value) => (state = value));
  await market.initialize();
  await market.selectCategory(7);
  const count = calls.length;

  await market.showFavorites();
  assert.equal(state.view, "favorites");
  assert.equal(state.query, "");
  await market.search("rev");
  assert.equal(calls.length, count);
  assert.equal(state.query, "rev");

  await market.selectCategory(7);
  assert.equal(state.view, "browse");
  assert.equal(state.categoryId, 7);
  assert.equal(calls.at(-1).tagId, 7);
});
