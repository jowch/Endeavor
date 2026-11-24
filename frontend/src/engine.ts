export const onEmber = (): boolean => location.pathname.startsWith("/ember/");
export const onMarimo = (): boolean => location.pathname.startsWith("/marimo/");

export type Dom = { cell: string; input: string; output: string; selected: string; id: (cell: Element) => string };

const pluto: Dom = { cell: "pluto-cell", input: "pluto-input", output: "pluto-output", selected: "pluto-cell.selected", id: (cell) => cell.id };

const marimo: Dom = {
  cell: ".marimo-cell[data-cell-id]",
  input: "[data-testid=cell-editor]",
  output: ".output-area[data-cell-role=output]",
  selected: ":not(*)",
  id: (cell) => cell.getAttribute("data-cell-id") ?? "",
};

export const dom = (): Dom => (onMarimo() ? marimo : pluto);
