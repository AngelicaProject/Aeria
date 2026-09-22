import { Icon, type IconName } from "../ui/primitives/Icon";

type ActivityRailProps = {
  side: "left" | "right";
  items: readonly { id: string; label: string; icon: IconName; active?: boolean; onSelect?: () => void }[];
};

export function ActivityRail({ side, items }: ActivityRailProps) {
  return (
    <aside className={`activity-rail ${side}-rail`} aria-label={`${side} activity rail`}>
      {items.map((item) => (
        <button className={item.active ? "rail-button active" : "rail-button"} type="button" aria-label={item.label} title={item.label} key={item.id} onClick={item.onSelect}>
          <Icon name={item.icon} size={16} />
        </button>
      ))}
    </aside>
  );
}
