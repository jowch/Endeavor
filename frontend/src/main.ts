// Endeavor's script for the notebook page, injected by the app into Pluto's
// page (src/annotate.rs embeds the built dist/page.js).

import { initActions } from "./actions";
import { initAnnotate } from "./annotate";
import { send } from "./bridge";
import { initCells } from "./cells";
import { initDebug } from "./debug";
import { initDiffs } from "./diff";
import { initDrawer } from "./drawer";
import { initPrompt } from "./prompt";
import { initReadonly } from "./readonly";
import { initErrors } from "./errors";
import { initRail } from "./rail";
import { initReveal } from "./reveal";
import { initSafe } from "./safe";
import { initState } from "./state";
import { initTheme } from "./theme";
import { watchRedraws } from "./redraw";

function init() {
  initTheme();
  initAnnotate();
  initCells();
  initDiffs();
  initPrompt();
  initErrors();
  initRail();
  initReveal();
  initActions();
  initDrawer();
  initSafe();
  initState();
  initReadonly();
  initDebug();
  watchRedraws();
  // Ask for the current state: this page may have loaded after it last changed.
  send({ type: "ready" });
}

document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
