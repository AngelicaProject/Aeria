import { IconButton } from "../ui/primitives/IconButton";
import type { UiIconName } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

export type ActivityRailItem = { id: string; label: string; icon: UiIconName; shortcut?: string; active?: boolean; onSelect?: () => void };

type ActivityRailProps = {
  side: "left" | "right";
  items: readonly ActivityRailItem[];
  footer?: readonly ActivityRailItem[];
};

export function ActivityRail({ side, items, footer = [] }: ActivityRailProps) {
  const { t } = useI18n();
  const render = (item: ActivityRailItem) => (
    <IconButton
      key={item.id}
      className={item.active ? "rail-button active" : "rail-button"}
      icon={item.icon}
      size="md"
      label={item.label}
      shortcut={item.shortcut}
      pressed={item.active}
      onClick={item.onSelect}
    />
  );
  return (
    <nav className={`rail rail-${side}`} aria-label={t(side === "left" ? "workbench.primaryTools" : "workbench.secondaryTools")}>
      {items.map(render)}
      {footer.length > 0 ? <><span className="spacer" />{footer.map(render)}</> : null}
    </nav>
  );
}
