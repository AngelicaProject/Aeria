import { useEffect, useState } from "react";
import { checkForUpdates, normalizeCommandError, setUpdateChannel } from "../ipc";
import { awaitsNewerStable } from "../appUpdate";
import { formatRelativeTime } from "../timeDisplay";
import type { CommandError, UpdateChannel } from "../types";
import { reopenUpdate, setUpdateStatus, useAppUpdate } from "../ui/appUpdateStore";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

/** The update state of this copy and a manual check. */
export function UpdatesSetting() {
  const { t, locale } = useI18n();
  const { status } = useAppUpdate();
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  if (!status) return <p className="muted">{t("settings.updates.unavailable")}</p>;

  const summary = status.checking
    ? t("settings.updates.checking")
    : status.available
      ? t("settings.updates.available", { version: status.available.version })
      : status.error
        ? t("settings.updates.failed", { message: status.error.message })
        : status.lastCheckedAt === null
          ? t("settings.updates.notChecked")
          : t("settings.updates.upToDate", { version: status.currentVersion, time: formatRelativeTime(status.lastCheckedAt, now, locale, t("time.justNow")) });

  return (
    <div className="update-setting">
      <p className={status.error && !status.checking ? "update-setting-summary update-notice-error" : "update-setting-summary"}>
        {status.error && !status.checking ? <UiIcon icon="circleAlert" size="xs" /> : null}
        {summary}
      </p>
      {awaitsNewerStable(status) ? <p className="muted">{t("settings.updates.awaitingStable", { version: status.currentVersion })}</p> : null}
      {!status.canInstall ? <p className="muted">{t("settings.updates.portable")}</p> : null}
      <div className="update-setting-actions">
        <button className="button button-secondary" type="button" disabled={status.checking} onClick={() => void checkForUpdates().then(setUpdateStatus).catch(() => undefined)}>
          <UiIcon icon="refreshCw" size="sm" />{t("settings.updates.check")}
        </button>
        {status.available ? <button className="button button-primary" type="button" onClick={reopenUpdate}>{t("settings.updates.show")}</button> : null}
      </div>
    </div>
  );
}

export function UpdateChannelSetting() {
  const { t } = useI18n();
  const { status } = useAppUpdate();
  const [error, setError] = useState<CommandError | null>(null);

  async function handleChange(channel: UpdateChannel) {
    setError(null);
    try {
      setUpdateStatus(await setUpdateChannel(channel));
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }

  return (
    <div className="update-setting">
      <Segmented
        label={t("settings.channel.title")}
        value={status?.channel ?? null}
        disabled={!status || status.installing}
        onChange={(channel) => void handleChange(channel)}
        options={[{ value: "stable", label: t("update.channel.stable") }, { value: "nightly", label: t("update.channel.nightly") }]}
      />
      {error ? <p className="update-setting-summary update-notice-error" role="alert"><UiIcon icon="circleAlert" size="xs" />{error.message}</p> : null}
    </div>
  );
}
