import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { removeEmptySections } = await server.ssrLoadModule("/src/lib/utils/markdown.ts");

test("removeEmptySections drops changelog categories without entries", () => {
  const notes = [
    "### Breaking Changes",
    "",
    "### Added",
    "",
    "### Fixed",
    "",
    "- Recover empty skill names.",
    "",
    "### Removed",
    "",
  ].join("\n");
  assert.equal(removeEmptySections(notes), "### Fixed\n\n- Recover empty skill names.");
});

test("removeEmptySections drops a parent heading whose sub-sections are all empty", () => {
  const notes = "## 1.0\n\n### Added\n\n## 0.9\n\n### Fixed\n\n- Fix.\n";
  assert.equal(removeEmptySections(notes), "## 0.9\n\n### Fixed\n\n- Fix.");
});

test("removeEmptySections keeps text before the first heading and code blocks", () => {
  const notes = "Intro.\n\n### Changed\n\n```sh\n### not a heading\n```\n\n### Removed\n";
  assert.equal(
    removeEmptySections(notes),
    "Intro.\n\n### Changed\n\n```sh\n### not a heading\n```"
  );
});

test("removeEmptySections returns an empty string when no section has content", () => {
  assert.equal(removeEmptySections("### Added\n\n### Fixed\n"), "");
  assert.equal(removeEmptySections(""), "");
});
