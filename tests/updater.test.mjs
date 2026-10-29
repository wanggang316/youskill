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
const { createAppUpdater } = await server.ssrLoadModule("/src/lib/stores/updater.ts");

const MINUTE = 60 * 1000;
const release = (version) => ({ version, body: `Notes ${version}`, downloadAndInstall: () => {} });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Fake clock and timers; `check` results are taken in order, with an optional delay in minutes. */
function setup(results = []) {
  let now = 0;
  let nextTimerId = 1;
  const timers = new Map();
  const checks = [];
  let checkDelay = 0;
  const updater = createAppUpdater({
    check: async () => {
      checks.push(now / MINUTE);
      const result = results.shift();
      if (checkDelay > 0) {
        const delay = checkDelay;
        checkDelay = 0;
        await new Promise((resolve) => schedule(resolve, delay * MINUTE));
      }
      if (result instanceof Error) throw result;
      return result ?? null;
    },
    relaunch: async () => {},
    now: () => now,
  });
  const schedule = (callback, ms) => {
    const id = nextTimerId++;
    timers.set(id, { at: now + ms, callback });
    return id;
  };
  let focusListener = null;
  const removed = { focus: 0 };
  const start = () =>
    updater.startAutomaticUpdateChecks({
      onFocus: async (listener) => {
        focusListener = listener;
        return () => {
          removed.focus += 1;
        };
      },
      setTimeout: schedule,
      clearTimeout: (id) => timers.delete(id),
    });
  const advance = async (minutes) => {
    const target = now + minutes * MINUTE;
    for (;;) {
      const due = [...timers].filter(([, t]) => t.at <= target).sort((a, b) => a[1].at - b[1].at);
      if (due.length === 0) break;
      const [id, timer] = due[0];
      timers.delete(id);
      now = timer.at;
      timer.callback();
      await settle();
    }
    now = target;
    await settle();
  };
  return {
    updater,
    checks,
    removed,
    timers,
    start,
    advance,
    delayNextCheck: (minutes) => {
      checkDelay = minutes;
    },
    focus: async () => {
      focusListener();
      await settle();
    },
  };
}

/** Like the layout: the startup check starts, then the automatic checks start. */
async function startApp(app) {
  const startup = app.updater.ensureUpdateChecked().catch(() => {});
  const stop = app.start();
  await settle();
  await startup;
  return stop;
}

test("focus checks only after 60 minutes since the last check", async () => {
  const app = setup();
  app.start();
  await app.updater.ensureUpdateChecked();
  assert.deepEqual(app.checks, [0]);
  // Move time without firing the timer, to isolate the focus trigger.
  app.timers.clear();

  await app.advance(59);
  await app.focus();
  assert.deepEqual(app.checks, [0]);

  await app.advance(1);
  await app.focus();
  assert.deepEqual(app.checks, [0, 60]);
});

test("the timer checks 60 minutes after a slow startup check ends", async () => {
  const app = setup();
  app.delayNextCheck(5);
  const startup = app.updater.ensureUpdateChecked();
  app.start();
  await app.advance(5);
  await startup;
  assert.deepEqual(app.checks, [0]);

  await app.advance(59);
  assert.deepEqual(app.checks, [0]);
  await app.advance(1);
  assert.deepEqual(app.checks, [0, 65]);

  await app.advance(60);
  assert.deepEqual(app.checks, [0, 65, 125]);
});

test("a recent focus check moves the next timer check", async () => {
  const app = setup();
  await startApp(app);

  await app.advance(60);
  assert.deepEqual(app.checks, [0, 60]);

  await app.advance(30);
  await app.focus();
  assert.deepEqual(app.checks, [0, 60]);
  await app.advance(30);
  assert.deepEqual(app.checks, [0, 60, 120]);
  await app.focus();
  assert.deepEqual(app.checks, [0, 60, 120]);
});

test("a failed check sets the last check time", async () => {
  const app = setup([null, new Error("offline")]);
  await startApp(app);

  await app.advance(60);
  assert.deepEqual(app.checks, [0, 60]);
  assert.equal(get(app.updater.updaterState).error, "offline");

  await app.advance(30);
  await app.focus();
  assert.deepEqual(app.checks, [0, 60]);

  await app.advance(30);
  assert.deepEqual(app.checks, [0, 60, 120]);
});

test("a manual check restarts the interval", async () => {
  const app = setup();
  await startApp(app);

  await app.advance(50);
  app.updater.openUpdateDialog();
  await settle();
  app.updater.updateDialogOpen.set(false);
  assert.deepEqual(app.checks, [0, 50]);

  await app.advance(10);
  await app.focus();
  assert.deepEqual(app.checks, [0, 50]);

  await app.advance(50);
  assert.deepEqual(app.checks, [0, 50, 110]);
});

test("no automatic check while the update dialog is open or an install runs", async () => {
  const app = setup();
  await startApp(app);

  app.updater.updateDialogOpen.set(true);
  await app.advance(60);
  await app.focus();
  assert.deepEqual(app.checks, [0]);
  app.updater.updateDialogOpen.set(false);

  app.updater.updaterState.update((state) => ({ ...state, installing: true }));
  await app.focus();
  await app.advance(60);
  assert.deepEqual(app.checks, [0]);

  app.updater.updaterState.update((state) => ({ ...state, installing: false }));
  await app.focus();
  assert.deepEqual(app.checks, [0, 120]);
});

test("automatic checks are silent and a later release replaces an earlier one", async () => {
  const app = setup([null, release("1.1.0"), release("1.2.0")]);
  await startApp(app);

  await app.advance(60);
  let state = get(app.updater.updaterState);
  assert.equal(state.hasUpdate, true);
  assert.equal(state.latestVersion, "1.1.0");
  assert.equal(get(app.updater.updateDialogOpen), false);

  await app.advance(60);
  state = get(app.updater.updaterState);
  assert.equal(state.latestVersion, "1.2.0");
  assert.equal(state.releaseNotes, "Notes 1.2.0");
  assert.equal(get(app.updater.updateDialogOpen), false);
});

test("stopping removes the timer and the focus listener", async () => {
  const app = setup();
  const stop = await startApp(app);
  assert.equal(app.timers.size, 1);
  stop();
  assert.equal(app.timers.size, 0);
  assert.equal(app.removed.focus, 1);

  // A check after stop does not start the timer again.
  await app.updater.ensureUpdateChecked(true);
  assert.equal(app.timers.size, 0);
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
    setTimeout: () => "timer",
    clearTimeout: () => {},
  });
  stop();
  resolveListen(() => {
    removed += 1;
  });
  await settle();
  assert.equal(removed, 1);
});
