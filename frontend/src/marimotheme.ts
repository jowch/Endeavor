import { on } from "./bridge";

const LOOK = `html[data-endeavor-look="endeavor"]`;
const NEUTRALS = (ramp: string[]) => ramp.map((value, i) => `--slate-${i + 1}: ${value}; --gray-${i + 1}: ${value};`).join(" ");

export const DARK_RAMP = ["#151517", "#18181A", "#1C1C1F", "#222225", "#26262A", "#2A2A2E", "#3A3A40", "#4A4A4E", "#6E6E72", "#858483", "#BDBCBA", "#F0EFEC"];
export const LIGHT_RAMP = ["#FCFCFD", "#F8F8FA", "#F4F4F6", "#EEEEF1", "#EAEAED", "#E1E1E6", "#D9D9DF", "#B9B9C0", "#8A8A92", "#6E6E76", "#5C5C64", "#1B1B1F"];

export const marimoLook = `
${LOOK} body.dark {
  ${NEUTRALS(DARK_RAMP)}
  --background: #151517; --foreground: #F0EFEC;
  --card: #151517; --card-foreground: #F0EFEC;
  --popover: #26262A; --popover-foreground: #F0EFEC;
  --muted: #1C1C1F; --muted-foreground: #8C8B8A;
  --secondary: #1C1C1F; --secondary-foreground: #F0EFEC;
  --accent: #313136; --accent-foreground: #F0EFEC;
  --border: #2A2A2E; --input: #3A3A40; --ring: #E08A5E;
  --cm-background: #1B1B1E;
}
${LOOK} body.light {
  ${NEUTRALS(LIGHT_RAMP)}
  --background: #FCFCFD; --foreground: #1B1B1F;
  --card: #FCFCFD; --card-foreground: #1B1B1F;
  --popover: #FFFFFF; --popover-foreground: #1B1B1F;
  --muted: #F4F4F6; --muted-foreground: #5C5C64;
  --secondary: #F4F4F6; --secondary-foreground: #1B1B1F;
  --accent: #EEEEF1; --accent-foreground: #1B1B1F;
  --border: #E1E1E6; --input: #8A8A92; --ring: #CC3F00;
  --cm-background: #F4F4F6;
}
${LOOK} #App { --e-side: clamp(28px, calc(100vw * 50 / 860), 50px); --content-width: 731px; }
${LOOK} #App > div.sticky:has([data-testid=dir-completion-input]) { display: none !important; }
${LOOK} #App > div:has(> .m-auto [data-testid=cell-column]) { padding: 48px 0 0 !important; }
${LOOK} #App .m-auto:has(> div > [data-testid=cell-column]) { margin-left: var(--e-side) !important; margin-right: auto !important; }
${LOOK} [data-testid=shutdown-button] { display: none !important; }
${LOOK} .marimo-cell :is(.cm-editor, .cm-gutters) { background-color: var(--cm-background) !important; }
`;

export function initMarimoLook(): void {
  const style = document.createElement("style");
  style.id = "endeavor-theme";
  style.textContent = marimoLook;
  document.head.append(style);
  on("theme", (msg) => {
    document.documentElement.dataset.endeavorLook = msg.name;
  });
}
