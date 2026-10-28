import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { buildMarketHref, getAppLocation } = await server.ssrLoadModule(
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
