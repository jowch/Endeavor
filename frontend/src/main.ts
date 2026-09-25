// Endeavor's script for the notebook page, injected by the app into Pluto's
// page (src/annotate.rs embeds the built dist/page.js).

import { initAnnotate } from "./annotate";

function init() {
  initAnnotate();
}

document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", init) : init();
