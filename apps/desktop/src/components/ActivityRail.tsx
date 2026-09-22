import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";

type ActivityRailProps = {
  side: "left" | "right";
  items: readonly { id: string; label: string; icon: UiIconName; active?: boolean; onSelect?: () => void }[];
};

export function ActivityRail({ side, items }: ActivityRailProps) {
  return (
    <aside className={`activity-rail ${side}-rail`} aria-label={`${side} activity rail`}>
      {items.map((item) => (
        <button className={item.active ? "rail-button active" : "rail-button"} type="button" aria-label={item.label} title={item.label} key={item.id} onClick={item.onSelect}>
          <UiIcon icon={item.icon} size="md" />
        </button>
      ))}
    </aside>
  );
}
