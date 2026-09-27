// The notebook's look (Settings, and the notebook's ⋮ menu). The app sets the
// webview's light/dark appearance, which Pluto's own themes follow
// (prefers-color-scheme). The Endeavor look maps Pluto's colour variables to
// Endeavor's tokens (docs/ui-spec.md) in dark mode, and hides Pluto's header,
// footer, floating panel and per-cell safe-preview labels: the app's header,
// the drawer and the safe-preview callout stand in for them. Pluto classic is
// Pluto's own page minus its file box (the app's header shows the file). Both
// hide Pluto's Instant feedback; the ⋮ menu offers it. There's no Endeavor
// light theme yet: in light mode the colours are Pluto's.

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
    /* Frontmatter and Pluto's other dialogs */
    --export-bg-color: #1C1C1F;
    --export-color: #D4D4D4;
    --frontmatter-button-bg-color: #26262A;
    --frontmatter-input-bg-color: #151517;
    --frontmatter-input-border-color: #3A3A40;
    /* A cell's run time: faint text, no chip */
    --pluto-runarea-bg-color: transparent;
    --pluto-runarea-span-color: #5E5E5E;
  }
  .pluto-modal { border: 1px solid #2A2A2E; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5) !important; }
  .pluto-modal-dark h1 { color: #ECECEC; font-weight: 600; }
  body.presentation nav#slide_controls { gap: 4px; padding: 4px; margin: 12px; border-radius: 8px;
    background: #1C1C1F; border: 1px solid #2A2A2E; }
  nav#slide_controls > button { border-radius: 5px; opacity: 0.8; }
  nav#slide_controls > button:hover { background: #26262A; opacity: 1; }
}
header#pluto-nav, footer, #helpbox-wrapper { display: none !important; }
html[data-endeavor-drawer="docs"] #helpbox-wrapper { display: block !important; }
.outline-frame.safe-preview, .outline-frame-actions-container.safe-preview { display: none !important; }
pluto-output.rich_output:has(> .safe-preview-output) { display: none !important; }
pluto-editor > main { padding-top: 16px; }
pluto-editor main { margin-right: max(0px, (100% - 731px) / 2) !important; }
pluto-runarea > span { font-size: 10px; }
`;

// Pluto classic: Pluto's page, without the file box (the app's header shows the file).
const classic = `
nav#at_the_top > pluto-filepicker, nav#at_the_top > div.desktop_picker_group { display: none !important; }
`;

// Instant feedback goes to Pluto's developers, not Endeavor's: the ⋮ menu offers it.
const both = `
footer form#feedback { display: none !important; }
`;

// The app's bundled JuliaMono (served by the app's endeavor: protocol), so code
// renders offline. Declared after Pluto's CDN faces of the same family, so these
// win for every character. Pluto declares only a regular-weight italic.
const juliaMono = [
  { file: "Regular", weight: 400, style: "normal" },
  { file: "Bold", weight: 700, style: "normal" },
  { file: "RegularItalic", weight: 400, style: "italic" },
]
  .map(
    ({ file, weight, style }) => `
@font-face {
  font-family: JuliaMono;
  src: url("endeavor://localhost/fonts/JuliaMono-${file}.ttf") format("truetype");
  font-display: swap;
  font-weight: ${weight};
  font-style: ${style};
}`,
  )
  .join("");

export function initTheme(): void {
  const fonts = document.createElement("style");
  fonts.id = "endeavor-fonts";
  fonts.textContent = juliaMono;
  document.head.append(fonts);

  const style = document.createElement("style");
  style.id = "endeavor-theme";
  // After Pluto's stylesheets, so equal-specificity rules win.
  document.head.append(style);
  on("theme", (msg) => {
    style.textContent = both + (msg.name === "endeavor" ? endeavorDark : classic);
    document.documentElement.dataset.endeavorLook = msg.name;
  });
}
