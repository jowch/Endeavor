import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";
import { JSDOM } from "jsdom";

const bundle = (secret) =>
  buildSync({
    entryPoints: [new URL("../src/bridge.ts", import.meta.url).pathname],
    bundle: true,
    format: "iife",
    globalName: "bridge",
    write: false,
    define: secret === undefined ? {} : { __ENDEAVOR_NONCE__: JSON.stringify(secret) },
  }).outputFiles[0].text;

function page(secret) {
  const { window } = new JSDOM(`<body></body>`, { runScripts: "outside-only" });
  const reached = [];
  window.webkit = { messageHandlers: { ipc: { postMessage: (body) => reached.push(JSON.parse(body)) } } };
  window.eval(`${bundle(secret)};window.bridge = bridge;`);
  return { window, reached, bridge: window.bridge };
}

test("a page script that swaps the IPC handler after load can't take the page's messages or its secret", () => {
  const { window, reached, bridge } = page("s3cret");
  const stolen = [];
  window.webkit.messageHandlers.ipc.postMessage = (body) => stolen.push(body);
  bridge.send({ type: "ready" });
  assert.deepEqual(reached, [{ type: "ready", nonce: "s3cret" }]);
  assert.deepEqual(stolen, []);
});

test("in the app, an event a page script makes isn't the user's", () => {
  const { window, bridge } = page("s3cret");
  assert.equal(bridge.byUser(new window.Event("click")), false);
});

test("outside the app the page's messages go to the host, not over IPC", () => {
  const { reached, bridge } = page("s3cret");
  const hosted = [];
  bridge.setHost((msg) => hosted.push(msg));
  bridge.send({ type: "ready" });
  assert.deepEqual(hosted, [{ type: "ready" }]);
  assert.deepEqual(reached, []);
});
