// The notebook view's room in Claude's side panel (about 513 x 721 px, down to 320 wide), so that the
// cells, not empty margins, get the pixels. Measured headlessly with real Julia and R at 320, 513 and
// 1280 px (2026-11-12). Before, at 320 px, a Pluto cell's code began 65 px from the left and ended 29
// px from the right edge (226 px of code); an Ember cell's 100 px and 50 px (170 px); 33 px (Pluto)
// and 94 px (Ember) sat empty under the toolbar, and 256 px under the last cell. In full screen the
// cells kept Pluto's 700 px column at the left, with 500 px empty on the right.
//
// What stays, and why: the 17 px (Pluto) and 26 px (Ember) between cells hold each cell's run time
// and the add-cell buttons; the room left of a cell holds its show/hide-code eye (and Ember's run
// button), so it shrinks only to what they need. The line numbers keep room for two digits: Point
// shows them. This is the view only: the app's notebook pane keeps the chat column's margins
// (theme.ts).

/** Ember's page (R notebooks), which is Pluto's fork with its own spacing. */
export const EMBER = "pluto-editor:has(> #ember-run-status)";
export const PLUTO = "pluto-editor:not(:has(> #ember-run-status))";

const css = `
  /* Left of a cell: Pluto's eye (16 px icon), or Ember's eye and run button (46 px, Ember's own
     measure). Right: 10 px, for an overlay scrollbar's thumb. */
  pluto-editor { --e-left: 26px; --e-right: 10px; }
  ${EMBER} { --e-left: 46px; }
  /* Ember keeps 52 px at the top for its own header, which the view hides. */
  pluto-editor { padding-top: 0 !important; }
  /* Under the toolbar, only the room the first cell's add-cell button needs (the cell's own margin). */
  pluto-editor > main { padding-top: 4px !important; }
  /* Under the last cell: enough to bring it up off the bottom edge and clear Point's bar, not Pluto's
     16rem, which in a 721 px panel was a third of it empty. */
  pluto-editor:not(.disable_ui) > main { padding-bottom: 96px !important; }
  /* The page is the panel's height under the toolbar, so a short notebook doesn't scroll by the toolbar's height. */
  body > div:has(> pluto-editor) { min-height: calc(100vh - var(--endeavor-bar)) !important; }
  /* 760 px is where the panel ends and full screen begins, here and for Docs and Status (view.ts):
     below it they open as a drawer along the bottom, from it as a sheet down the right. */
  @media (max-width: 759px) {
    /* The panel: cells from the eye to the scrollbar, whatever the width. */
    pluto-editor main { margin-left: 0 !important; margin-right: 0 !important; width: 100% !important; max-width: none !important;
      box-sizing: border-box; padding-left: var(--e-left) !important; padding-right: var(--e-right) !important; }
  }
  @media (min-width: 760px) {
    /* Full screen: a wider column, centred, for long code lines and wide tables. Prose keeps a
       readable measure (the cap holds a Markdown cell's own images and tables to it too). Not beside
       PlutoUI's table of contents, which floats at the right. */
    ${PLUTO}:not(:has(.plutoui-toc.aside)) main, ${EMBER} main { max-width: 1031px !important; margin-left: auto !important; margin-right: auto !important; }
    pluto-output .markdown { max-width: 720px; }
    /* With Docs or Status open as a sheet down the right (view.ts), the column moves out from under it,
       to the left edge. The selector must outweigh the column rule above (1,2,2: its :has(.plutoui-toc.aside)
       counts two classes); this one is (1,2,4). */
    body:has(pluto-helpbox:not(.hidden)) :is(${PLUTO}, ${EMBER}):not(.disable_ui) main { margin-left: 0 !important; margin-right: 392px !important;
      padding-left: var(--e-left) !important; max-width: min(1031px, calc(100% - 392px)) !important; }
  }
  /* A Markdown cell's first heading starts at the top of its output, not a line below it. */
  pluto-editor pluto-output .markdown > :first-child { margin-top: 0 !important; }
  /* The eye sits right against the cell, without Pluto's 10 px of padding to its left. */
  ${PLUTO} pluto-shoulder > button { padding-left: 4px !important; }
  /* Inside a cell, the line numbers (dots until hovered) take two digits' room, not 31 px (Pluto) or 34 px (Ember). */
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
