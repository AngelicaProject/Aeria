export type CommandError = {
  code: string;
  message: string;
};

export type AtlasEvent =
  | { type: "started"; protocolVersion: number; language: string | null }
  | { type: "phase"; protocolVersion: number; phase: string }
  | {
      type: "progress";
      protocolVersion: number;
      phase: string;
      sheet: string | null;
      language: string | null;
      sheetIndex: number | null;
      sheetCount: number | null;
      rowsProcessed: number | null;
      sheetCompleted: boolean | null;
    }
  | {
      type: "completed";
      protocolVersion: number;
      packageId: string;
      outputPath: string | null;
      metadata: Record<string, unknown>;
    }
  | {
      type: "failed";
      protocolVersion: number;
      code: string | null;
      message: string | null;
    };

export type SourcePackageEventPayload = {
  jobId: string;
  event: AtlasEvent;
};

export type SourcePackageJobDto = {
  jobId: string;
};

export type ReviewState = "draft" | "reviewed" | "needsReview";

export type SourceBinding = {
  sheetName: string;
  rowId: number;
  subrowId: number;
  columnIndex: number;
};

export type ProjectSheetDto = {
  name: string;
  effectiveLanguage: string;
  rowCount: number;
  translatableCellCount: number;
};

export type ProjectSummaryDto = {
  repositoryRoot: string;
  sourcePackagePath: string;
  sourcePackageId: string;
  sourceLanguage: string;
  targetLanguage: string;
  sourceContentId: string;
  sourceSnapshotId: string;
  gameVersion: string;
  scope: string;
  sheets: ProjectSheetDto[];
  /** Translations preserved without a current source occurrence. */
  detachedUnitCount: number;
};

/** Workspace coverage for one sheet; sheets without translations are omitted. */
export type SheetProgressDto = {
  sheetName: string;
  translated: number;
  reviewed: number;
  needsReview: number;
};

export type ProjectOpenResultDto = {
  project: ProjectSummaryDto;
  warning: CommandError | null;
  /** Present when opening applied a source update. */
  sourceUpdate: SourceUpdateReportDto | null;
};

/** Why a translation is preserved without a current source occurrence. */
export type DetachReason =
  | "sheetRemoved"
  | "sheetUnavailable"
  | "rowRemoved"
  | "cellRemoved"
  | "columnUnresolved"
  | "notTranslatable"
  | "bindingConflict";

export type SheetSchemaUpdateDto = {
  sheetName: string;
  removed: boolean;
  /** The sheet still exists in the game but the source could not read it. */
  unavailable: boolean;
  mappedColumns: number;
  unresolvedColumns: number;
};

/** Counts of a previewed or applied deterministic source update. */
export type SourceUpdateReportDto = {
  previousContentId: string;
  contentId: string;
  gameVersion: string;
  previousFormatVersion: number;
  unchanged: number;
  encodingChanged: number;
  sourceChanged: number;
  detached: number;
  newlyDetached: number;
  reattached: number;
  columnMapped: number;
  rowMoved: number;
  changedUnits: number;
  sheetSchemaUpdates: SheetSchemaUpdateDto[];
};

export type DetachedUnitDto = {
  translationUnitId: string;
  lastSourceBinding: SourceBinding;
  reason: DetachReason;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type RecentProjectAvailability =
  | "ready"
  | "repositoryMissing"
  | "sourcePackageMissing"
  | "repositoryAndSourceMissing";

/** Outcome of opening a project from a game installation. */
export type GameOpenResultDto =
  | { status: "opened"; result: ProjectOpenResultDto }
  | {
    /** Nothing was written; confirm, then open with this package and `acceptSourceUpdate`. */
    status: "sourceUpdateRequired";
    sourcePackagePath: string;
    report: SourceUpdateReportDto;
  };

export type GameOrigin = "settings" | "squareEnix" | "steam" | "xivLauncher" | "defaultLocation";

/** A detected game installation root with its `game/ffxivgame.ver`. */
export type GameInstallationDto = {
  path: string;
  gameVersion: string;
  origin: GameOrigin;
};

/** One package in Aeria's source-package store. */
export type SourcePackageEntryDto = {
  path: string;
  packageId: string;
  sourceLanguage: string;
  gameVersion: string;
  sizeBytes: number;
  builtAtUnixMs: number | null;
  /** Whether a build record allows reusing it instead of running Atlas. */
  reusable: boolean;
  /** Built from the installed game with the current Atlas. */
  current: boolean;
  /** Repository roots of recent projects, and the open project, that use it. */
  usedBy: string[];
  /** Nothing uses it and it is not current, so it can be deleted. */
  removable: boolean;
};

/** Whether a job finds a package without running Atlas. */
export type SourceAvailability = "ready" | "build" | "unknown";

/** The game installation setting and what it resolves to. */
export type GameSettingsDto = {
  /** The folder chosen in Settings; `null` uses the first detected installation. */
  configuredPath: string | null;
  /** The installation game operations use; `null` when none is usable. */
  active: GameInstallationDto | null;
  detected: GameInstallationDto[];
};

export type RecentProjectDto = {
  id: string;
  repositoryRoot: string;
  sourcePackagePath: string;
  sourcePackageId: string;
  sourceLanguage: string;
  targetLanguage: string;
  gameVersion: string;
  lastOpenedAtUnixMs: number;
  availability: RecentProjectAvailability;
};

export type TranslationOverlayDto = {
  translationUnitId: string;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type TranslationRowCursorDto = {
  sheetName: string;
  rowId: number;
  subrowId: number;
};

export type TranslationContextCellDto = {
  columnIndex: number;
  sourceMacro: string;
};

export type TranslationCellDto = {
  sourceBinding: SourceBinding;
  sourceMacro: string;
  /** The source has no letters outside macros: punctuation, digits, or number formatting. */
  formattingOnly: boolean;
  translation: TranslationOverlayDto | null;
};

export type TranslationRowDto = {
  sheetName: string;
  rowId: number;
  subrowId: number;
  context: TranslationContextCellDto[];
  cells: TranslationCellDto[];
};

export type TranslationRowPageDto = {
  rows: TranslationRowDto[];
  nextAfter: TranslationRowCursorDto | null;
};

export type TranslationUnitIdDto = {
  translationUnitId: string;
};

export type GitFileKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "copied"
  | "typeChanged"
  | "untracked"
  | "conflicted";

export type GitFileDto = {
  path: string;
  originalPath: string | null;
  kind: GitFileKind;
  staged: boolean;
  translationData: boolean;
};

export type GitStatusDto = {
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  mergeInProgress: boolean;
  hasTranslationChanges: boolean;
  files: GitFileDto[];
};

export type GitConfigScope = "system" | "global" | "repository" | "worktree" | "command";

/** The translator identity is the Git author identity. */
export type TranslatorIdentityDto = {
  name: string | null;
  email: string | null;
  nameScope: GitConfigScope | null;
  emailScope: GitConfigScope | null;
};

export type GitRemoteDto = {
  name: string;
  url: string;
};

export type GitRuntimeDto = {
  /** `git --version` output, or null when Git cannot be run. */
  version: string | null;
  origin: "system" | "bundled" | "override";
};


/** Changes reach the main branch only through pull requests. */
export type CollaborationDto = {
  /** The main branch set in aeria-collaboration.json, if any. */
  configuredMainBranch: string | null;
  /** The main branch in effect: configured or detected. */
  mainBranch: string | null;
  /** Why aeria-collaboration.json cannot be used. */
  error: string | null;
};

export type ContributionDto = {
  mainBranch: string;
  /** Null while on the main branch. */
  branch: string | null;
  published: boolean;
  unmergedCommits: number;
  /** No remote: the contribution is merged locally instead of through a pull request. */
  local: boolean;
};

export type GitOverviewDto = {
  runtime: GitRuntimeDto;
  repository: GitStatusDto | null;
  identity: TranslatorIdentityDto | null;
  remotes: GitRemoteDto[];
  collaboration: CollaborationDto | null;
  contribution: ContributionDto | null;
};

export type GitBranchDto = {
  name: string;
  remote: boolean;
  current: boolean;
  upstream: string | null;
  /** Every commit of the local branch is in the main branch. */
  merged: boolean;
  /** Why the open project cannot switch to this branch. */
  blocked: "noProject" | "olderFormat" | "otherSource" | null;
};

export type AttributionDto = {
  commit: string;
  authorName: string;
  authorEmail: string;
  authoredAt: number;
};

export type UnitAttributionDto = {
  translationUnitId: string;
  translatedBy: AttributionDto | null;
  reviewedBy: AttributionDto | null;
  lastChangedBy: AttributionDto | null;
};

export type GitCommitDto = {
  id: string;
  parents: string[];
  authorName: string;
  authorEmail: string;
  /** Seconds since the Unix epoch. */
  authoredAt: number;
  /** Branch and tag names at the commit: "HEAD -> main", "origin/main", "tag: harmonia/3". */
  refs: string[];
  subject: string;
};

export type UnitVersionDto = {
  sourceBinding: SourceBinding;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type UnitChangeKind = "added" | "modified" | "removed";

export type UnitChangeDto = {
  translationUnitId: string;
  kind: UnitChangeKind;
  before: UnitVersionDto | null;
  after: UnitVersionDto | null;
  targetChanged: boolean;
  reviewChanged: boolean;
  noteChanged: boolean;
};

export type RecordVersionDto =
  | { state: "absent" }
  | { state: "valid"; unit: UnitVersionDto }
  | { state: "invalid"; message: string };

export type UnitRevisionDto = {
  commit: GitCommitDto;
  kind: UnitChangeKind;
  before: RecordVersionDto;
  after: RecordVersionDto;
};

export type UnitHistoryDto = {
  translationUnitId: string;
  pending: UnitChangeDto | null;
  revisions: UnitRevisionDto[];
  truncated: boolean;
  translatedBy: AttributionDto | null;
  reviewedBy: AttributionDto | null;
};

export type ProjectArea = "glossary" | "guidance" | "packSettings" | "fontSettings" | "fontFile" | "collaboration" | "gitAttributes" | "feedWorkflow";

export type ProjectChangeDetailDto = {
  kind: UnitChangeKind;
  /** The term, the settings path ("fonts › MiedingerMid › source"), or "" for a guidance line. */
  label: string;
  before: string | null;
  after: string | null;
};

/** A readable change of a glossary, guidance, settings, or font file. */
export type ProjectChangeDto = {
  path: string;
  area: ProjectArea;
  kind: UnitChangeKind;
  details: ProjectChangeDetailDto[];
  truncated: boolean;
  unreadable: boolean;
  size: number;
};

export type GitCommitChangesDto = {
  commit: GitCommitDto;
  changes: UnitChangeDto[];
  projectChanges: ProjectChangeDto[];
  branchCreated: string | null;
};

export type ContributorDto = {
  name: string;
  email: string;
  translated: number;
  reviewed: number;
  lastAuthoredAt: number;
};

export type UnitConflictDto = {
  translationUnitId: string;
  base: UnitVersionDto | null;
  ours: UnitVersionDto | null;
  theirs: UnitVersionDto | null;
};

export type ConflictResolution = "ours" | "theirs";

export type UnitResolutionDto = {
  translationUnitId: string;
  resolution: ConflictResolution;
};

export type GitIntegration = "upToDate" | "fastForward" | "merged";

export type GitSyncDto = {
  integration: GitIntegration;
  pushed: boolean;
  workspaceChanged: boolean;
  /** When non-empty nothing was integrated; sync again with resolutions. */
  conflicts: UnitConflictDto[];
  /** Merged translations were reconciled with the current source and wait for a checkpoint. */
  reconciled: boolean;
};

export type GitFinishDto = {
  integration: GitIntegration;
  deletedBranch: string | null;
};

export type AiProviderKind = "openCodeGo" | "openRouter" | "custom" | "chatGpt";
export type ReasoningEffort = "minimal" | "low" | "medium" | "high" | "xhigh";
export type ApiKeyState = "stored" | "missing" | "unavailable";

export type AiModelConfig = {
  id: string;
  contextWindow: number | null;
  reasoningEfforts: ReasoningEffort[];
};

export type AiHeaderConfig = { name: string; value: string };

export type AiProviderDto = {
  id: string;
  kind: AiProviderKind;
  name: string;
  baseUrl: string;
  models: AiModelConfig[];
  sessionHeader: string | null;
  headers: AiHeaderConfig[];
  apiKey: ApiKeyState;
};

export type AiProviderPresetDto = {
  kind: AiProviderKind;
  name: string;
  baseUrl: string | null;
  sessionHeader: string | null;
};

export type AiModelSelection = {
  providerId: string;
  modelId: string;
  effort: ReasoningEffort | null;
};

export type AiSettingsDto = {
  providers: AiProviderDto[];
  agentModel: AiModelSelection | null;
  /** The model for translation-job workers; Angelica's model when null. */
  workerModel: AiModelSelection | null;
  /** Domains whose pages Angelica reads without asking. */
  webDomains: string[];
  presets: AiProviderPresetDto[];
};

export type AiProviderInput = {
  id: string | null;
  kind: AiProviderKind;
  name: string;
  baseUrl: string;
  models: AiModelConfig[];
  sessionHeader: string | null;
  headers: AiHeaderConfig[];
};

export type ChatGptLoginDto = { loginId: string; userCode: string; verificationUrl: string; browserOpened: boolean };

export type ChatGptLoginEventDto = { loginId: string; providerId: string; succeeded: boolean; code: string | null; message: string | null };

export type AiConnectionCheckDto = {
  latencyMs: number;
  model: string | null;
};

export type ChatToolCall = { id: string; name: string; arguments: string };

export type ChatMessage =
  | { role: "user"; content: string; automatic?: boolean }
  | { role: "assistant"; content: string; reasoning?: string; toolCalls?: ChatToolCall[] }
  | { role: "tool"; toolCallId: string; name: string; content: string };

export type AiUsage = { promptTokens: number; completionTokens: number };

export type ConversationDto = {
  id: string;
  title: string;
  model: AiModelSelection | null;
  messages: ChatMessage[];
  usage: AiUsage;
  running: boolean;
};

export type ConversationSummaryDto = { id: string; title: string; updatedAtUnixMs: number; running: boolean };

export type AgentEvent =
  | { type: "textDelta"; text: string }
  | { type: "reasoningDelta"; text: string }
  | { type: "responseFinished" }
  | { type: "toolStarted"; id: string; name: string; arguments: string }
  | { type: "toolFinished"; id: string; name: string; content: string; isError: boolean }
  | ({ type: "usage" } & AiUsage)
  | { type: "turnFinished"; outcome: "completed" | "roundLimit"; usage: AiUsage }
  | { type: "turnFailed"; code: string; message: string }
  | { type: "turnCancelled" };

export type AngelicaEventDto = { conversationId: string; event: AgentEvent };

export type UnitLocationDto = { sheet: string; row: number; subrow: number; column: number | null };

export type EditorContextDto = { sheet: string | null; selection: UnitLocationDto | null; unsavedDraft: boolean };

export type AgentMode = "chat" | "ask" | "autoDraft";

export type ProposalRecord = {
  id: string;
  /** The changed project file; null for a translation. */
  file: "guidance" | "glossary" | null;
  /** A job to start; `target` then holds its one-line summary. */
  job?: JobProposal | null;
  /** A domain Angelica asked to read; `target` then holds the link. */
  web?: string | null;
  /** Translations Angelica suggests marking reviewed; `target` holds her reason. */
  review?: { reason: string; items: { location: UnitLocationDto; source: string; target: string }[] } | null;
  location: UnitLocationDto | null;
  source: string;
  target: string;
  expected: { target: string | null; reviewState: ReviewState | null };
  status: "pending" | "applied" | "rejected" | "conflict" | "failed";
  message: string | null;
  createdAtUnixMs: number;
};

export type TranslationAppliedDto = { sourceBinding: SourceBinding; overlay: TranslationOverlayDto };

export type JobFilter = "untranslated" | "needsReview" | "untranslatedAndDrafts";

export type JobScope = { sheets: string[]; filter: JobFilter };

export type JobEstimate = { units: number; chunks: number; estimatedTokens: number };

export type JobProposal = { scope: JobScope; instructions: string; concurrency: number; estimate: JobEstimate; tokenLimit: number };

export type JobSpec = { scope: JobScope; instructions: string; model: AiModelSelection; tokenLimit: number; concurrency: number };

export type JobStatus = "running" | "paused" | "completed" | "cancelled";

export type JobUnitStatus = "pending" | "running" | "drafted" | "rejected" | "failed" | "conflict";

export type JobCounts = { total: number; pending: number; running: number; drafted: number; rejected: number; failed: number; conflict: number };

export type JobSummary = {
  id: string;
  conversationId: string;
  status: JobStatus;
  /** Why a paused job paused. */
  reason: string | null;
  spec: JobSpec;
  createdAtUnixMs: number;
  counts: JobCounts;
  /** Chunks being translated right now, one per busy worker. */
  activeWorkers: number;
  usage: AiUsage;
};

export type JobUnit = {
  seq: number;
  chunk: number;
  location: UnitLocationDto;
  status: JobUnitStatus;
  attempts: number;
  message: string | null;
  expected: { target: string | null; reviewState: ReviewState | null };
};

export type JobEvent = { seq: number; createdAtUnixMs: number; kind: string; message: string; location: UnitLocationDto | null };

export type JobAction = "pause" | "resume" | "cancel";

export type WorkerPhase = "idle" | "preparing" | "waiting" | "reasoning" | "writing" | "tool" | "recording" | "backoff" | "stopped";

/** What one lane of a running job is doing; live state, never stored. */
export type WorkerActivity = {
  lane: number;
  phase: WorkerPhase;
  chunk: number | null;
  sheet: string | null;
  units: number;
  finishedUnits: number;
  round: number;
  maxRounds: number;
  tool: string | null;
  chunkTokens: number;
  chunksDone: number;
  phaseStartedUnixMs: number;
  lastActivityUnixMs: number;
  retryAtUnixMs: number | null;
  lastError: string | null;
};

export type GlossaryEntry = { term: string; translation: string; note?: string; forbidden?: string[] };

export type GlossaryEntryInput = { term: string; translation: string; note: string | null; forbidden: string[] };

export type ProjectGuideDto = {
  /** The guidance text; null when the file does not exist. */
  guidance: string | null;
  /** The glossary file's exact content, sent back when saving. */
  glossaryText: string | null;
  entries: GlossaryEntry[];
  diagnostics: { line: number; message: string }[];
  guidanceError: string | null;
  glossaryError: string | null;
};

/** Project-shared pack identity in aeria-pack.json. */
export type PackSettings = {
  packId: string;
  title: string;
  publisherName: string;
  publisherUrl: string | null;
  license: string | null;
  minHarmonia: string;
};

export type ExportOverviewDto = {
  settings: (PackSettings & { signingKeyFingerprint: string | null }) | null;
  /** Why aeria-pack.json exists but cannot be used. */
  settingsError: string | null;
  key: { state: "stored" | "missing" | "unavailable"; fingerprint: string | null };
  project: {
    sourceLanguage: string;
    targetLanguage: string;
    gameVersion: string;
    commit: string | null;
    uncommitted: boolean;
    /** aeria-pack.json itself has uncommitted changes. */
    settingsUncommitted: boolean;
    /** What the export needs committed but is not. */
    uncommittedParts: ("translations" | "pack" | "fonts")[];
    upstream: string | null;
    ahead: number;
  };
  github: { owner: string; name: string; homepage: string; feedUrl: string } | null;
  workflow: "missing" | "current" | "different";
  /** The main branch on GitHub (as of the last fetch) has this Aeria's feed workflow. */
  workflowOnGithub: boolean;
  mainBranch: string | null;
  /** Highest harmonia/<n> release tag in the local repository. */
  latestReleaseTag: number | null;
  /** aeria-fonts.json exists. */
  fontsConfigured: boolean;
};

export type ReleaseChannel = "stable" | "testing";
export type ContentPolicy = "reviewed" | "all";

export type ReleaseInput = {
  sequence: number;
  version: string;
  channel: ReleaseChannel;
  contentPolicy: ContentPolicy;
  changelog: string | null;
};

export type ExportReportDto = {
  exported: number;
  skippedDetached: number;
  skippedUntranslated: number;
  skippedUnreviewed: number;
  skippedWithoutRawHash: number;
  sheets: number;
  strings: number;
  packHash: string;
  fontTargets: number;
  fontGlyphs: number;
  signedBy: string | null;
};

export type LocalExportDto = { path: string; report: ExportReportDto };
export type PublishedReleaseDto = { sequence: number; releaseUrl: string; feedUrl: string; report: ExportReportDto };

/** aeria-fonts.json: source fonts for glyphs the game fonts lack. */
export type FontCaseMapping = "none" | "upper";

export type FontSource = {
  id: string;
  file: string;
  family: string;
  copyright: string;
  license: string;
  licenseFile: string;
};

export type FontSizeOverride = {
  source?: string;
  axes?: Record<string, number>;
  scale?: number;
  widthScale?: number;
  baselineShift?: number;
  tracking?: number;
};

export type FontTarget = {
  font: string;
  source: string;
  axes: Record<string, number>;
  scale: number;
  widthScale: number;
  baselineShift: number;
  tracking: number;
  caseMapping: FontCaseMapping;
  sizes: Record<string, FontSizeOverride>;
};

export type FontSettings = {
  characters: string;
  sources: FontSource[];
  fonts: FontTarget[];
};

export type GameFontSizeDto = { size: string; lineHeight: number; ascent: number; capHeight: number; capAdvance: number; spaceAdvance: number };

export type FontsOverviewDto = {
  settings: FontSettings | null;
  /** Why aeria-fonts.json exists but cannot be used. */
  settingsError: string | null;
  /** aeria-fonts.json or fonts/ has uncommitted changes. */
  uncommitted: boolean;
  gameFonts: { name: string; sizes: GameFontSizeDto[] }[];
  sources: { id: string; error: string | null; axes: { tag: string; min: number; default: number; max: number }[] }[];
};

export type ImportedFontFileDto = { file: string; family: string | null; copyright: string | null; isFont: boolean };

export type FontPreviewSizeDto = {
  size: string;
  lineHeight: number;
  ascent: number;
  capHeight: number;
  capAdvance: number;
  generatedCapAdvance: number | null;
  width: number;
  height: number;
  pixels: number[];
  missing: string[];
  error: string | null;
};

export type UpdateChannel = "stable" | "nightly";

/** Work an application update waits for instead of interrupting. */
export type RunningActivity = "translation" | "sync" | "export" | "sourcePackage";

export type AvailableUpdateDto = {
  version: string;
  channel: UpdateChannel;
  notes: string | null;
  /** RFC 3339, as published in the feed. */
  publishedAt: string | null;
  releaseUrl: string;
};

export type UpdateStatusDto = {
  currentVersion: string;
  channel: UpdateChannel;
  /** Only an installed copy updates itself; portable copies link to the release. */
  canInstall: boolean;
  checking: boolean;
  /** Milliseconds since the Unix epoch. */
  lastCheckedAt: number | null;
  available: AvailableUpdateDto | null;
  download: { received: number; total: number | null; ready: boolean } | null;
  installing: boolean;
  error: CommandError | null;
  runningActivities: RunningActivity[];
};
