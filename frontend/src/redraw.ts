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
