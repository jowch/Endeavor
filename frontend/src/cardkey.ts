import { byUser, on, send } from "./bridge";
import { notebookId } from "./state";

let card: number | null = null;

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
