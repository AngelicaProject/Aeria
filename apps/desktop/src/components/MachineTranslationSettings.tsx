import { useCallback, useEffect, useRef, useState } from "react";
import { modelAccount, modelList, modelOpenSignInPage, modelSignInPoll, modelSignInStart, modelSignOut, normalizeCommandError } from "../ipc";
import type { CommandError, ModelAccountDto, ModelInfo, ModelSignInDto } from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import { usePreferences } from "../ui/preferences";

/** Signing in with a ChatGPT subscription, and the model machine translation uses. */
export function MachineTranslationSettings() {
  const { t } = useI18n();
  const { preferences, setPreference } = usePreferences();
  const [account, setAccount] = useState<ModelAccountDto | null>(null);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [signIn, setSignIn] = useState<ModelSignInDto | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const polling = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await modelAccount();
      setAccount(next);
      if (next.signedIn) setModels(await modelList());
    } catch (reason) {
      setError(normalizeCommandError(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
    return () => { if (polling.current !== null) window.clearTimeout(polling.current); };
  }, [refresh]);

  const poll = useCallback((interval: number) => {
    polling.current = window.setTimeout(() => {
      void modelSignInPoll().then((done) => {
        if (done) {
          setSignIn(null);
          void refresh();
        } else {
          poll(interval);
        }
      }).catch((reason: unknown) => {
        setSignIn(null);
        setError(normalizeCommandError(reason));
      });
    }, interval * 1000);
  }, [refresh]);

  async function start() {
    setBusy(true);
    setError(null);
    try {
      const login = await modelSignInStart();
      setSignIn(login);
      await modelOpenSignInPage().catch(() => undefined);
      poll(login.interval);
    } catch (reason) {
      setError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  }

  async function signOut() {
    setBusy(true);
    try {
      await modelSignOut();
      setModels(null);
      await refresh();
    } catch (reason) {
      setError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  }

  const model = models?.find((entry) => entry.id === preferences.translationModel) ?? null;
  return (
    <div className="game-settings">
      {error ? <ErrorBanner title={t("translate.settings.title")} error={error} onDismiss={() => setError(null)} /> : null}
      <p className="field-hint">{t("translate.settings.unofficial")}</p>
      {account === null ? (
        error ? null : <p className="muted">{t("common.loading")}</p>
      ) : account.signedIn ? (
        <>
          <div className="game-active">
            <UiIcon icon="circleCheck" size="md" />
            <div className="game-active-text">
              <strong>{t("translate.settings.signedIn")}</strong>
              {account.account ? <small>{account.account}</small> : null}
            </div>
          </div>
          <div className="game-actions">
            <button className="button button-ghost" type="button" disabled={busy} onClick={() => void signOut()}>{t("translate.settings.signOut")}</button>
          </div>
          <div className="field">
            <span className="field-label">{t("translate.settings.model")}</span>
            <Select<string>
              value={preferences.translationModel}
              label={t("translate.settings.model")}
              onChange={(value) => { setPreference("translationModel", value); setPreference("translationEffort", ""); }}
              options={[{ value: "", label: t("translate.settings.chooseModel") }, ...(models ?? []).map((entry) => ({ value: entry.id, label: entry.id }))]}
            />
          </div>
          {model && model.efforts.length > 0 ? (
            <div className="field">
              <span className="field-label">{t("translate.settings.effort")}</span>
              <Select<string>
                value={preferences.translationEffort}
                label={t("translate.settings.effort")}
                onChange={(value) => setPreference("translationEffort", value)}
                options={[{ value: "", label: t("translate.settings.defaultEffort") }, ...model.efforts.map((effort) => ({ value: effort, label: effort }))]}
              />
            </div>
          ) : null}
        </>
      ) : signIn ? (
        <div className="game-active">
          <span className="spinner" />
          <div className="game-active-text">
            <strong className="mono">{signIn.userCode}</strong>
            <small>{t("translate.settings.enterCode", { url: signIn.verificationUrl })}</small>
          </div>
          <button className="button button-secondary" type="button" onClick={() => void modelOpenSignInPage()}>{t("translate.settings.openPage")}</button>
        </div>
      ) : (
        <div className="game-actions">
          <button className="button button-primary" type="button" disabled={busy} onClick={() => void start()}>{t("translate.settings.signIn")}</button>
        </div>
      )}
    </div>
  );
}
