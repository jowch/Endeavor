// The page's part of a debug build's state dump (src/debug_state.rs): what the
// page shows that the app can't see from its side.

import { on, send } from "./bridge";

export function initDebug(): void {
  on("debug", () => {
    const annotating = document.body.classList.contains("annotating");
    const drawer = document.documentElement.dataset.endeavorDrawer;
    send({
      type: "debug",
      point: annotating,
      picked: [...document.querySelectorAll<HTMLElement>("pluto-cell.annotate-picked")].map((c) => c.id),
      box: !!document.querySelector("#annotate-box.shown"),
      point_status: annotating ? (document.querySelector("#annotate-bar .status")?.textContent ?? "") : "",
      comment: document.querySelector<HTMLTextAreaElement>("#annotate-bar textarea")?.value ?? "",
      drawer: drawer === "docs" || drawer === "status" ? drawer : null,
      callout: !!document.querySelector("#endeavor-safe.shown"),
      reply: document.querySelector("#endeavor-reply") ? "prompt" : document.querySelector("#endeavor-reply-pill") ? "pill" : null,
      // Recorded by the debug build's own script, which wraps `alert`.
      alerts: (window as { __endeavorAlerts?: string[] }).__endeavorAlerts ?? null,
    });
  });
}
