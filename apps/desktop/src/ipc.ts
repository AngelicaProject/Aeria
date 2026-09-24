import { invoke } from "@tauri-apps/api/core";
import type {
  AgentMode,
  JobAction,
  JobEvent,
  JobSummary,
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
  CollaborationPolicy,
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

/** Builds a source package from the installed game and updates the project to it. */
export function updateProjectFromGame(jobId: string, repositoryRoot: string, gamePath: string): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("update_project_from_game", { jobId, repositoryRoot, gamePath });
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
  gamePath: string,
  sourceLanguage: string,
  targetLanguage: string,
): Promise<ProjectOpenResultDto> {
  return call<ProjectOpenResultDto>("initialize_project_from_game", {
    jobId,
    repositoryRoot,
    gamePath,
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

export function gitSetCollaboration(policy: CollaborationPolicy, mainBranch: string | null): Promise<CollaborationDto> {
  return call<CollaborationDto>("git_set_collaboration", { policy, mainBranch });
}

export function gitFinishContribution(): Promise<GitFinishDto> {
  return call<GitFinishDto>("git_finish_contribution");
}

export function gitCloneRepository(url: string, destination: string): Promise<string> {
  return call<string>("git_clone_repository", { url, destination });
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

export function angelicaJobControl(jobId: string, action: JobAction): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_control", { jobId, action });
}

export function angelicaJobRetry(jobId: string, statuses: JobUnitStatus[]): Promise<JobSummary> {
  return call<JobSummary>("angelica_job_retry", { jobId, statuses });
}
