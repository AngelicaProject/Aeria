import type { MessageKey, Translate } from "./i18n/translate";
import type { CommandError } from "./types";

export type LauncherErrorOperation = "open" | "create" | "update" | "recentOpen";

export type LauncherError = {
  operation: LauncherErrorOperation;
  error: CommandError;
};

export function launcherErrorTitle(operation: LauncherErrorOperation): MessageKey {
  switch (operation) {
    case "open":
      return "launcher.error.open";
    case "create":
      return "launcher.error.create";
    case "update":
      return "launcher.error.update";
    case "recentOpen":
      return "launcher.error.recentOpen";
  }
}

export function sourcePackageListenerError(t: Translate): CommandError {
  return { code: "sourcePackageListener", message: t("launcher.error.listener") };
}
