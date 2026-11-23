export const EMBER = "pluto-editor:has(> #ember-run-status)";
export const PLUTO = "pluto-editor:not(:has(> #ember-run-status))";

const css = `
  pluto-editor { --e-left: 26px; --e-right: 10px; }
  ${EMBER} { --e-left: 46px; }
  pluto-editor { padding-top: 0 !important; }
  pluto-editor > main { padding-top: 4px !important; }
  pluto-editor:not(.disable_ui) > main { padding-bottom: 96px !important; }
  body > div:has(> pluto-editor) { min-height: calc(100vh - var(--endeavor-bar)) !important; }
  @media (max-width: 759px) {
    pluto-editor main { margin-left: 0 !important; margin-right: 0 !important; width: 100% !important; max-width: none !important;
      box-sizing: border-box; padding-left: var(--e-left) !important; padding-right: var(--e-right) !important; }
  }
  @media (min-width: 760px) {
    ${PLUTO}:not(:has(.plutoui-toc.aside)) main, ${EMBER} main { max-width: 1031px !important; margin-left: auto !important; margin-right: auto !important; }
    pluto-output .markdown { max-width: 720px; }
    body:has(pluto-helpbox:not(.hidden)) :is(${PLUTO}, ${EMBER}):not(.disable_ui) main { margin-left: 0 !important; margin-right: 392px !important;
      padding-left: var(--e-left) !important; max-width: min(1031px, calc(100% - 392px)) !important; }
  }
  pluto-editor pluto-output .markdown > :first-child { margin-top: 0 !important; }
  ${PLUTO} pluto-shoulder > button { padding-left: 4px !important; }
  ${PLUTO} pluto-input .cm-editor .cm-gutter { min-width: 0 !important; }
  ${PLUTO} pluto-input .cm-editor .cm-lineNumbers .cm-gutterElement { min-width: 16px; padding: 0 2px 0 3px !important; }
  ${PLUTO} pluto-input .cm-editor .cm-line { padding-left: 4px !important; }
  ${EMBER} pluto-input .cm-editor :is(.cm-gutters, .cm-gutter.cm-lineNumbers) { min-width: 24px !important; }
  ${EMBER} pluto-input .cm-editor .cm-lineNumbers .cm-gutterElement { padding: 0 6px 0 4px !important; }
  ${EMBER} pluto-input .cm-editor .cm-content { padding: 7px 8px 7px 6px !important; }
`;

export function initSpace(): void {
  const style = document.createElement("style");
  style.id = "endeavor-space";
  style.textContent = css;
  document.head.append(style);
}
