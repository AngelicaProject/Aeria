import type { SVGProps } from "react";

export type IconName =
  | "folder" | "search" | "branch" | "sparkles" | "more" | "chevron"
  | "chevronDown" | "chevronRight" | "close" | "minimize" | "maximize"
  | "plus" | "settings" | "target" | "collapse" | "external" | "help" | "file";

export function Icon({ name, size = 16, ...props }: { name: IconName; size?: number } & Omit<SVGProps<SVGSVGElement>, "name">) {
  const common = { viewBox: "0 0 24 24", width: size, height: size, fill: "none", stroke: "currentColor", strokeWidth: 1.7, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true, ...props };
  switch (name) {
    case "folder": return <svg {...common}><path d="M3.5 6.5h6l2 2h9v10h-17z" /></svg>;
    case "search": return <svg {...common}><circle cx="10.5" cy="10.5" r="5.5" /><path d="m15 15 5 5" /></svg>;
    case "branch": return <svg {...common}><circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="7" r="2" /><path d="M6 7v10M8 7h4a6 6 0 0 1 6 6v-4" /></svg>;
    case "sparkles": return <svg {...common}><path d="m12 3 1.7 5.3L19 10l-5.3 1.7L12 17l-1.7-5.3L5 10l5.3-1.7z" /><path d="m18 15 .8 2.2L21 18l-2.2.8L18 21l-.8-2.2L15 18l2.2-.8z" /></svg>;
    case "more": return <svg {...common}><circle cx="5" cy="12" r="1" fill="currentColor" /><circle cx="12" cy="12" r="1" fill="currentColor" /><circle cx="19" cy="12" r="1" fill="currentColor" /></svg>;
    case "chevron": return <svg {...common}><path d="m7 9 5 5 5-5" /></svg>;
    case "chevronDown": return <svg {...common}><path d="m6.5 9 5.5 5.5L17.5 9" /></svg>;
    case "chevronRight": return <svg {...common}><path d="m9 6.5 5.5 5.5L9 17.5" /></svg>;
    case "close": return <svg {...common}><path d="m6 6 12 12M18 6 6 18" /></svg>;
    case "minimize": return <svg {...common}><path d="M5 17h14" /></svg>;
    case "maximize": return <svg {...common}><path d="M6 6h12v12H6z" /></svg>;
    case "plus": return <svg {...common}><path d="M12 5v14M5 12h14" /></svg>;
    case "settings": return <svg {...common}><circle cx="12" cy="12" r="3.5" /><path d="m19 13.5 1.5 1-.9 1.7-1.8-.4a7 7 0 0 1-1.4 1.1l-.2 1.8h-2l-.7-1.7a7 7 0 0 1-1.8 0l-.7 1.7h-2l-.2-1.8a7 7 0 0 1-1.4-1.1l-1.8.4-.9-1.7 1.5-1a7 7 0 0 1 0-1.8l-1.5-1 .9-1.7 1.8.4a7 7 0 0 1 1.4-1.1l.2-1.8h2l.7 1.7a7 7 0 0 1 1.8 0l.7-1.7h2l.2 1.8a7 7 0 0 1 1.4 1.1l1.8-.4.9 1.7-1.5 1a7 7 0 0 1 0 1.8Z" /></svg>;
    case "target": return <svg {...common}><circle cx="12" cy="12" r="3" /><path d="M3 12h3M18 12h3M12 3v3M12 18v3" /></svg>;
    case "collapse": return <svg {...common}><path d="m7 14 5-5 5 5" /></svg>;
    case "external": return <svg {...common}><path d="M9 5H5v14h14v-4M13 5h6v6M19 5l-8 8" /></svg>;
    case "help": return <svg {...common}><circle cx="12" cy="12" r="9" /><path d="M9.7 9a2.4 2.4 0 1 1 3.5 2.1c-.8.4-1.2.9-1.2 1.9M12 16.5h.01" /></svg>;
    case "file": return <svg {...common}><path d="M6 3.5h8l4 4v13H6z" /><path d="M14 3.5v4h4" /></svg>;
  }
}
