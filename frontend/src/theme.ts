// The notebook's look (Settings, and the notebook's ⋮ menu). The app sets the
// webview's light/dark appearance, which Pluto's own themes follow
// (prefers-color-scheme) and which the page follows live, with no reload. The
// Endeavor look maps Pluto's colour variables to Endeavor's tokens
// (docs/ui-spec.md) in both dark and light mode, and hides Pluto's header,
// footer, floating panel and per-cell safe-preview labels: the app's header,
// the drawer and the safe-preview callout stand in for them. Pluto classic is
// Pluto's own page minus its file box (the app's header shows the file), in
// Pluto's own dark/light colours. Both hide Pluto's Instant feedback; the ⋮
// menu offers it.

import { on } from "./bridge";

// Endeavor's own design tokens (--e-*), used by every module that draws its
// own UI (drawer.ts, rail.ts, prompt.ts, etc.) in both notebook looks and
// both colour schemes. Injected once as its own stylesheet, before any other
// module's styles, so the variables exist no matter what order modules init
// in -- CSS variables resolve at use time, not declaration time, so the
// order of <style> elements doesn't matter.
const tokens = `
:root {
  --e-bg-sidebar: #111113;
  --e-bg-page: #151517;
  --e-bg-card: #1C1C1F;
  --e-bg-raised: #26262A;
  --e-bg-sunken: #18181A;
  --e-bg-tag: #222225;
  --e-text-tag: #9A9A9A;
  --e-border: #2A2A2E;
  --e-control-edge: #3A3A40;
  --e-divider: #1F1F22;
  --e-text-primary: #ECECEC;
  --e-text-secondary: #BDBDBD;
  --e-text-muted: #8C8C8C;
  --e-text-faint: #858585;
  --e-text-section: #888888;
  --e-accent: #CC3F00;
  --e-accent-text: #E08A5E;
  --e-focus-ring: #E08A5E;
  --e-diff-add: #6CC784;
  --e-diff-del: #E07A7A;
  --e-diff-add-tint: rgba(108, 199, 132, 0.12);
  --e-diff-del-tint: rgba(224, 122, 122, 0.12);
  --e-popover-bg: #26262A;
  --e-popover-edge: #3A3A40;
  --e-menu-hover: #313136;
  --e-shadow-popover: rgba(0, 0, 0, 0.45);
  --e-button-hover: #2E2E33;
  --e-dialog-bg: #1C1C1F;
  --e-dialog-edge: #2A2A2E;
  --e-dialog-shadow: rgba(0, 0, 0, 0.5);

  /* Colours used by one or a few modules that don't match one of the tokens
     above closely enough to reuse it without shifting the dark look. */
  --e-text-dim: #7A7A7A;
  --e-text-waiting: #5E5E5E;
  --e-text-code: #D4D4D4;
  --e-dot-waiting: #4A4A4E;
  --e-danger-edge: rgba(224, 122, 122, 0.4);
  --e-danger-bg: rgba(224, 122, 122, 0.06);
  --e-diff-add-ch: rgba(108, 199, 132, 0.28);
  --e-diff-del-ch: rgba(224, 122, 122, 0.28);
  --e-you-stripe: #9A9A9A;
  --e-you-stripe-tint: rgba(154, 154, 154, 0.25);
  --e-stripe-tint: rgba(204, 63, 0, 0.3);
  --e-hover-edge: #FF7A40;
  --e-dim: rgba(0, 0, 0, 0.25);
  --e-hint-bg: rgba(28, 28, 30, 0.85);
  --e-hint-text: #ccc;
  --e-bar-bg: rgba(28, 28, 30, 0.72);
  --e-bar-shadow: rgba(0, 0, 0, 0.4);
  --e-field-bg: rgba(0, 0, 0, 0.35);
  --e-field-edge: #555555;
  --e-annotate-btn-bg: #3a3a3c;
  --e-pill-bg: #1C1C1E;
  --e-pill-edge: #333333;
  --e-prompt-hover: #FF9A6B;
  --e-backdrop: rgba(0, 0, 0, 0.45);
}
@media (prefers-color-scheme: light) {
  :root {
    --e-bg-sidebar: #F3F3F5;
    --e-bg-page: #FCFCFD;
    --e-bg-card: #F4F4F6;
    --e-bg-raised: #EAEAED;
    --e-bg-sunken: #F8F8FA;
    --e-bg-tag: #ECECEF;
    --e-text-tag: #5C5C64;
    --e-border: #E1E1E6;
    --e-control-edge: #8A8A92;
    --e-divider: #E6E6EA;
    --e-text-primary: #1B1B1F;
    --e-text-secondary: #45454C;
    --e-text-muted: #5C5C64;
    --e-text-faint: #66666E;
    --e-text-section: #6B6B73;
    --e-accent: #CC3F00;
    --e-accent-text: #B23600;
    --e-focus-ring: #CC3F00;
    --e-diff-add: #1C7038;
    --e-diff-del: #B42A36;
    --e-diff-add-tint: rgba(28, 140, 70, 0.12);
    --e-diff-del-tint: rgba(200, 40, 60, 0.1);
    --e-popover-bg: #FFFFFF;
    --e-popover-edge: #D9D9DF;
    --e-menu-hover: #EEEEF1;
    --e-shadow-popover: rgba(20, 20, 30, 0.14);
    --e-button-hover: #E2E2E6;
    --e-dialog-bg: #FFFFFF;
    --e-dialog-edge: #D9D9DF;
    --e-dialog-shadow: rgba(20, 20, 30, 0.16);

    --e-text-dim: #66666E;
    --e-text-waiting: #707078;
    --e-text-code: #26262B;
    --e-dot-waiting: #A6A6AE;
    --e-danger-edge: rgba(180, 42, 54, 0.35);
    --e-danger-bg: rgba(180, 42, 54, 0.05);
    --e-diff-add-ch: rgba(28, 140, 70, 0.24);
    --e-diff-del-ch: rgba(200, 40, 60, 0.2);
    --e-you-stripe: #8A8A92;
    --e-you-stripe-tint: rgba(138, 138, 146, 0.3);
    --e-stripe-tint: rgba(204, 63, 0, 0.25);
    --e-hover-edge: #CC3F00;
    --e-dim: rgba(20, 20, 30, 0.1);
    --e-hint-bg: rgba(255, 255, 255, 0.94);
    --e-hint-text: #45454C;
    --e-bar-bg: rgba(255, 255, 255, 0.94);
    --e-bar-shadow: rgba(20, 20, 30, 0.14);
    --e-field-bg: rgba(20, 20, 30, 0.06);
    --e-field-edge: #8A8A92;
    --e-annotate-btn-bg: #EAEAED;
    --e-pill-bg: #F4F4F6;
    --e-pill-edge: #D2D2D8;
    --e-prompt-hover: #B23600;
    --e-backdrop: rgba(20, 20, 30, 0.35);
  }
}
`;

// The Endeavor look, dark and light: Pluto's own CSS variables mapped to
// Endeavor's tokens (docs/ui-spec.md), plus the handful of rules Pluto has no
// variable for (.pluto-modal, the presentation slide controls).
const endeavorLook = `
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
    --cm-color-comment: #858585;
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
    --pluto-runarea-span-color: #858585;
  }
  .pluto-modal { border: 1px solid #2A2A2E; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5) !important; }
  .pluto-modal-dark h1 { color: #ECECEC; font-weight: 600; }
  body.presentation nav#slide_controls { gap: 4px; padding: 4px; margin: 12px; border-radius: 8px;
    background: #1C1C1F; border: 1px solid #2A2A2E; }
  nav#slide_controls > button { border-radius: 5px; opacity: 0.8; }
  nav#slide_controls > button:hover { background: #26262A; opacity: 1; }
}
@media (prefers-color-scheme: light) {
  :root {
    --main-bg-color: #FCFCFD;
    --header-bg-color: #FCFCFD;
    --footer-bg-color: #FCFCFD;
    --rule-color: rgba(20, 20, 30, 0.08);
    --code-background: #F4F4F6;
    --normal-cell-color: rgba(120, 120, 130, 0.2);
    --dark-normal-cell-color: rgba(120, 120, 130, 0.34);
    --code-differs-cell-color: #8A8A92;
    --selected-cell-color: rgba(64, 112, 196, 0.22);
    --pluto-output-color: #45454C;
    --pluto-output-h-color: #1B1B1F;
    --pluto-output-bg-color: #FCFCFD;
    --pluto-runarea-bg-color: #F4F4F6;
    --pluto-logs-bg-color: #F4F4F6;
    --overlay-button-bg: #F4F4F6;
    --input-context-menu-bg-color: #FFFFFF;
    --input-context-menu-border-color: #D9D9DF;
    --cm-selection-background: rgba(64, 112, 196, 0.18);
    --cm-color-editor-text: #26262B;
    --cm-color-keyword: #A2366F;
    --cm-color-control-operator: #A2366F;
    --cm-color-literal: #955A12;
    --cm-color-symbol: #955A12;
    --cm-color-string: #3F7A22;
    --cm-color-function: #2F5FA6;
    --cm-color-builtin: #2F5FA6;
    --cm-color-comment: #6B6B73;
    --cm-color-line-numbers: #6E6E76;
    /* Live docs */
    --helpbox-bg-color: #F4F4F6;
    --helpbox-header-bg-color: #EAEAED;
    --helpbox-header-tab-bg-color: #EAEAED;
    --helpbox-header-color: #1B1B1F;
    --helpbox-text-color: #26262B;
    --helpbox-search-bg-color: #FCFCFD;
    --helpbox-search-border-color: #8A8A92;
    --helpbox-box-shadow-color: rgba(20, 20, 30, 0.12);
    --docs-binding-bg: #EAEAED;
    /* Frontmatter and Pluto's other dialogs */
    --export-bg-color: #FFFFFF;
    --export-color: #26262B;
    --frontmatter-button-bg-color: #EAEAED;
    --frontmatter-input-bg-color: #FCFCFD;
    --frontmatter-input-border-color: #8A8A92;
    /* A cell's run time: faint text, no chip */
    --pluto-runarea-bg-color: transparent;
    --pluto-runarea-span-color: #6B6B73;
  }
  .pluto-modal { border: 1px solid #D9D9DF; border-radius: 8px !important; box-shadow: 0 12px 40px rgba(20, 20, 30, 0.16) !important; }
  .pluto-modal h1 { color: #1B1B1F; font-weight: 600; }
  body.presentation nav#slide_controls { gap: 4px; padding: 4px; margin: 12px; border-radius: 8px;
    background: #FFFFFF; border: 1px solid #D9D9DF; }
  nav#slide_controls > button { border-radius: 5px; opacity: 0.8; }
  nav#slide_controls > button:hover { background: #EAEAED; opacity: 1; }
}
header#pluto-nav, footer { display: none !important; }
/* Not display: none — Pluto alerts "window too small to show docs" whenever it opens a panel it finds undisplayed. */
html:not([data-endeavor-drawer="docs"]) #helpbox-wrapper { visibility: hidden !important; pointer-events: none !important;
  position: fixed !important; width: 0 !important; height: 0 !important; overflow: hidden !important; }
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
  // Before anything else: other modules' <style> elements reference --e-*
  // variables, and CSS variables resolve at use time, so this only needs to
  // exist somewhere in the document by the time they're used, not first in
  // the cascade -- but giving it a fixed, early spot keeps that obviously true.
  const tokenSheet = document.createElement("style");
  tokenSheet.id = "endeavor-tokens";
  tokenSheet.textContent = tokens;
  document.head.append(tokenSheet);

  const fonts = document.createElement("style");
  fonts.id = "endeavor-fonts";
  fonts.textContent = juliaMono;
  document.head.append(fonts);

  const style = document.createElement("style");
  style.id = "endeavor-theme";
  // After Pluto's stylesheets, so equal-specificity rules win.
  document.head.append(style);
  on("theme", (msg) => {
    style.textContent = both + (msg.name === "endeavor" ? endeavorLook : classic);
    document.documentElement.dataset.endeavorLook = msg.name;
  });
}
