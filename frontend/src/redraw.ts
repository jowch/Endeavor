// Pluto redraws cells and outputs freely; features that decorate them re-run
// on each redraw (batched to one pass per microtask). Attribute changes of our
// own (data-*) don't count, so decorating never loops.

const hooks: Array<() => void> = [];
let queued = false;

export function onRedraw(hook: () => void): void {
  hooks.push(hook);
  hook();
}

export function watchRedraws(): void {
  new MutationObserver((records) => {
    if (queued || records.every((r) => r.type === "attributes" && r.attributeName?.startsWith("data-"))) return;
    queued = true;
    queueMicrotask(() => {
      queued = false;
      hooks.forEach((hook) => hook());
    });
  }).observe(document.body, { childList: true, subtree: true });
}
