type ActivityRailProps = {
  active: "sheets";
};

export function ActivityRail({ active }: ActivityRailProps) {
  return (
    <nav className="activity-rail" aria-label="Workspace navigation">
      <div className="activity-rail-top">
        <span className="rail-separator" aria-hidden="true" />
        <button
          className={active === "sheets" ? "rail-button active" : "rail-button"}
          type="button"
          aria-label="Sheets"
          aria-current={active === "sheets" ? "page" : undefined}
        >
          <span aria-hidden="true">S</span>
        </button>
      </div>
      <div className="activity-rail-bottom">
        <span className="rail-caption">AERIA</span>
      </div>
    </nav>
  );
}
