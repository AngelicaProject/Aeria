import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsUp,
  Circle,
  CircleAlert,
  CircleCheck,
  CircleDot,
  CircleHelp,
  CircleX,
  Clock,
  Cloud,
  CloudOff,
  Copy,
  CopyPlus,
  CornerDownLeft,
  Ellipsis,
  Eye,
  EyeOff,
  ExternalLink,
  FileDiff,
  Folder,
  FolderOpen,
  FolderPlus,
  Gamepad2,
  GitBranch,
  GitCommitHorizontal,
  GitCompareArrows,
  GitMerge,
  GitPullRequestArrow,
  History,
  Info,
  Languages,
  ListFilter,
  LocateFixed,
  MessageSquareText,
  Minus,
  Palette,
  Pause,
  PanelBottom,
  PanelLeft,
  PanelRight,
  Play,
  Plus,
  RefreshCw,
  Save,
  Search,
  Settings,
  Sparkles,
  Square,
  Table2,
  Trash2,
  TriangleAlert,
  Undo2,
  User,
  Users,
  X,
  type LucideIcon,
} from "lucide-react";

export type UiIconSize = "xs" | "sm" | "md" | "lg" | "xl";

const ICON_SIZE = {
  xs: 12,
  sm: 14,
  md: 16,
  lg: 18,
  xl: 22,
} satisfies Record<UiIconSize, number>;

const ICONS = {
  arrowDown: ArrowDown,
  arrowLeft: ArrowLeft,
  arrowRight: ArrowRight,
  arrowUp: ArrowUp,
  arrowUpRight: ArrowUpRight,
  check: Check,
  chevronDown: ChevronDown,
  chevronRight: ChevronRight,
  chevronsUp: ChevronsUp,
  circle: Circle,
  circleAlert: CircleAlert,
  circleCheck: CircleCheck,
  circleDot: CircleDot,
  circleHelp: CircleHelp,
  circleX: CircleX,
  clock: Clock,
  cloud: Cloud,
  cloudOff: CloudOff,
  copy: Copy,
  copyPlus: CopyPlus,
  cornerDownLeft: CornerDownLeft,
  ellipsis: Ellipsis,
  eye: Eye,
  eyeOff: EyeOff,
  externalLink: ExternalLink,
  fileDiff: FileDiff,
  folder: Folder,
  folderOpen: FolderOpen,
  folderPlus: FolderPlus,
  gamepad: Gamepad2,
  gitBranch: GitBranch,
  gitCommit: GitCommitHorizontal,
  gitCompareArrows: GitCompareArrows,
  gitMerge: GitMerge,
  gitPullRequest: GitPullRequestArrow,
  history: History,
  info: Info,
  languages: Languages,
  listFilter: ListFilter,
  locateFixed: LocateFixed,
  messageSquare: MessageSquareText,
  minus: Minus,
  palette: Palette,
  pause: Pause,
  panelBottom: PanelBottom,
  panelLeft: PanelLeft,
  panelRight: PanelRight,
  play: Play,
  plus: Plus,
  refreshCw: RefreshCw,
  save: Save,
  search: Search,
  settings: Settings,
  sparkles: Sparkles,
  square: Square,
  table2: Table2,
  trash: Trash2,
  triangleAlert: TriangleAlert,
  undo: Undo2,
  user: User,
  users: Users,
  x: X,
} satisfies Record<string, LucideIcon>;

export type UiIconName = keyof typeof ICONS;

type UiIconProps = {
  icon: UiIconName;
  size?: UiIconSize;
  decorative?: boolean;
  className?: string | undefined;
};

export function UiIcon({ icon, size = "sm", decorative = true, className }: UiIconProps) {
  const Icon = ICONS[icon];
  return (
    <Icon
      size={ICON_SIZE[size]}
      strokeWidth={1.75}
      color="currentColor"
      aria-hidden={decorative || undefined}
      focusable="false"
      className={className}
    />
  );
}
