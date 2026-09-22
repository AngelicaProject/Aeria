import type { CommandError } from "../types";
import { UiIcon } from "../ui/primitives/UiIcon";

type ErrorBannerProps = {
  title: string;
  error: CommandError;
  onDismiss: () => void;
};

export function ErrorBanner({ title, error, onDismiss }: ErrorBannerProps) {
  return (
    <div className="error-banner" role="alert">
      <div>
        <strong>{title}</strong>
        <p>{error.message}</p>
        <span className="error-code">{error.code}</span>
      </div>
      <button className="icon-button" type="button" aria-label="Dismiss error" onClick={onDismiss}>
        <UiIcon icon="x" size="sm" />
      </button>
    </div>
  );
}
