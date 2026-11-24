import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";

const banner = (title, description) =>
  `<div class="banner"><div class="flex"><span>${title}</span><button data-testid="remove-banner-button"></button></div><div><span>${description}</span><button>Restart</button></div></div>`;

test("marimo's page dismisses its resumed-session banner, which every open shows, and keeps the others", async () => {
  const dom = new JSDOM(`<body><div id="banners">${banner("Reconnected", "You have reconnected to an existing session.")}</div></body>`, {
    url: "http://localhost/marimo/?file=/w/fit.py",
    runScripts: "outside-only",
    pretendToBeVisual: true,
  });
  const { window } = dom;
  window.ipc = { postMessage: () => {} };
  const clicked = [];
  window.document.addEventListener("click", (e) => clicked.push(e.target.closest(".banner").textContent), true);
  window.eval(readFileSync(new URL("../dist/page.js", import.meta.url), "utf8"));
  await new Promise((done) => (window.document.readyState === "loading" ? window.addEventListener("DOMContentLoaded", done) : done()));
  assert.equal(clicked.length, 1);
  assert.match(clicked[0], /existing session/);

  const banners = window.document.getElementById("banners");
  banners.insertAdjacentHTML("beforeend", banner("Kernel stopped", "The kernel stopped."));
  banners.insertAdjacentHTML("beforeend", banner("Reconnected", "You have reconnected to an existing session."));
  await new Promise((done) => setTimeout(done, 0));
  assert.equal(clicked.length, 3, "the first banner is still there: marimo's own click handler takes it away");
  assert.ok(clicked.every((text) => text.includes("existing session")), clicked.join(" | "));
});
