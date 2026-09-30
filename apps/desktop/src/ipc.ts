import { invoke } from "@tauri-apps/api/core";
import type {
  CheckWorkflowDto,
  MacroIdiomDto,
  MacroInsertionDto,
  MacroViewDto,
  UpdateChannel,
  UpdateStatusDto,
  ExportOverviewDto,
  FontPreviewSizeDto,
  ProjectChangeDto,
  FontSettings,
  FontsOverviewDto,
  ImportedFontFileDto,
  LocalExportDto,
  PackSettings,
  PublishedReleaseDto,
  ReleaseInput,
  TermInput,
  ProjectKnowledgeDto,
  CommandError,
  CollaborationDto,
  GameOpenResultDto,
  GameSettingsDto,
  GitBranchDto,
  GitCommitChangesDto,
  GitFinishDto,
  GitCommitDto,
  GitOverviewDto,
  GitRemoteDto,
  GitSyncDto,
  TranslatorIdentityDto,
  EntryChangeDto,
  PendingChangesDto,
  StringHistoryDto,
  EntryResolutionDto,
  ProjectOpenResultDto,
  ProjectSummaryDto,
  RecentProjectDto,
  SheetProgressDto,
  SourceBinding,
  TranslationRowCursorDto,
  TranslationRowPageDto,
  TranslationOverlayDto,
  OtherLanguageTextDto,
  SheetDialogueDto,
  ModelAccountDto,
  ModelInfo,
  ModelSignInDto,
  TranslationCountDto,
  TranslationStatus,
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

export type AppInfoDto = { name: string; version: string };

export function appInfo(): Promise<AppInfoDto> {
  return call<AppInfoDto>("app_info");
}

export function updateStatus(): Promise<UpdateStatusDto> {
  return call<UpdateStatusDto>("update_status");
}

/** Checks the release feed now; a failed check is reported in the status. */
export function checkForUpdates(): Promise<UpdateStatusDto> {
  return call<UpdateStatusDto>("update_check");
}

export function setUpdateChannel(channel: UpdateChannel): Promise<UpdateStatusDto> {
  return call<UpdateStatusDto>("update_set_channel", { channel });
}

/** Downloads and verifies the offered update; progress arrives as status events. */
export function downloadUpdate(): Promise<UpdateStatusDto> {
  return call<UpdateStatusDto>("update_download");
}

/** Installs the downloaded update and restarts Aeria. */
export function installUpdate(): Promise<void> {
  return call<void>("update_install");
}

/** Opens the release page of the offered update in the browser. */
export function openUpdateRelease(): Promise<void> {
  return call<void>("update_open_release");
}

export function currentProject(): Promise<ProjectSummaryDto | null> {
  return call<ProjectSummaryDto | null>("current_project");
}

/** The macros a translator can insert, with the open game's races and classes. */
export function macroInsertions(): Promise<MacroInsertionDto[]> {
  return call<MacroInsertionDto[]>("macro_insertions");
}

/** The constructs of several macros that read as one value. */
export function macroIdioms(): Promise<MacroIdiomDto[]> {
  return call<MacroIdiomDto[]>("macro_idioms");
}

/** Describes macro text: its diagnostics and tags. */
export function macroView(text: string): Promise<MacroViewDto> {
  return call<MacroViewDto>("macro_view", { text });
}

/** A TrueType font of the game font's private use glyphs; empty without a project. */
export function gameGlyphFont(): Promise<ArrayBuffer> {
  return call<ArrayBuffer>("game_glyph_font");
}

/** An inline game icon: width and height as little-endian u16, then RGBA; empty when absent. */
export function gameIcon(id: number): Promise<ArrayBuffer> {
  return call<ArrayBuffer>("game_icon", { id });
}

export function translationProgress(): Promise<SheetProgressDto[]> {
  return call<SheetProgressDto[]>("translation_progress");
}

/**
 * Opens a project with the configured game installation. When the project's
 * files are for an older game version, nothing is written and the versions
 * are returned.
 */
export function openProjectFromGame(repositoryRoot: string): Promise<GameOpenResultDto> {
  return call<GameOpenResultDto>("open_project_from_game", { repositoryRoot });
}

/** The game installation setting, what it resolves to, and detected installations. */
export function gameSettings(): Promise<GameSettingsDto> {
  return call<GameSettingsDto>("game_settings");
}

/** Chooses the game installation; `null` returns to automatic detection. */
export function setGamePath(path: string | null): Promise<GameSettingsDto> {
  return call<GameSettingsDto>("set_game_path", { path });
}

/**
 * Updates the project to the configured game and opens it: every file is
 * made again, translations are carried over (fuzzy where their source
 * changed), and the update is committed.
 */
export function updateProjectFromGame(repositoryRoot: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("update_project_from_game", { repositoryRoot });
}

/** Creates a project for the configured game installation. */
export function initializeProjectFromGame(
  repositoryRoot: string,
  sourceLanguage: string,
  targetLanguage: string,
): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("initialize_project_from_game", {
    repositoryRoot,
    sourceLanguage,
    targetLanguage,
  });
}

/** Sets the open project's target language, a BCP 47 tag other than `und`. */
export function setProjectTargetLanguage(targetLanguage: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("set_project_target_language", { targetLanguage });
}

export function listRecentProjects(): Promise<RecentProjectDto[]> {
  return call<RecentProjectDto[]>("list_recent_projects");
}

/**
 * Opens a recent project with the configured game. Without
 * `acceptSourceUpdate`, a required update returns its plan instead.
 */
export function openRecentProject(projectId: string, acceptSourceUpdate = false): Promise<GameOpenResultDto> {
  return call<GameOpenResultDto>("open_recent_project", { projectId, acceptSourceUpdate });
}

export function forgetRecentProject(projectId: string): Promise<void> {
  return call<void>("forget_recent_project", { projectId });
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

/** One source cell in the game's other client languages, for comparison. */
export function sourceInOtherLanguages(sourceBinding: SourceBinding): Promise<OtherLanguageTextDto[]> {
  return call<OtherLanguageTextDto[]>("source_in_other_languages", { sourceBinding });
}

/** The scene structure of a quest or cutscene sheet; `null` for other sheets. */
export function sheetDialogue(sheetName: string): Promise<SheetDialogueDto | null> {
  return call<SheetDialogueDto | null>("sheet_dialogue", { sheetName });
}

/**
 * Saves the translation of one string; an empty text leaves it untranslated.
 * Refused with `translationInvalid` when the checks find a problem. Returns
 * what the string's entry holds afterwards.
 */
export function setTranslationTarget(
  sourceBinding: SourceBinding,
  targetMacro: string,
): Promise<TranslationOverlayDto | null> {
  return call<TranslationOverlayDto | null>("set_translation_target", {
    sourceBinding,
    targetMacro,
  });
}

export function setTranslationNote(
  sourceBinding: SourceBinding,
  note: string | null,
): Promise<TranslationOverlayDto | null> {
  return call<TranslationOverlayDto | null>("set_translation_note", { sourceBinding, note });
}

export function gitOverview(): Promise<GitOverviewDto> {
  return call<GitOverviewDto>("git_overview");
}

export function gitInitialize(): Promise<GitOverviewDto> {
  return call<GitOverviewDto>("git_initialize");
}

export function gitSetIdentity(name: string, email: string | null, global: boolean): Promise<TranslatorIdentityDto> {
  return call<TranslatorIdentityDto>("git_set_identity", { name, email, global });
}

export function gitSetRemote(name: string, url: string): Promise<GitRemoteDto[]> {
  return call<GitRemoteDto[]>("git_set_remote", { name, url });
}

/** How many strings have uncommitted changes, with the first few hundred of them. */
export function gitPendingChanges(): Promise<PendingChangesDto> {
  return call<PendingChangesDto>("git_pending_changes");
}

/** Every uncommitted string change of one sheet. */
export function gitPendingSheetChanges(sheetName: string): Promise<EntryChangeDto[]> {
  return call<EntryChangeDto[]>("git_pending_sheet_changes", { sheetName });
}

export function gitCheckpoint(message: string | null): Promise<GitCommitChangesDto> {
  return call<GitCommitChangesDto>("git_checkpoint", { message });
}

export function gitLog(skip: number, limit: number): Promise<GitCommitDto[]> {
  return call<GitCommitDto[]>("git_log", { skip, limit });
}

export function gitCommitChanges(commitId: string): Promise<GitCommitChangesDto> {
  return call<GitCommitChangesDto>("git_commit_changes", { commitId });
}

/** The history of one string: its uncommitted change and the commits that changed it. */
export function gitStringHistory(sourceBinding: SourceBinding, limit: number): Promise<StringHistoryDto> {
  return call<StringHistoryDto>("git_string_history", { sourceBinding, limit });
}

export function gitProjectChanges(): Promise<ProjectChangeDto[]> {
  return call<ProjectChangeDto[]>("git_project_changes");
}

export function gitRemoveRemote(name: string): Promise<GitOverviewDto> {
  return call<GitOverviewDto>("git_remove_remote", { name });
}

/** Fetches every remote first, so it needs the network. */
/** Checks the remote main branch in the background; true when it moved. */
export function gitFetchMain(): Promise<boolean> {
  return call<boolean>("git_fetch_main");
}

export function gitRemoteBranches(): Promise<string[]> {
  return call<string[]>("git_remote_branches");
}

export function gitSetUpstream(remoteBranch: string): Promise<GitOverviewDto> {
  return call<GitOverviewDto>("git_set_upstream", { remoteBranch });
}

export function gitFetch(): Promise<void> {
  return call<void>("git_fetch");
}

export function gitPull(resolutions: EntryResolutionDto[] = []): Promise<GitSyncDto> {
  return call<GitSyncDto>("git_pull", { resolutions });
}

export function gitPush(): Promise<boolean> {
  return call<boolean>("git_push");
}

export function gitCheckWorkflow(): Promise<CheckWorkflowDto> {
  return call<CheckWorkflowDto>("git_check_workflow");
}

export function gitInstallCheckWorkflow(): Promise<CheckWorkflowDto> {
  return call<CheckWorkflowDto>("git_install_check_workflow");
}

export function gitOpenBranchSettings(): Promise<void> {
  return call<void>("git_open_branch_settings");
}

export function gitSync(resolutions: EntryResolutionDto[] = []): Promise<GitSyncDto> {
  return call<GitSyncDto>("git_sync", { resolutions });
}

export function gitBranches(): Promise<GitBranchDto[]> {
  return call<GitBranchDto[]>("git_branches");
}

export function gitCreateBranch(name: string): Promise<void> {
  return call<void>("git_create_branch", { name });
}

export function gitSwitchBranch(name: string): Promise<void> {
  return call<void>("git_switch_branch", { name });
}

/** Sets the main branch, or clears it so Aeria detects it. Writes the settings file only. */
export function gitSetMainBranch(mainBranch: string | null): Promise<CollaborationDto> {
  return call<CollaborationDto>("git_set_main_branch", { mainBranch });
}

/** A fingerprint of the repository state; null outside a repository. */
export function gitStateStamp(): Promise<string | null> {
  return call<string | null>("git_state_stamp");
}

export function gitFinishContribution(): Promise<GitFinishDto> {
  return call<GitFinishDto>("git_finish_contribution");
}

/** Deletes a local branch; `force` is needed when it has commits outside the main branch. */
export function gitDeleteBranch(name: string, force: boolean): Promise<void> {
  return call<void>("git_delete_branch", { name, force });
}

/** Merges the contribution into the main branch; only for repositories without a remote. */
export function gitMergeContribution(): Promise<GitFinishDto> {
  return call<GitFinishDto>("git_merge_contribution");
}

/** Clones into a folder named after the repository; `parent` defaults to the default projects directory. */
export function gitCloneRepository(url: string, parent: string | null): Promise<string> {
  return call<string>("git_clone_repository", { url, parent });
}

/** The folder that receives new and cloned projects when no other folder is chosen. */
export function defaultProjectsDirectory(): Promise<string> {
  return call<string>("default_projects_directory_path");
}

/** Whether a ChatGPT subscription is signed in for machine translation. */
export function modelAccount(): Promise<ModelAccountDto> {
  return call<ModelAccountDto>("model_account");
}

/** Starts signing in with a ChatGPT subscription; the code is entered on OpenAI's page. */
export function modelSignInStart(): Promise<ModelSignInDto> {
  return call<ModelSignInDto>("model_sign_in_start");
}

/** Opens OpenAI's page where the sign-in code is entered. */
export function modelOpenSignInPage(): Promise<void> {
  return call<void>("model_open_sign_in_page");
}

/** Polls the sign-in once; true once it finished. */
export function modelSignInPoll(): Promise<boolean> {
  return call<boolean>("model_sign_in_poll");
}

export function modelSignOut(): Promise<void> {
  return call<void>("model_sign_out");
}

/** The models the signed-in account can use. */
export function modelList(): Promise<ModelInfo[]> {
  return call<ModelInfo[]>("model_list");
}

/**
 * What a machine translation of `scope` would translate. The scope names
 * sheets, and folders of sheets ending with "/"; empty is the whole project.
 */
export function translationCount(scope: string[], fuzzy: boolean): Promise<TranslationCountDto> {
  return call<TranslationCountDto>("translation_count", { scope, fuzzy });
}

export function translationStart(scope: string[], fuzzy: boolean, model: string, effort: string | null): Promise<void> {
  return call<void>("translation_start", { scope, fuzzy, model, effort });
}

/** The progress of the last machine translation run; null before one started. */
export function translationStatus(): Promise<TranslationStatus | null> {
  return call<TranslationStatus | null>("translation_status");
}

export function translationStop(): Promise<void> {
  return call<void>("translation_stop");
}


export function projectKnowledge(): Promise<ProjectKnowledgeDto> {
  return call<ProjectKnowledgeDto>("project_knowledge");
}

export function saveKnowledgeStyle(expected: string | null, text: string): Promise<ProjectKnowledgeDto> {
  return call<ProjectKnowledgeDto>("save_knowledge_style", { expected, text });
}


export function saveKnowledgeTerms(expected: string | null, entries: TermInput[]): Promise<ProjectKnowledgeDto> {
  return call<ProjectKnowledgeDto>("save_knowledge_terms", { expected, entries });
}

export function exportOverview(): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_overview");
}

export function exportSaveSettings(settings: PackSettings): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_save_settings", { settings });
}

export function exportGenerateKey(replaceProjectKey: boolean): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_generate_key", { replaceProjectKey });
}

export function exportImportKey(path: string): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_import_key", { path });
}

export function exportBackupKey(path: string): Promise<void> {
  return call<void>("export_backup_key", { path });
}

export function exportRemoveKey(): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_remove_key");
}

export function exportInstallWorkflow(): Promise<ExportOverviewDto> {
  return call<ExportOverviewDto>("export_install_workflow");
}

export function exportPack(release: ReleaseInput, directory: string, sign: boolean): Promise<LocalExportDto> {
  return call<LocalExportDto>("export_pack", { release, directory, sign });
}

export function exportPublish(release: ReleaseInput): Promise<PublishedReleaseDto> {
  return call<PublishedReleaseDto>("export_publish", { release });
}

export function fontsOverview(): Promise<FontsOverviewDto> {
  return call<FontsOverviewDto>("fonts_overview");
}

export function fontsUseRecommended(): Promise<FontsOverviewDto> {
  return call<FontsOverviewDto>("fonts_use_recommended");
}

export function fontsSave(settings: FontSettings): Promise<FontsOverviewDto> {
  return call<FontsOverviewDto>("fonts_save", { settings });
}

export function fontsImportFile(path: string): Promise<ImportedFontFileDto> {
  return call<ImportedFontFileDto>("fonts_import_file", { path });
}

export function fontsPreview(settings: FontSettings, font: string, text: string): Promise<FontPreviewSizeDto[]> {
  return call<FontPreviewSizeDto[]>("fonts_preview", { settings, font, text });
}
