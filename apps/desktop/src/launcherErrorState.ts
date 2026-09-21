import type { CommandError } from "./types";

export type LauncherErrorOperation = "open" | "create" | "recentOpen";

export type LauncherError = {
  operation: LauncherErrorOperation;
  error: CommandError;
};

export function launcherErrorTitle(operation: LauncherErrorOperation): string {
  switch (operation) {
    case "open":
      return "Could not open project";
    case "create":
      return "Could not create project";
    case "recentOpen":
      return "Could not reopen recent project";
  }
}

export function sourcePackageListenerError(): CommandError {
  return {
    code: "sourcePackageListener",
    message:
      "Source-package progress updates could not be connected. Project creation is disabled until this connection is available.",
  };
}
