const css = `
  pluto-editor pluto-output :is(p, li, blockquote, h1, h2, h3, h4, h5, h6, dt, dd) { overflow-wrap: break-word; }
  @media (max-width: 600px) {
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
