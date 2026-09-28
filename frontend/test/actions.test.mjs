// The feedback sheet sends through Pluto's own form and reports what Pluto says (src/actions.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSync } from "esbuild";
import { JSDOM } from "jsdom";

const { outputFiles } = buildSync({
  entryPoints: [new URL("../src/actions.ts", import.meta.url).pathname],
  bundle: true,
  format: "iife",
  globalName: "actions",
  write: false,
});

/** A page with Pluto's footer form, whose submit handler does what Pluto's Feedback.js does. */
function page(answer) {
  const dom = new JSDOM(`<body><form id="feedback"><input id="opinion"></form></body>`, { runScripts: "outside-only" });
  const { window } = dom;
  const seen = {};
  window.alert = () => assert.fail("the page's own alert() is not used while sending");
  window.prompt = () => assert.fail("the page's own prompt() is not used while sending");
  const originals = { alert: window.alert, prompt: window.prompt };
  window.document.querySelector("form#feedback").addEventListener("submit", (e) => {
    seen.email = window.prompt("Would you like us to contact you?");
    e.preventDefault();
    seen.opinion = new window.FormData(e.target).get("opinion") ?? window.document.querySelector("#opinion").value;
    if (answer !== null) setTimeout(() => window.alert(answer), 5);
  });
  window.eval(`${outputFiles[0].text};window.actions = actions;`);
  return { window, seen, originals, actions: window.actions };
}

test("Pluto's answer comes back to the sheet, and the page's own dialogs are put back", async () => {
  const { window, seen, originals, actions } = page("Submitted. Thank you for your feedback! 💕");
  const said = await actions.submitFeedback("The docs panel is great", "a@b.org", 1000);
  assert.equal(said, "Submitted. Thank you for your feedback! 💕");
  assert.deepEqual(seen, { email: "a@b.org", opinion: "The docs panel is great" });
  assert.equal(window.alert, originals.alert);
  assert.equal(window.prompt, originals.prompt);
  assert.deepEqual({ ...actions.feedbackOutcome(said) }, { sent: true, title: "Sent to Pluto's developers", body: "Submitted. Thank you for your feedback! 💕" });
});

test("a failure says so in Pluto's words", async () => {
  const failure = "Whoops, failed to send feedback 😢\nWe would really like to hear from you!\n\nTypeError: Load failed";
  const { actions } = page(failure);
  const said = await actions.submitFeedback("Crashes on open", "", 1000);
  assert.deepEqual({ ...actions.feedbackOutcome(said) }, { sent: false, title: "Pluto couldn't send it", body: failure });
});

test("no answer is not reported as sent", async () => {
  const { window, originals, actions } = page(null);
  const said = await actions.submitFeedback("Hello there", "", 20);
  assert.equal(said, null);
  assert.equal(window.alert, originals.alert);
  assert.equal(actions.feedbackOutcome(said).sent, false);
});
