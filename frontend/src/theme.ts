// The notebook's theme. The app sets the webview's light/dark appearance, which
// Pluto's own themes follow (prefers-color-scheme). On top, the "endeavor" theme
// maps Pluto's colour variables to Endeavor's tokens (docs/ui-spec.md) in dark
// mode, and hides Pluto's footer; "pluto" leaves Pluto's look alone. There's no
// Endeavor light theme yet: in light mode both are Pluto's light theme.

import { on } from "./bridge";

const endeavorDark = `
@media (prefers-color-scheme: dark) {
  :root {
    --main-bg-color: #151517;
    --header-bg-color: #151517;
    --footer-bg-color: #151517;
    --rule-color: rgba(255, 255, 255, 0.08);
    --code-background: #1B1B1E;
    --normal-cell-color: rgba(100, 100, 100, 0.18);
    --dark-normal-cell-color: rgba(100, 100, 100, 0.3);
    --code-differs-cell-color: #9A9A9A;
    --selected-cell-color: rgba(143, 170, 216, 0.45);
    --pluto-output-color: #BDBDBD;
    --pluto-output-h-color: #E0E0E0;
    --pluto-output-bg-color: #151517;
    --pluto-runarea-bg-color: #1C1C1F;
    --pluto-logs-bg-color: #1C1C1F;
    --overlay-button-bg: #1C1C1F;
    --input-context-menu-bg-color: #1C1C1F;
    --input-context-menu-border-color: #2A2A2E;
    --cm-selection-background: rgba(143, 170, 216, 0.3);
    --cm-color-editor-text: #D4D4D4;
    --cm-color-keyword: #D98BB5;
    --cm-color-control-operator: #D98BB5;
    --cm-color-literal: #CFA47A;
    --cm-color-symbol: #CFA47A;
    --cm-color-string: #A3C48C;
    --cm-color-function: #9AB6E6;
    --cm-color-builtin: #9AB6E6;
    --cm-color-comment: #5E5E5E;
    --cm-color-line-numbers: #555555;
    /* Live docs */
    --helpbox-bg-color: #1C1C1F;
    --helpbox-header-bg-color: #26262A;
    --helpbox-header-tab-bg-color: #26262A;
    --helpbox-header-color: #ECECEC;
    --helpbox-text-color: #D4D4D4;
    --helpbox-search-bg-color: #151517;
    --helpbox-search-border-color: #3A3A40;
    --helpbox-box-shadow-color: rgba(0, 0, 0, 0.3);
    --docs-binding-bg: #26262A;
  }
}
footer { display: none !important; }
`;

export function initTheme(): void {
  const style = document.createElement("style");
  style.id = "endeavor-theme";
  // After Pluto's stylesheets, so equal-specificity rules win.
  document.head.append(style);
  on("theme", (msg) => {
    style.textContent = msg.name === "endeavor" ? endeavorDark : "";
  });
}
