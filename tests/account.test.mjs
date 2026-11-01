import assert from "node:assert/strict";
import { after, test } from "node:test";
import { createServer } from "vite";

const server = await createServer({
  configFile: false,
  server: { middlewareMode: true, hmr: false, ws: false, watch: null },
  appType: "custom",
});
after(() => server.close());
const { createAccount } = await server.ssrLoadModule("/src/lib/stores/account.ts");

const octo = { accountId: "12", provider: "github", displayName: "octo" };

/** A fake backend: the test sends account change events by hand. */
function setup({ identity = null, failStart = false, lastError = null } = {}) {
  let emit = () => {};
  const opened = [];
  let signedOut = 0;
  const account = createAccount(
    {
      status: async () => ({ status: { identity }, error: lastError }),
      start: async (provider) => {
        if (failStart) throw new Error("no keychain");
        return `https://api.example.test/api/account/sign-in/${provider}`;
      },
      signOut: async () => ({ identity: null }),
      openUrl: async (url) => opened.push(url),
      listen: async (handler) => {
        emit = handler;
        return () => {};
      },
    },
    () => signedOut++
  );
  let state;
  account.state.subscribe((value) => {
    state = value;
  });
  return {
    account,
    opened,
    read: () => state,
    emit: (event) => emit(event),
    signedOut: () => signedOut,
  };
}

test("sign-in opens the browser and the callback shows the identity", async () => {
  const { account, opened, read, emit } = setup();
  await account.load();
  assert.equal(read().identity, null);

  await account.signIn("apple");
  assert.deepEqual(opened, ["https://api.example.test/api/account/sign-in/apple"]);
  assert.equal(read().signingIn, "apple");

  emit({ status: { identity: octo }, error: null });
  assert.deepEqual(read().identity, octo);
  assert.equal(read().signingIn, null);
  assert.equal(read().error, "");
});

test("a cancelled sign-in returns to signed out without a message", async () => {
  const { account, read, emit } = setup();
  await account.load();
  await account.signIn("github");
  emit({ status: { identity: null }, error: null });
  assert.equal(read().identity, null);
  assert.equal(read().signingIn, null);
  assert.equal(read().error, "");
});

test("a failed sign-in shows the message", async () => {
  const { account, read, emit } = setup();
  await account.load();
  await account.signIn("github");
  emit({ status: { identity: null }, error: "Sign-in failed (server_error). Try again." });
  assert.equal(read().error, "Sign-in failed (server_error). Try again.");

  const failing = setup({ failStart: true });
  await failing.account.load();
  await failing.account.signIn("github");
  assert.equal(failing.read().signingIn, null);
  assert.equal(failing.read().error, "no keychain");
});

test("sign-out shows signed out and reloads the cleared favorites", async () => {
  const { account, read, signedOut } = setup({ identity: octo });
  await account.load();
  assert.deepEqual(read().identity, octo);
  await account.signOut();
  assert.equal(read().identity, null);
  assert.equal(read().signingOut, false);
  assert.equal(signedOut(), 1);
});

test("a session the server revoked shows signed out", async () => {
  const { account, read, emit } = setup({ identity: octo });
  await account.load();
  emit({ status: { identity: null }, error: null });
  assert.equal(read().identity, null);
});

test("a failure of a link that started the app shows after the load", async () => {
  const { account, read } = setup({ lastError: "Sign-in failed (server_error). Try again." });
  await account.load();
  assert.equal(read().error, "Sign-in failed (server_error). Try again.");
});
