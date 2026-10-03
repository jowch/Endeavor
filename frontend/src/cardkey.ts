// ⏎ in the notebook answers the chat's waiting card, as ⏎ in an empty message
// box does (docs/ui-spec.md, "Approval card", Keys). Only while nothing in the
// page has the keyboard: in a cell's editor, a field or on a button, ⏎ is the
// page's own, so typing in a cell while a card waits never answers it.

import { byUser, on, send } from "./bridge";
import { notebookId } from "./state";

/** The waiting card the app last told the page about (the app's own id for it). */
let card: number | null = null;

/** Nothing in the page has the keyboard: ⏎ would do nothing of its own. */
function nothingFocused(): boolean {
  const el = document.activeElement;
  return !el || el === document.body || el === document.documentElement;
}

function onKey(e: KeyboardEvent) {
  if (card === null || e.key !== "Enter" || e.repeat || e.isComposing || !byUser(e)) return;
  if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  if (!nothingFocused() || document.body.classList.contains("annotating")) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  send({ type: "answer_card", notebook: notebookId(), card });
}

export function initCardKey(): void {
  on("context", (msg) => {
    card = msg.card ?? null;
  });
  window.addEventListener("keydown", onKey, true);
}
