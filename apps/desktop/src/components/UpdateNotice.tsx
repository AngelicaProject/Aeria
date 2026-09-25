import { useEffect, useState } from "react";
import { downloadUpdate, installUpdate, normalizeCommandError, openUpdateRelease } from "../ipc";
import { downloadShare, formatMegabytes, installBlockers, shouldOfferUpdate, type UpdateBlocker } from "../appUpdate";
import type { CommandError } from "../types";
import { postponeUpdate, refreshUpdateStatus, reopenUpdate, setUpdateStatus, useAppUpdate } from "../ui/appUpdateStore";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

const blockerLabels: Record<UpdateBlocker, MessageKey> = {
  unsavedDraft: "update.blocker.unsavedDraft",
  translation: "update.blocker.translation",
  sync: "update.blocker.sync",
  export: "update.blocker.export",
  sourcePackage: "update.blocker.sourcePackage",
};

/** Running work is re-read this often while it holds back a requested install. */
const BLOCKED_REFRESH_MS = 2000;

/** Title-bar button that brings back a postponed update offer. */
export function UpdateTitleButton() {
  const { t } = useI18n();
  const { status } = useAppUpdate();
  const available = status?.available;
  if (!available) return null;
  return (
    <button className="titlebar-update" type="button" title={t("update.available.title", { version: available.version })} onClick={reopenUpdate}>
      <UiIcon icon="circleArrowUp" size="xs" />{t("update.titleButton")}
    </button>
  );
}

/**
 * Offers a newer version in a corner card. The card never installs on its
 * own; "Later" hides it until the next launch or a newer version, and the
 * title-bar button brings it back.
 */
export function UpdateNotice() {
  const { t, locale } = useI18n();
  const { status, unsavedDraft, postponedVersion } = useAppUpdate();
  const [requested, setRequested] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);

  const available = status?.available ?? null;
  const blockers = status ? installBlockers(status, unsavedDraft) : [];
  const ready = status?.download?.ready ?? false;
  const backendBlocked = (status?.runningActivities.length ?? 0) > 0;

  useEffect(() => {
    setRequested(false);
    setError(null);
  }, [available?.version]);

  // Poll only while a requested install waits for backend work to finish.
  useEffect(() => {
    if (!requested || !backendBlocked) return;
    const timer = window.setInterval(() => void refreshUpdateStatus(), BLOCKED_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [backendBlocked, requested]);

  // Install once the download finished and nothing would be interrupted.
  useEffect(() => {
    if (!requested || !ready || blockers.length > 0 || status?.installing) return;
    setRequested(false);
    void installUpdate().catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [blockers.length, ready, requested, status?.installing]);

  if (!status || !available || !shouldOfferUpdate(status, postponedVersion)) return null;

  async function handleInstall() {
    setError(null);
    setRequested(true);
    if (ready) return;
    try {
      setUpdateStatus(await downloadUpdate());
    } catch (reason) {
      setRequested(false);
      setError(normalizeCommandError(reason));
    }
  }

  function handleLater(version: string) {
    setRequested(false);
    postponeUpdate(version);
  }

  function handleOpenRelease() {
    void openUpdateRelease().catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }

  const download = status.download;
  const share = downloadShare(download);
  const downloading = download !== null && !download.ready;
  const waiting = requested && blockers.length > 0;
  const shownError = error ?? status.error;
  const published = available.publishedAt ? new Date(available.publishedAt) : null;
  const facts = [
    t(available.channel === "nightly" ? "update.channel.nightly" : "update.channel.stable"),
    published && !Number.isNaN(published.getTime()) ? published.toLocaleDateString(locale, { day: "numeric", month: "long" }) : null,
  ].filter(Boolean).join(" · ");

  return (
    <aside className="update-notice" role="status" aria-label={t("update.available.title", { version: available.version })}>
      <span className="update-notice-icon" aria-hidden="true"><UiIcon icon="circleArrowUp" size="md" /></span>
      <div className="update-notice-body">
        <div className="update-notice-head">
          <strong>{t("update.available.title", { version: available.version })}</strong>
          {!status.installing ? (
            <button className="icon-button icon-button-ghost" type="button" aria-label={t("update.later")} onClick={() => handleLater(available.version)}>
              <UiIcon icon="x" size="sm" />
            </button>
          ) : null}
        </div>
        <p className="update-notice-facts">
          {facts}
          {" · "}
          <button className="link-button" type="button" onClick={handleOpenRelease}>{t("update.whatsNew")}</button>
        </p>

        {!status.canInstall ? <p className="update-notice-text">{t("update.portable")}</p> : null}

        {downloading ? (
          <div className="update-notice-progress">
            <div className={share === null ? "progress-track indeterminate" : "progress-track"}><span style={{ width: `${(share ?? 0.3) * 100}%` }} /></div>
            <small>
              {download.total
                ? t("update.downloading", { received: formatMegabytes(download.received, locale), total: formatMegabytes(download.total, locale) })
                : t("update.downloadingUnknown", { received: formatMegabytes(download.received, locale) })}
            </small>
          </div>
        ) : null}

        {status.installing ? <p className="update-notice-text">{t("update.installing")}</p> : null}

        {status.canInstall && !status.installing && blockers.length > 0 ? (
          <p className="update-notice-text update-notice-waiting">
            <UiIcon icon="clock" size="xs" />
            {t(waiting ? "update.waitingRequested" : "update.waiting", { reasons: blockers.map((blocker) => t(blockerLabels[blocker])).join(", ") })}
          </p>
        ) : null}

        {shownError ? <p className="update-notice-text update-notice-error" role="alert"><UiIcon icon="circleAlert" size="xs" />{shownError.message}</p> : null}

        {!status.installing ? (
          <div className="update-notice-actions">
            {status.canInstall ? (
              <button className="button button-primary" type="button" disabled={downloading || waiting} onClick={() => void handleInstall()}>
                {t(ready ? "update.install" : "update.downloadAndInstall")}
              </button>
            ) : (
              <button className="button button-primary" type="button" onClick={handleOpenRelease}>
                <UiIcon icon="externalLink" size="sm" />{t("update.openRelease")}
              </button>
            )}
            <button className="button button-secondary" type="button" onClick={() => handleLater(available.version)}>{t("update.later")}</button>
          </div>
        ) : null}
      </div>
    </aside>
  );
}
