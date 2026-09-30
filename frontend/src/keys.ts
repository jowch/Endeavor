// The page's own shortcuts use ⌘ on macOS and Ctrl elsewhere, as the app's do.

export const mac = /Mac/.test(navigator.platform);

/** ⌘ on macOS, Ctrl elsewhere, held for this key event. */
export function modHeld(e: KeyboardEvent): boolean {
  return mac ? e.metaKey : e.ctrlKey;
}

/** A shortcut's hint: "⌘E" on macOS, "Ctrl+E" elsewhere. */
export function shortcut(key: string): string {
  return mac ? `⌘${key}` : `Ctrl+${key}`;
}
