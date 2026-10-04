// Endeavor's script for the notebook page, injected by the app into Pluto's
// page (src/annotate.rs embeds the built dist/page.js).

import { initActions } from "./actions";
import { initAnnotate } from "./annotate";
import { initAskBox } from "./askbox";
import { initAsking } from "./asking";
import { initCardKey } from "./cardkey";
import { send } from "./bridge";
import { initCells } from "./cells";
import { initDebug } from "./debug";
import { initDiffs } from "./diff";
import { initDrawer } from "./drawer";
import { initPrompt } from "./prompt";
import { initQuote } from "./quote";
import { initReadonly } from "./readonly";
import { initErrors } from "./errors";
import { initFolded } from "./folded";
import { initRail } from "./rail";
import { initRunGuard } from "./runguard";
import { initReveal } from "./reveal";
import { initSafe } from "./safe";
import { initState } from "./state";
import { initTheme } from "./theme";
import { watchRedraws } from "./redraw";

function init() {
  // First: other features read `context.agent` from their own "context"
  // handlers, registered after this one so it has already updated it.
  initState();
  initTheme();
  initQuote();
  initAskBox();
  initAnnotate();
  initCells();
  initFolded();
  initAsking();
  initDiffs();
  initPrompt();
  initErrors();
  initRail();
  initReveal();
  initRunGuard();
  initCardKey();
  initActions();
  initDrawer();
  initSafe();
  initReadonly();
  initDebug();
  watchRedraws();
  // Ask for the current state: this page may have loaded after it last changed.
  send({ type: "ready" });
}

document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
