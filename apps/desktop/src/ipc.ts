import { invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  ProjectSummaryDto,
  ReviewState,
  SourceBinding,
  TranslationRowCursorDto,
  TranslationRowPageDto,
  TranslationUnitIdDto,
} from "./types";

export function normalizeCommandError(error: unknown): CommandError {
  if (typeof error === "string") {
    return { code: "commandFailed", message: error };
  }

  if (error && typeof error === "object") {
    const candidate = error as Record<string, unknown>;
    const code = typeof candidate.code === "string" ? candidate.code : "commandFailed";
    const message =
      typeof candidate.message === "string"
        ? candidate.message
        : "The desktop command failed.";

    return { code, message };
  }

  return { code: "commandFailed", message: "The desktop command failed." };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return args === undefined ? await invoke<T>(command) : await invoke<T>(command, args);
  } catch (error) {
    throw normalizeCommandError(error);
  }
}

export function currentProject(): Promise<ProjectSummaryDto | null> {
  return call<ProjectSummaryDto | null>("current_project");
}

export function openProject(repositoryRoot: string, sourcePackagePath: string): Promise<ProjectSummaryDto> {
  return call<ProjectSummaryDto>("open_project", { repositoryRoot, sourcePackagePath });
}

export function initializeProject(
  repositoryRoot: string,
  sourcePackagePath: string,
  targetLanguage: string,
): Promise<ProjectSummaryDto> {
  return call<ProjectSummaryDto>("initialize_project", {
    repositoryRoot,
    sourcePackagePath,
    targetLanguage,
  });
}

export function closeProject(): Promise<void> {
  return call<void>("close_project");
}

export function pageTranslationRows(
  sheetName: string,
  after: TranslationRowCursorDto | null,
  limit: number,
): Promise<TranslationRowPageDto> {
  return call<TranslationRowPageDto>("page_translation_rows", {
    sheetName,
    after,
    limit,
  });
}

export function setTranslationTarget(
  sourceBinding: SourceBinding,
  targetMacro: string,
): Promise<TranslationUnitIdDto> {
  return call<TranslationUnitIdDto>("set_translation_target", {
    sourceBinding,
    targetMacro,
  });
}

export function setTranslationNote(
  translationUnitId: string,
  note: string | null,
): Promise<void> {
  return call<void>("set_translation_note", { translationUnitId, note });
}

export function setTranslationReviewState(
  translationUnitId: string,
  reviewState: ReviewState,
): Promise<void> {
  return call<void>("set_translation_review_state", { translationUnitId, reviewState });
}
