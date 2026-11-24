const RESUMED_ON_OPEN = "You have reconnected to an existing session.";

function dismissResumedBanners(root: ParentNode) {
  for (const remove of root.querySelectorAll<HTMLElement>("[data-testid=remove-banner-button]")) {
    const banner = remove.parentElement?.parentElement;
    if (banner?.textContent?.includes(RESUMED_ON_OPEN)) remove.click();
  }
}

export function initDismissResumedBanner() {
  dismissResumedBanners(document);
  new MutationObserver(() => dismissResumedBanners(document)).observe(document.body, { childList: true, subtree: true });
}
