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
  GameOpenResultDto,
  GameSettingsDto,
  GitBranchDto,
  GitCommitChangesDto,
  GitCommitDto,
  GitOverviewDto,
  GitRemoteDto,
  GitPullDto,
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
  StringHintsDto,
  DraftCheckDto,
  TranslationOverlayDto,
  OtherLanguageTextDto,
  SheetDialogueDto,
  ModelAccountDto,
  ModelInfo,
  ModelSignInDto,
  TranslationStatus,
  BulkEditDto,
  EntryRefDto,
  ReplaceChangeDto,
  ReplacementDto,
  SearchEntryDto,
  SearchQueryDto,
  SearchResultDto,
  TermCandidateDto,
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

    const issues = Array.isArray(candidate.issues) ? (candidate.issues as CommandError["issues"]) : undefined;
    return issues && issues.length > 0 ? { code, message, issues } : { code, message };
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

/** The string guide: names, terms, speaker, length, and gender of one string, as a machine translation request reads them. */
export function stringHints(sourceBinding: SourceBinding): Promise<StringHintsDto> {
  return call<StringHintsDto>("string_hints", { sourceBinding });
}

/** What the checks find in a translation before it is saved. */
export function checkDraft(sourceBinding: SourceBinding, text: string): Promise<DraftCheckDto> {
  return call<DraftCheckDto>("check_draft", { sourceBinding, text });
}

/** Adds or removes a term exception of one string: the glossary term does not apply to it. */
export function setTranslationTermException(
  sourceBinding: SourceBinding,
  term: string,
  add: boolean,
): Promise<TranslationOverlayDto | null> {
  return call<TranslationOverlayDto | null>("set_translation_term_exception", { sourceBinding, term, add });
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
export function gitRemoteBranches(): Promise<string[]> {
  return call<string[]>("git_remote_branches");
}

export function gitSetUpstream(remoteBranch: string): Promise<GitOverviewDto> {
  return call<GitOverviewDto>("git_set_upstream", { remoteBranch });
}

export function gitFetch(): Promise<void> {
  return call<void>("git_fetch");
}

export function gitPull(resolutions: EntryResolutionDto[] = []): Promise<GitPullDto> {
  return call<GitPullDto>("git_pull", { resolutions });
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

export function gitBranches(): Promise<GitBranchDto[]> {
  return call<GitBranchDto[]>("git_branches");
}

export function gitCreateBranch(name: string): Promise<void> {
  return call<void>("git_create_branch", { name });
}

export function gitSwitchBranch(name: string): Promise<void> {
  return call<void>("git_switch_branch", { name });
}

/** A fingerprint of the repository state; null outside a repository. */
export function gitStateStamp(): Promise<string | null> {
  return call<string | null>("git_state_stamp");
}

/**
 * Deletes a local branch, and with `withUpstream` its upstream on the remote too.
 * `force` is needed when the deletion loses commits no other branch has.
 */
export function gitDeleteBranch(name: string, withUpstream: boolean, force: boolean): Promise<void> {
  return call<void>("git_delete_branch", { name, withUpstream, force });
}

/** Deletes a branch on its remote, named like `origin/feature`; `force` as for local branches. */
export function gitDeleteRemoteBranch(name: string, force: boolean): Promise<void> {
  return call<void>("git_delete_remote_branch", { name, force });
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
/** The sheets whose strings are the game's names, in the order a run translates them first. */
export function translationNameSheets(): Promise<string[]> {
  return call<string[]>("translation_name_sheets");
}

export function translationStart(scope: string[], fuzzy: boolean, model: string, effort: string | null): Promise<void> {
  return call<void>("translation_start", { scope, fuzzy, model, effort });
}

/** Translates the given strings again, by `msgctxt`: those still untranslated or fuzzy. */
export function translationRetry(contexts: string[], model: string, effort: string | null): Promise<void> {
  return call<void>("translation_retry", { contexts, model, effort });
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


/** Names the project renders in several ways that the glossary does not have. */
export function projectTermCandidates(): Promise<TermCandidateDto[]> {
  return call<TermCandidateDto[]>("project_term_candidates");
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

/** The project's commit authors, most commits first. */
export function exportGitAuthors(): Promise<string[]> {
  return call<string[]>("export_git_authors");
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

/** Searches the project's files; a new search cancels the one in progress. */
export function projectSearch(query: SearchQueryDto): Promise<SearchResultDto> {
  return call<SearchResultDto>("project_search", { query });
}

/** Every string the query finds, without the limit of a search, for choosing a whole result or sheet. */
export function projectSearchEntries(query: SearchQueryDto): Promise<SearchEntryDto[]> {
  return call<SearchEntryDto[]>("project_search_entries", { query });
}

export function projectSearchCancel(): Promise<void> {
  return call<void>("project_search_cancel");
}

/** The changes a replacement would make to the translations the query finds. */
export function projectReplacePreview(query: SearchQueryDto, replacement: ReplacementDto): Promise<ReplaceChangeDto[]> {
  return call<ReplaceChangeDto[]>("project_replace_preview", { query, replacement });
}

/** Writes replacements; strings changed since the preview or with problems are skipped. */
export function projectReplaceApply(edits: (EntryRefDto & { after: string })[]): Promise<BulkEditDto> {
  return call<BulkEditDto>("project_replace_apply", { edits });
}

/** Reverts the last bulk edit for the strings unchanged since. */
export function projectEditUndo(): Promise<BulkEditDto> {
  return call<BulkEditDto>("project_edit_undo");
}

/** Clears the strings' translations and machine-translates exactly them. */
/** Adds or removes a term exception of strings found by a search; an undo reverts it. */
export function projectTermException(entries: EntryRefDto[], term: string, add: boolean): Promise<BulkEditDto> {
  return call<BulkEditDto>("project_term_exception", { entries, term, add });
}

export function projectRetranslate(entries: EntryRefDto[], model: string, effort: string | null): Promise<BulkEditDto> {
  return call<BulkEditDto>("project_retranslate", { entries, model, effort });
}
