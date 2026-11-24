#!/usr/bin/env node
"use strict";

const readline = require("readline");

let sessions = 0;

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
}

function say(sessionId, text) {
  send({ method: "session/update", params: { sessionId, update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } } } });
}

let calls = 0;

function showCall(sessionId, lines) {
  calls += 1;
  const toolCallId = `stand-in-call-${calls}`;
  const command = `seq 1 ${lines}`;
  const output = Array.from({ length: lines }, (_, i) => `line ${i + 1} of the stand-in's made-up output`).join("\n");
  const update = (u) => send({ method: "session/update", params: { sessionId, update: u } });
  update({ sessionUpdate: "tool_call", toolCallId, title: command, kind: "execute", status: "in_progress", rawInput: { command, description: "Made-up output" } });
  update({ sessionUpdate: "tool_call_update", toolCallId, status: "completed", rawOutput: output, content: [{ type: "content", content: { type: "text", text: output } }] });
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
      const tool = /\bTOOL (\d+)/.exec(text);
      if (tool) showCall(params.sessionId, Number(tool[1]));
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
  if (message.id === undefined || !message.method) return;
  send({ id: message.id, result: answer(message.method, message.params || {}) });
});
