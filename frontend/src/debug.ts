// The page's part of a debug build's state dump (src/debug_state.rs): what the
// page shows that the app can't see from its side.

import { pointState } from "./annotate";
import { on, send } from "./bridge";

export function initDebug(): void {
  on("debug", () => {
    const annotating = document.body.classList.contains("annotating");
    const drawer = document.documentElement.dataset.endeavorDrawer;
    const point = pointState();
    send({
      type: "debug",
      point: annotating,
      picked: point.picked,
      picks: point.picks,
      box: point.box,
      point_status: point.status,
      comment: point.comment,
      drawer: drawer === "docs" || drawer === "status" ? drawer : null,
      callout: !!document.querySelector("#endeavor-safe.shown"),
      reply: document.querySelector("#endeavor-reply") ? "prompt" : document.querySelector("#endeavor-reply-pill") ? "pill" : null,
      // Recorded by the debug build's own script, which wraps `alert`.
      alerts: (window as { __endeavorAlerts?: string[] }).__endeavorAlerts ?? null,
    });
  });
}
