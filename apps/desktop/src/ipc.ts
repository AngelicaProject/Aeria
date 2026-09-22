import { invoke } from "@tauri-apps/api/core";
import type {
  CommandError,
  ProjectOpenResultDto,
  ProjectSummaryDto,
  RecentProjectDto,
  ReviewState,
  SourceBinding,
  SourcePackageJobDto,
  TranslationRowCursorDto,
  TranslationRowPageDto,
  TranslationOverlayDto,
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

export function openProject(repositoryRoot: string, sourcePackagePath: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("open_project", { repositoryRoot, sourcePackagePath });
}

export function initializeProject(
  repositoryRoot: string,
  sourcePackagePath: string,
  targetLanguage?: string,
): Promise<ProjectOpenResultDto> {
  const args: Record<string, unknown> = {
    repositoryRoot,
    sourcePackagePath,
  };
  if (targetLanguage !== undefined) args.targetLanguage = targetLanguage;
  return call<ProjectOpenResultDto>("initialize_project", args);
}

export function initializeProjectFromGame(
  jobId: string,
  repositoryRoot: string,
  gamePath: string,
  sourceLanguage: string,
  targetLanguage?: string,
): Promise<ProjectOpenResultDto> {
  const args: Record<string, unknown> = {
    jobId,
    repositoryRoot,
    gamePath,
    sourceLanguage,
  };
  if (targetLanguage !== undefined) args.targetLanguage = targetLanguage;
  return call<ProjectOpenResultDto>("initialize_project_from_game", args);
}

export function startSourcePackage(): Promise<SourcePackageJobDto> {
  return call<SourcePackageJobDto>("start_source_package");
}

export function listRecentProjects(): Promise<RecentProjectDto[]> {
  return call<RecentProjectDto[]>("list_recent_projects");
}

export function openRecentProject(projectId: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("open_recent_project", { projectId });
}

export function forgetRecentProject(projectId: string): Promise<void> {
  return call<void>("forget_recent_project", { projectId });
}

export function cancelSourcePackage(jobId: string): Promise<void> {
  return call<void>("cancel_source_package", { jobId });
}

export function closeProject(): Promise<void> {
  return call<void>("close_project");
}

export function setProjectTargetLanguage(targetLanguage: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("set_project_target_language", { targetLanguage });
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
): Promise<TranslationOverlayDto> {
  return call<TranslationOverlayDto>("set_translation_target", {
    sourceBinding,
    targetMacro,
  });
}

export function setTranslationNote(
  translationUnitId: string,
  note: string | null,
): Promise<TranslationOverlayDto> {
  return call<TranslationOverlayDto>("set_translation_note", { translationUnitId, note });
}

export function setTranslationReviewState(
  translationUnitId: string,
  reviewState: ReviewState,
): Promise<TranslationOverlayDto> {
  return call<TranslationOverlayDto>("set_translation_review_state", { translationUnitId, reviewState });
}
