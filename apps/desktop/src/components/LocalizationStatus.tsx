import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { angelicaJobs } from "../ipc";
import type { JobSummary } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

/** One line in Angelica's panel while localizations run, with a way to open
 * the localization view; nothing when none runs. */
export function LocalizationStatus({ onOpen }: { onOpen: () => void }) {
  const { t, formatNumber } = useI18n();
  const [jobs, setJobs] = useState<JobSummary[]>([]);
  const reloadTimer = useRef<number | null>(null);

  const load = useCallback(() => {
    void angelicaJobs().then(setJobs).catch(() => undefined);
  }, []);

  useEffect(() => {
    load();
    const subscription = listen("angelica://job", () => {
      if (reloadTimer.current !== null) return;
      reloadTimer.current = window.setTimeout(() => { reloadTimer.current = null; load(); }, 1000);
    });
    return () => {
      void subscription.then((unlisten) => unlisten());
      if (reloadTimer.current !== null) window.clearTimeout(reloadTimer.current);
    };
  }, [load]);

  const live = jobs.filter((job) => job.status === "running" || job.status === "paused");
  if (live.length === 0) return null;
  const left = live.reduce((sum, job) => sum + job.counts.pending + job.counts.running, 0);
  const paused = live.every((job) => job.status === "paused");
  return (
    <button className="localization-status" type="button" onClick={onOpen} title={t("localization.open")}>
      <UiIcon icon={paused ? "pause" : "languages"} size="xs" />
      <span>{t(paused ? "localization.statusPaused" : "localization.status", { count: live.length, left: formatNumber(left) })}</span>
      <span className="localization-status-open">{t("localization.open")}</span>
    </button>
  );
}
