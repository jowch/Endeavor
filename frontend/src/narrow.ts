// The notebook view at the widths of Claude's side panel, which goes down to
// about 320 px (the app's notebook pane is wider). Measured headlessly at 320,
// 400, 480 and 640 px (2026-11-12): Markdown headings sized for a wide page
// (a short title took five lines at 320 px), and long words and links in
// Markdown that ran past the cell's edge and were cut off. Outputs wider than
// the cell (a matrix, a plot) already scroll sideways inside the cell, as in
// the app. Pluto's header and footer, and Point's button, are the panel
// chrome's (view.ts), not this.

const css = `
  /* Long words and links in Markdown wrap at the cell's edge, rather than run past it. */
  pluto-editor pluto-output :is(p, li, blockquote, h1, h2, h3, h4, h5, h6, dt, dd) { overflow-wrap: break-word; }
  @media (max-width: 600px) {
    /* Pluto's are 2.2, 1.8, 1.6, 1.4 and 1.2rem. The pluto-editor in front outweighs Pluto's own rules, whichever loads last. */
    pluto-editor pluto-output h1 { font-size: 1.6rem; }
    pluto-editor pluto-output h2 { font-size: 1.4rem; }
    pluto-editor pluto-output h3 { font-size: 1.25rem; }
    pluto-editor pluto-output h4 { font-size: 1.15rem; }
    pluto-editor pluto-output h5 { font-size: 1.05rem; }
  }
`;

export function initNarrow(): void {
  const style = document.createElement("style");
  style.id = "endeavor-narrow";
  style.textContent = css;
  document.head.append(style);
}
