import {
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsUp,
  CircleHelp,
  CircleX,
  Copy,
  Ellipsis,
  Eye,
  EyeOff,
  ExternalLink,
  FileDiff,
  Folder,
  FolderOpen,
  GitBranch,
  GitCompareArrows,
  History,
  Info,
  ListFilter,
  LocateFixed,
  Minus,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Sparkles,
  Square,
  Table2,
  TriangleAlert,
  X,
  type LucideIcon,
} from "lucide-react";

export type UiIconSize = "xs" | "sm" | "md" | "lg";

export type UiIconName =
  | "arrowUpRight"
  | "check"
  | "chevronDown"
  | "chevronRight"
  | "chevronsUp"
  | "circleHelp"
  | "circleX"
  | "ellipsis"
  | "eye"
  | "eyeOff"
  | "externalLink"
  | "fileDiff"
  | "folder"
  | "folderOpen"
  | "gitBranch"
  | "gitCompareArrows"
  | "history"
  | "info"
  | "listFilter"
  | "locateFixed"
  | "panelBottom"
  | "panelLeft"
  | "panelRight"
  | "plus"
  | "refreshCw"
  | "search"
  | "settings"
  | "sparkles"
  | "square"
  | "table2"
  | "triangleAlert"
  | "minus"
  | "x"
  | "copy";

const ICON_SIZE = {
  xs: 12,
  sm: 14,
  md: 16,
  lg: 18,
} satisfies Record<UiIconSize, number>;

const ICONS = {
  arrowUpRight: ArrowUpRight,
  check: Check,
  chevronDown: ChevronDown,
  chevronRight: ChevronRight,
  chevronsUp: ChevronsUp,
  circleHelp: CircleHelp,
  circleX: CircleX,
  ellipsis: Ellipsis,
  eye: Eye,
  eyeOff: EyeOff,
  externalLink: ExternalLink,
  fileDiff: FileDiff,
  folder: Folder,
  folderOpen: FolderOpen,
  gitBranch: GitBranch,
  gitCompareArrows: GitCompareArrows,
  history: History,
  info: Info,
  listFilter: ListFilter,
  locateFixed: LocateFixed,
  panelBottom: PanelBottom,
  panelLeft: PanelLeft,
  panelRight: PanelRight,
  plus: Plus,
  refreshCw: RefreshCw,
  search: Search,
  settings: Settings,
  sparkles: Sparkles,
  square: Square,
  table2: Table2,
  triangleAlert: TriangleAlert,
  minus: Minus,
  x: X,
  copy: Copy,
} satisfies Record<UiIconName, LucideIcon>;

type UiIconProps = {
  icon: UiIconName;
  size?: UiIconSize;
  decorative?: boolean;
  className?: string;
};

export function UiIcon({ icon, size = "sm", decorative = true, className }: UiIconProps) {
  const Icon = ICONS[icon];
  return (
    <Icon
      size={ICON_SIZE[size]}
      strokeWidth={1.7}
      color="currentColor"
      aria-hidden={decorative || undefined}
      focusable="false"
      className={className}
    />
  );
}
