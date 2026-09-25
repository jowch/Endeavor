// Pluto redraws cells and outputs freely; features that decorate them re-run
// on each redraw (batched to one pass per microtask). Our own changes don't
// count, so decorating never loops: data-* attributes, and anything inside an
// element marked data-endeavor-ui (e.g. the rail, which redraws its marks).

const hooks: Array<() => void> = [];
let queued = false;

export function onRedraw(hook: () => void): void {
  hooks.push(hook);
  hook();
}

export function watchRedraws(): void {
  new MutationObserver((records) => {
    const ours = (r: MutationRecord) =>
      (r.type === "attributes" && !!r.attributeName?.startsWith("data-")) ||
      (r.target instanceof Element && !!r.target.closest("[data-endeavor-ui]"));
    if (queued || records.every(ours)) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      hooks.forEach((hook) => hook());
    });
  }).observe(document.body, { childList: true, subtree: true });
}
