import assert from "node:assert/strict";
import { after, test } from "node:test";
import { get } from "svelte/store";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { APP_UPDATE_CHECK_INTERVAL_MS, createAppUpdater } = await server.ssrLoadModule(
  "/src/lib/stores/updater.ts"
);

const MINUTE = 60 * 1000;
const release = (version) => ({ version, body: `Notes ${version}`, downloadAndInstall: () => {} });

function setup(results = []) {
  let now = 0;
  const checks = [];
  const updater = createAppUpdater({
    check: async () => {
      checks.push(now);
      const result = results.shift();
      if (result instanceof Error) throw result;
      return result ?? null;
    },
    relaunch: async () => {},
    now: () => now,
  });
  let focusListener = null;
  let timerCallback = null;
  const removed = { focus: 0, timer: 0 };
  const start = () =>
    updater.startAutomaticUpdateChecks({
      onFocus: async (listener) => {
        focusListener = listener;
        return () => {
          removed.focus += 1;
        };
      },
      setInterval: (callback, ms) => {
        assert.equal(ms, APP_UPDATE_CHECK_INTERVAL_MS);
        timerCallback = callback;
        return "timer";
      },
      clearInterval: (id) => {
        assert.equal(id, "timer");
        removed.timer += 1;
      },
    });
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  return {
    updater,
    checks,
    removed,
    start,
    advance: (minutes) => {
      now += minutes * MINUTE;
    },
    focus: async () => {
      focusListener();
      await settle();
    },
    tick: async () => {
      timerCallback();
      await settle();
    },
    settle,
  };
}

test("focus checks only after 60 minutes since the last check", async () => {
  const app = setup();
  app.start();
  await app.updater.ensureUpdateChecked();
  assert.equal(app.checks.length, 1);

  app.advance(59);
  await app.focus();
  assert.equal(app.checks.length, 1);

  app.advance(1);
  await app.focus();
  assert.equal(app.checks.length, 2);
});

test("hourly timer uses the interval rule and skips after a recent focus check", async () => {
  const app = setup();
  app.start();
  await app.updater.ensureUpdateChecked();

  app.advance(60);
  await app.tick();
  assert.equal(app.checks.length, 2);

  app.advance(40);
  await app.focus();
  assert.equal(app.checks.length, 2);
  app.advance(30);
  await app.focus();
  assert.equal(app.checks.length, 3);

  app.advance(30);
  await app.tick();
  assert.equal(app.checks.length, 3);
});

test("a failed check sets the last check time", async () => {
  const app = setup([new Error("offline")]);
  app.start();
  await assert.rejects(app.updater.ensureUpdateChecked(), /offline/);
  assert.equal(get(app.updater.updaterState).error, "offline");

  app.advance(30);
  await app.focus();
  assert.equal(app.checks.length, 1);

  app.advance(30);
  await app.focus();
  assert.equal(app.checks.length, 2);
});

test("a manual check restarts the interval", async () => {
  const app = setup();
  app.start();
  await app.updater.ensureUpdateChecked();

  app.advance(50);
  app.updater.openUpdateDialog();
  await app.settle();
  app.updater.updateDialogOpen.set(false);
  assert.equal(app.checks.length, 2);

  app.advance(10);
  await app.focus();
  await app.tick();
  assert.equal(app.checks.length, 2);

  app.advance(50);
  await app.focus();
  assert.equal(app.checks.length, 3);
});

test("no automatic check while the update dialog is open or an install runs", async () => {
  const app = setup();
  app.start();
  await app.updater.ensureUpdateChecked();
  app.advance(60);

  app.updater.updateDialogOpen.set(true);
  await app.focus();
  await app.tick();
  assert.equal(app.checks.length, 1);
  app.updater.updateDialogOpen.set(false);

  app.updater.updaterState.update((state) => ({ ...state, installing: true }));
  await app.focus();
  await app.tick();
  assert.equal(app.checks.length, 1);

  app.updater.updaterState.update((state) => ({ ...state, installing: false }));
  await app.focus();
  assert.equal(app.checks.length, 2);
});

test("automatic checks are silent and a later release replaces an earlier one", async () => {
  const app = setup([null, release("1.1.0"), release("1.2.0")]);
  app.start();
  await app.updater.ensureUpdateChecked();

  app.advance(60);
  await app.tick();
  let state = get(app.updater.updaterState);
  assert.equal(state.hasUpdate, true);
  assert.equal(state.latestVersion, "1.1.0");
  assert.equal(get(app.updater.updateDialogOpen), false);

  app.advance(60);
  await app.focus();
  state = get(app.updater.updaterState);
  assert.equal(state.latestVersion, "1.2.0");
  assert.equal(state.releaseNotes, "Notes 1.2.0");
  assert.equal(get(app.updater.updateDialogOpen), false);
});

test("stopping removes the timer and the focus listener", async () => {
  const app = setup();
  const stop = app.start();
  await app.settle();
  stop();
  assert.deepEqual(app.removed, { focus: 1, timer: 1 });
});

test("a focus listener registered after stop is removed at once", async () => {
  const app = setup();
  let removed = 0;
  let resolveListen;
  const stop = app.updater.startAutomaticUpdateChecks({
    onFocus: () =>
      new Promise((resolve) => {
        resolveListen = resolve;
      }),
    setInterval: () => "timer",
    clearInterval: () => {},
  });
  stop();
  resolveListen(() => {
    removed += 1;
  });
  await app.settle();
  assert.equal(removed, 1);
});
