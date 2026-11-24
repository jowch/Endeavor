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
import { initTheme, initTokens } from "./theme";
import { onMarimo } from "./engine";
import { initOutOfStep } from "./outofstep";
import { watchRedraws } from "./redraw";

function initOnMarimo() {
  initState();
  initTokens();
  initQuote();
  initAskBox();
  initAnnotate();
  initDebug();
  send({ type: "ready" });
}

function init() {
  if (onMarimo()) return initOnMarimo();
  initOutOfStep();
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
  send({ type: "ready" });
}

document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
