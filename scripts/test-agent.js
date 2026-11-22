#!/usr/bin/env node
// test-agent.js: a scripted stand-in for the ACP adapter, for checks on a
// computer that must not reach a real assistant (a cloud VM, CI). A debug
// build runs it in place of the adapter when ENDEAVOR_TEST_AGENT is set to
// this file (docs/testing.md). It answers every prompt with one short line
// and runs no tools, so it never reads, writes or runs anything.
"use strict";

const readline = require("readline");

let sessions = 0;

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
}

function say(sessionId, text) {
  send({ method: "session/update", params: { sessionId, update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } } } });
}

function promptText(prompt) {
  return (prompt || []).filter((block) => block.type === "text").map((block) => block.text).join(" ").trim();
}

function answer(method, params) {
  switch (method) {
    case "initialize":
      return { protocolVersion: 1, agentCapabilities: { loadSession: true }, authMethods: [] };
    case "session/new":
    case "session/fork":
      sessions += 1;
      return { sessionId: `stand-in-${process.pid}-${sessions}` };
    case "session/load":
      return {};
    case "session/list":
      return { sessions: [] };
    case "session/set_config_option":
      return { configOptions: [] };
    case "session/prompt": {
      const text = promptText(params.prompt);
      say(params.sessionId, `Stand-in agent: got "${text.slice(0, 200)}". No real assistant answered.`);
      return { stopReason: "end_turn" };
    }
    default:
      return {};
  }
}

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  // Notifications (session/cancel and the like) and our requests' replies need no answer.
  if (message.id === undefined || !message.method) return;
  send({ id: message.id, result: answer(message.method, message.params || {}) });
});
