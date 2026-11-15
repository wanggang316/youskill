import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { buildMarketHref, buildScopeHref, getAppLocation } = await server.ssrLoadModule(
  "/src/lib/navigation/app-shell.ts"
);

test("market skill detail stays on the market page", () => {
  assert.equal(buildMarketHref(), "/market");
  const href = buildMarketHref("owner/repo skill");
  assert.equal(href, "/market?skill=owner%2Frepo%20skill");
  const url = new URL(href, "http://app");
  assert.equal(url.searchParams.get("skill"), "owner/repo skill");
  assert.equal(getAppLocation(url).activeKey, "market");
});

test("project item detail stays on the projects page", () => {
  const project = { scope: "project", projectPath: "/work/my app" };
  const skillHref = buildScopeHref(project, { skill: "owner/skill" });
  assert.equal(skillHref, "/projects?project=%2Fwork%2Fmy%20app&skill=owner%2Fskill");
  const skill = getAppLocation(new URL(skillHref, "http://app"));
  assert.equal(skill.activeKey, "projects");
  assert.deepEqual(skill.scope, project);
  assert.equal(skill.skill, "owner/skill");

  const user = { scope: "user", projectPath: null };
  const fileHref = buildScopeHref(user, { file: "/home/me/.claude/CLAUDE.md" });
  assert.equal(fileHref, "/projects?scope=user&file=%2Fhome%2Fme%2F.claude%2FCLAUDE.md");
  const file = getAppLocation(new URL(fileHref, "http://app"));
  assert.deepEqual(file.scope, user);
  assert.equal(file.instructionFile, "/home/me/.claude/CLAUDE.md");

  const templateHref = buildScopeHref(project, { instruction: "tpl-1" });
  assert.equal(templateHref, "/projects?project=%2Fwork%2Fmy%20app&instruction=tpl-1");
  const template = getAppLocation(new URL(templateHref, "http://app"));
  assert.equal(template.instruction, "tpl-1");
});

test("project href without an item keeps the old form", () => {
  assert.equal(buildScopeHref(), "/projects");
  assert.equal(buildScopeHref({ scope: "user", projectPath: null }), "/projects?scope=user");
  assert.equal(
    buildScopeHref({ scope: "project", projectPath: "/a b" }),
    "/projects?project=%2Fa%20b"
  );
});
