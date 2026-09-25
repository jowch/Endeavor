// Endeavor's script for the notebook page, injected by the app into Pluto's
// page (src/annotate.rs embeds the built dist/page.js).

import { initAnnotate } from "./annotate";
import { send } from "./bridge";
import { initCells } from "./cells";
import { initErrors } from "./errors";
import { initRail } from "./rail";
import { watchRedraws } from "./redraw";

function init() {
  initAnnotate();
  initCells();
  initErrors();
  initRail();
  watchRedraws();
  // Ask for the current state: this page may have loaded after it last changed.
  send({ type: "ready" });
}

document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
