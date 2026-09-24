import type { CommandError } from "../types";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

type ErrorBannerProps = {
  title: string;
  error: CommandError;
  onDismiss: () => void;
  tone?: "error" | "warning";
};

export function ErrorBanner({ title, error, onDismiss, tone = "error" }: ErrorBannerProps) {
  const { t } = useI18n();
  return (
    <div className={`notice notice-${tone}`} role="alert">
      <UiIcon icon={tone === "error" ? "circleAlert" : "triangleAlert"} size="md" className="notice-icon" />
      <div className="notice-text">
        <strong>{title}</strong>
        <p>{error.message}</p>
        <code className="notice-code">{error.code}</code>
      </div>
      <button className="icon-button icon-button-ghost" type="button" aria-label={t("common.dismiss")} onClick={onDismiss}>
        <UiIcon icon="x" size="sm" />
      </button>
    </div>
  );
}
