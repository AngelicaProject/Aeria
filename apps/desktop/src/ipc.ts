import { invoke } from "@tauri-apps/api/core";
import type {
  CheckWorkflowDto,
  UpdateChannel,
  UpdateStatusDto,
  AgentMode,
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
  GlossaryEntryInput,
  ProjectGuideDto,
  JobAction,
  JobEvent,
  JobSummary,
  WorkerActivity,
  JobUnit,
  JobUnitStatus,
  ProposalRecord,
  AiConnectionCheckDto,
  AiModelConfig,
  ChatGptLoginDto,
  ConversationDto,
  ConversationSummaryDto,
  EditorContextDto,
  AiModelSelection,
  AiProviderInput,
  AiSettingsDto,
  CommandError,
  ReasoningEffort,
  CollaborationDto,
  DetachedUnitDto,
  GameOpenResultDto,
  GameSettingsDto,
  ContributorDto,
  GitBranchDto,
  GitCommitChangesDto,
  GitFinishDto,
  GitCommitDto,
  GitOverviewDto,
  GitRemoteDto,
  GitSyncDto,
  TranslatorIdentityDto,
  UnitAttributionDto,
  UnitChangeDto,
  UnitHistoryDto,
  UnitResolutionDto,
  ProjectOpenResultDto,
  ProjectSummaryDto,
  RecentProjectDto,
  ReviewState,
  SheetProgressDto,
  SourceBinding,
  SourceAvailability,
  SourcePackageEntryDto,
  SourcePackageJobDto,
  SourceUpdateReportDto,
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

export function translationProgress(): Promise<SheetProgressDto[]> {
  return call<SheetProgressDto[]>("translation_progress");
}

/**
 * Opens a project. A workspace that is not current for the package fails with
 * `sourceUpdateRequired` unless `acceptSourceUpdate` is set.
 */
export function openProject(
  repositoryRoot: string,
  sourcePackagePath: string,
  acceptSourceUpdate = false,
): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("open_project", { repositoryRoot, sourcePackagePath, acceptSourceUpdate });
}

/** Plans the source update opening would apply, without writing anything. */
export function previewSourceUpdate(repositoryRoot: string, sourcePackagePath: string): Promise<SourceUpdateReportDto> {
  return call<SourceUpdateReportDto>("preview_source_update", { repositoryRoot, sourcePackagePath });
}

/**
 * Opens a project with a local source package matching its workspace, or
 * builds one from the configured game installation under `jobId`.
 */
export function openProjectFromGame(jobId: string, repositoryRoot: string): Promise<GameOpenResultDto> {
  return call<GameOpenResultDto>("open_project_from_game", { jobId, repositoryRoot });
}

/** The game installation setting, what it resolves to, and detected installations. */
export function gameSettings(): Promise<GameSettingsDto> {
  return call<GameSettingsDto>("game_settings");
}

/** Chooses the game installation; `null` returns to automatic detection. */
export function setGamePath(path: string | null): Promise<GameSettingsDto> {
  return call<GameSettingsDto>("set_game_path", { path });
}

/** Packages in Aeria's source-package store, newest first. */
export function listSourcePackages(): Promise<SourcePackageEntryDto[]> {
  return call<SourcePackageEntryDto[]>("list_source_packages");
}

/** Deletes a package Aeria no longer needs and returns the updated list. */
export function deleteSourcePackage(packageId: string): Promise<SourcePackageEntryDto[]> {
  return call<SourcePackageEntryDto[]>("delete_source_package", { packageId });
}

/**
 * Whether opening (`opening`) or updating the project at `repositoryRoot`, or
 * creating one in `sourceLanguage`, finds a package without running Atlas.
 */
export function sourceAvailability(repositoryRoot: string | null, sourceLanguage: string | null, opening: boolean): Promise<SourceAvailability> {
  return call<SourceAvailability>("source_availability", { repositoryRoot, sourceLanguage, opening });
}

/** Opens the source-package store folder in the file manager. */
export function revealSourcePackages(): Promise<void> {
  return call<void>("reveal_source_packages");
}

/** Builds a source package from the configured game and updates the project to it. */
export function updateProjectFromGame(jobId: string, repositoryRoot: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("update_project_from_game", { jobId, repositoryRoot });
}

export function listDetachedUnits(): Promise<DetachedUnitDto[]> {
  return call<DetachedUnitDto[]>("list_detached_units");
}

export function initializeProject(
  repositoryRoot: string,
  sourcePackagePath: string,
  targetLanguage: string,
): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("initialize_project", {
    repositoryRoot,
    sourcePackagePath,
    targetLanguage,
  });
}

export function initializeProjectFromGame(
  jobId: string,
  repositoryRoot: string,
  sourceLanguage: string,
  targetLanguage: string,
): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("initialize_project_from_game", {
    jobId,
    repositoryRoot,
    sourceLanguage,
    targetLanguage,
  });
}

export function startSourcePackage(): Promise<SourcePackageJobDto> {
  return call<SourcePackageJobDto>("start_source_package");
}

export function listRecentProjects(): Promise<RecentProjectDto[]> {
  return call<RecentProjectDto[]>("list_recent_projects");
}

export function openRecentProject(projectId: string, acceptSourceUpdate = false): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("open_recent_project", { projectId, acceptSourceUpdate });
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

export function gitPendingChanges(): Promise<UnitChangeDto[]> {
  return call<UnitChangeDto[]>("git_pending_changes");
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

export function gitUnitHistory(translationUnitId: string, limit: number): Promise<UnitHistoryDto> {
  return call<UnitHistoryDto>("git_unit_history", { translationUnitId, limit });
}

export function gitContributors(): Promise<ContributorDto[]> {
  return call<ContributorDto[]>("git_contributors");
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

export function gitPull(resolutions: UnitResolutionDto[] = []): Promise<GitSyncDto> {
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

export function gitSync(resolutions: UnitResolutionDto[] = []): Promise<GitSyncDto> {
  return call<GitSyncDto>("git_sync", { resolutions });
}

export function gitUnitAttribution(translationUnitIds: string[]): Promise<UnitAttributionDto[]> {
  return call<UnitAttributionDto[]>("git_unit_attribution", { translationUnitIds });
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

export function aiSettings(): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_settings");
}

export function aiSaveProvider(provider: AiProviderInput): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_save_provider", { provider });
}

export function aiRemoveProvider(providerId: string): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_remove_provider", { providerId });
}

export function aiSetApiKey(providerId: string, apiKey: string): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_set_api_key", { providerId, apiKey });
}

export function aiClearApiKey(providerId: string): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_clear_api_key", { providerId });
}

export function aiSetAgentModel(selection: AiModelSelection | null): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_set_agent_model", { selection });
}

export function aiSetWorkerModel(selection: AiModelSelection | null): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_set_worker_model", { selection });
}

export function aiSetWebDomains(domains: string[]): Promise<AiSettingsDto> {
  return call<AiSettingsDto>("ai_set_web_domains", { domains });
}

export function aiListRemoteModels(providerId: string): Promise<AiModelConfig[]> {
  return call<AiModelConfig[]>("ai_list_remote_models", { providerId });
}

export function aiChatGptLoginStart(providerId: string): Promise<ChatGptLoginDto> {
  return call<ChatGptLoginDto>("ai_chatgpt_login_start", { providerId });
}

export function aiChatGptLoginCancel(loginId: string): Promise<void> {
  return call<void>("ai_chatgpt_login_cancel", { loginId });
}

export function aiTestConnection(providerId: string, modelId: string, effort: ReasoningEffort | null): Promise<AiConnectionCheckDto> {
  return call<AiConnectionCheckDto>("ai_test_connection", { providerId, modelId, effort });
}

export function angelicaConversations(): Promise<ConversationSummaryDto[]> {
  return call<ConversationSummaryDto[]>("angelica_conversations");
}

export function angelicaConversation(conversationId: string): Promise<ConversationDto> {
  return call<ConversationDto>("angelica_conversation", { conversationId });
}

export function angelicaDeleteConversation(conversationId: string): Promise<void> {
  return call<void>("angelica_delete_conversation", { conversationId });
}

export function angelicaCancel(conversationId: string): Promise<void> {
  return call<void>("angelica_cancel", { conversationId });
}

export function angelicaSend(conversationId: string | null, text: string, model: AiModelSelection, editor: EditorContextDto | null, mode: AgentMode): Promise<ConversationDto> {
  return call<ConversationDto>("angelica_send", { conversationId, text, model, editor, mode });
}

export function angelicaProposals(conversationId: string): Promise<ProposalRecord[]> {
  return call<ProposalRecord[]>("angelica_proposals", { conversationId });
}

export function angelicaApplyProposal(conversationId: string, proposalId: string): Promise<ProposalRecord[]> {
  return call<ProposalRecord[]>("angelica_apply_proposal", { conversationId, proposalId });
}

export function angelicaRejectProposal(conversationId: string, proposalId: string): Promise<ProposalRecord[]> {
  return call<ProposalRecord[]>("angelica_reject_proposal", { conversationId, proposalId });
}

export function angelicaDraft(sourceBinding: SourceBinding): Promise<{ target: string }> {
  return call<{ target: string }>("angelica_draft", { sourceBinding });
}

export function angelicaJobs(): Promise<JobSummary[]> {
  return call<JobSummary[]>("angelica_jobs");
}

export function angelicaJobUnits(jobId: string, statuses: JobUnitStatus[]): Promise<JobUnit[]> {
  return call<JobUnit[]>("angelica_job_units", { jobId, statuses });
}

export function angelicaJobEvents(jobId: string): Promise<JobEvent[]> {
  return call<JobEvent[]>("angelica_job_events", { jobId });
}

export function angelicaJobWorkers(jobId: string): Promise<WorkerActivity[]> {
  return call<WorkerActivity[]>("angelica_job_workers", { jobId });
}

export function angelicaJobControl(jobId: string, action: JobAction): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_control", { jobId, action });
}

export function angelicaJobSetConcurrency(jobId: string, concurrency: number): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_set_concurrency", { jobId, concurrency });
}

export function angelicaJobSetLimit(jobId: string, tokenLimit: number, resume: boolean): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_set_limit", { jobId, tokenLimit, resume });
}

export function angelicaJobRemove(jobId: string): Promise<void> {
  return call<void>("angelica_job_remove", { jobId });
}

export function angelicaJobRetry(jobId: string, statuses: JobUnitStatus[]): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_retry", { jobId, statuses });
}

export function projectGuide(): Promise<ProjectGuideDto> {
  return call<ProjectGuideDto>("project_guide");
}

export function saveProjectGuidance(expected: string | null, text: string): Promise<ProjectGuideDto> {
  return call<ProjectGuideDto>("save_project_guidance", { expected, text });
}

export function saveProjectGlossary(expected: string | null, entries: GlossaryEntryInput[]): Promise<ProjectGuideDto> {
  return call<ProjectGuideDto>("save_project_glossary", { expected, entries });
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
