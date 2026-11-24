export const PLUTO_OUT_OF_STEP = "Oopsie!! please refresh your browser and everything will be alright!";
export const RELOADED_AT = "endeavor-out-of-step-reloaded-at";
export const RELOAD_GAP_MS = 20_000;

function lastReload(): number {
  try {
    return Number(sessionStorage.getItem(RELOADED_AT)) || 0;
  } catch {
    return 0;
  }
}

function noteReload(at: number): void {
  try {
    sessionStorage.setItem(RELOADED_AT, String(at));
  } catch {}
}

let reloadDue = false;

function reloadOutOfStepPage(): void {
  if (reloadDue) return;
  reloadDue = true;
  const wait = Math.max(0, lastReload() + RELOAD_GAP_MS - Date.now());
  setTimeout(() => {
    noteReload(Date.now());
    location.reload();
  }, wait);
}

export function initOutOfStep(): void {
  const shown = window.alert;
  window.alert = function (message?: unknown) {
    if (message === PLUTO_OUT_OF_STEP) return reloadOutOfStepPage();
    return shown.call(window, message);
  };
}
