export const mac = /Mac/.test(navigator.platform);

export function modHeld(e: KeyboardEvent): boolean {
  return mac ? e.metaKey : e.ctrlKey;
}

export function shortcut(key: string): string {
  return mac ? `⌘${key}` : `Ctrl+${key}`;
}
