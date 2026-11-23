import { pointState } from "./annotate";
import { on, send } from "./bridge";
import { askState } from "./prompt";

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
      reply: askState()?.kind === "selection" ? "prompt" : document.querySelector("#endeavor-reply-pill") ? "pill" : null,
      prompt: askState(),
      alerts: (window as { __endeavorAlerts?: string[] }).__endeavorAlerts ?? null,
      dialogs: [...document.querySelectorAll("dialog[open]")].map((d) => (d.textContent ?? "").trim().replace(/\s+/g, " ")),
    });
  });
}
