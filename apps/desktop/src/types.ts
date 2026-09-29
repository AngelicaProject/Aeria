export type CommandError = {
  code: string;
  message: string;
};

export type ReviewState = "draft" | "reviewed" | "needsReview";

export type SourceBinding = {
  sheetName: string;
  rowId: number;
  subrowId: number;
  columnIndex: number;
};

/** One source cell in another client language; `text` is null when that language has no such cell. */
export type OtherLanguageTextDto = {
  language: string;
  text: string | null;
};

/** The dialogue structure of a quest or cutscene sheet, read from its row keys. */
export type SheetDialogueDto = {
  kind: "quest" | "cutscene";
  quest: QuestNameDto | null;
  /** Other quest sheets whose `Quest` row has the same name. */
  versions: string[];
  lines: DialogueLineDto[];
  /** The scenes traced from the quest's script; `null` for cutscenes, quests without a script, or an unreadable script. */
  scenes: SceneFlowDto[] | null;
  /** Why the quest's script could not be read. */
  scriptError: string | null;
  /** Every cutscene file that names lines of the sheet, in `Cutscene` row order. */
  cutscenes: IndexedCutsceneDto[];
};

/** A cutscene file and the sheet's lines it names, in row order. */
export type IndexedCutsceneDto = { row: number; path: string; lines: string[]; plays: CutscenePlayDto[] };

/** A scene or handler of a quest's scripts that plays a cutscene: the quest's sheet and name, and where in its scripts. */
export type CutscenePlayDto = { quest: string; name: string | null; scene: number | null; handler: string | null; script: string | null };

export type SceneFlowDto = {
  /** The number of `OnScene<number>`; `null` for another function of the script. */
  scene: number | null;
  /** The name another function is assigned to, such as `GetBalloonTalkArgs`. */
  handler: string | null;
  /** The quest's battle script it belongs to, such as `ClsRog250Btl`; `null` for the quest's own script. */
  script: string | null;
  /** `false` when the scene lists its lines in code order, without branches. */
  traced: boolean;
  nodes: FlowNodeDto[];
};

/** One step of a scene; line keys follow the sheet's `TEXT_<ID>_` prefix, like `DialogueLineDto.key`. */
export type FlowNodeDto =
  | { kind: "line"; key: string }
  /** `prompts` are the possible questions: several when the script chooses one earlier. */
  | { kind: "choice"; id: number; choice: ChoiceKindDto; prompts: string[]; options: ChoiceOptionDto[] }
  | { kind: "branch"; condition: GuardDto; then: FlowNodeDto[]; otherwise: FlowNodeDto[] }
  | { kind: "loop"; id: number; body: FlowNodeDto[] }
  | { kind: "repeat"; loopId: number }
  /** `lines` are keys of this sheet the cutscene's file names, in row order; `sheets` are other dialogue sheets it names lines of. */
  | { kind: "cutscene"; name: string | null; path: string | null; lines: string[]; sheets: string[] }
  | { kind: "cancelled" }
  | { kind: "accepted" }
  | { kind: "completed" };

export type ChoiceKindDto = "questOffer" | "yesNo" | "menu";

export type ChoiceOptionDto = { label: OptionLabelDto; available: AvailabilityDto; then: FlowNodeDto[] };

/** When the player can pick an answer: `never` shows it grayed out; `when` only where the guard holds. */
export type AvailabilityDto =
  | { kind: "always" }
  | { kind: "never" }
  | { kind: "when"; guard: GuardDto }
  | { kind: "unknown" };

export type OptionLabelDto =
  | { kind: "accept" }
  | { kind: "decline" }
  | { kind: "yes" }
  | { kind: "no" }
  | { kind: "text"; key: string }
  /** An answer the script passes from elsewhere, such as an entry of a list it built. */
  | { kind: "script" };

/** One test of a value. */
export type ConditionDto = { kind: "test"; subject: OperandDto; test: TestDto };

/** What a branch tests: one condition, or several joined with `or` (`any`) or `and` (`all`). */
export type GuardDto = ConditionDto | { kind: "any"; guards: GuardDto[] } | { kind: "all"; guards: GuardDto[] };

export type TestDto =
  | { kind: "truthy"; value: boolean }
  | { kind: "compare"; comparison: "eq" | "ne" | "lt" | "le" | "gt" | "ge"; value: OperandDto };

export type OperandDto =
  | { kind: "answer"; choice: number }
  | { kind: "call"; function: string | null; arguments: OperandDto[] }
  | { kind: "number"; value: number }
  | { kind: "boolean"; value: boolean }
  | { kind: "nil" }
  | { kind: "string"; value: string }
  | { kind: "field"; name: string }
  /** A quest a script variable such as `QUEST0` names: its `Quest` row, name, and sheet when known. */
  | { kind: "quest"; variable: string; row: number; name: string | null; sheet: string | null }
  | { kind: "global"; name: string }
  | { kind: "oneOf"; operands: OperandDto[] }
  | { kind: "unknown" };

export type QuestNameDto = {
  sourceBinding: SourceBinding;
  sourceMacro: string;
  targetMacro: string | null;
};

export type DialogueLineDto = {
  sourceBinding: SourceBinding;
  role: "journal" | "objective" | "speech" | "other";
  /** The speaker label of speech, such as `URIANGER`, `SYSTEM`, or `A1`. */
  speaker: string | null;
  /** The row key after its `TEXT_<ID>_` prefix. */
  key: string;
  sourceMacro: string;
};

export type ProjectSheetDto = {
  name: string;
  rowCount: number;
  translatableCellCount: number;
  /** The sheet is listed by the game but cannot be read. */
  unavailable: boolean;
};

export type ProjectSummaryDto = {
  repositoryRoot: string;
  sourceLanguage: string;
  targetLanguage: string;
  /** The game version the project describes. */
  gameVersion: string;
  /** The game installation the project reads. */
  gamePath: string;
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

export type SheetLayoutUpdateDto = {
  sheetName: string;
  removed: boolean;
  /** The sheet still exists in the game but cannot be read. */
  unavailable: boolean;
  mappedColumns: number;
  unresolvedColumns: number;
};

/** Counts of a previewed or applied deterministic source update. */
export type SourceUpdateReportDto = {
  previousGameVersion: string;
  gameVersion: string;
  unchanged: number;
  sourceChanged: number;
  detached: number;
  newlyDetached: number;
  reattached: number;
  columnMapped: number;
  rowMoved: number;
  changedUnits: number;
  sheetLayoutUpdates: SheetLayoutUpdateDto[];
};

export type DetachedUnitDto = {
  translationUnitId: string;
  lastSourceBinding: SourceBinding;
  /** The source text the translation was last bound to. */
  lastSourceText: string;
  reason: DetachReason;
  targetMacro: string;
  reviewState: ReviewState;
  translatorNote: string | null;
};

export type RecentProjectAvailability = "ready" | "repositoryMissing";

/** Outcome of opening a project from a game installation. */
export type GameOpenResultDto =
  | { status: "opened"; result: ProjectOpenResultDto }
  | {
    /** Nothing was written; confirm, then update the project from the game. */
    status: "sourceUpdateRequired";
    report: SourceUpdateReportDto;
  };

export type GameOrigin = "settings" | "squareEnix" | "steam" | "xivLauncher" | "defaultLocation";

/** A detected game installation root with its `game/ffxivgame.ver`. */
export type GameInstallationDto = {
  path: string;
  gameVersion: string;
  origin: GameOrigin;
};

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
  /** Commits on the remote main branch, as last fetched, that this branch does not contain yet. */
  mainAhead: number;
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

export type ProjectArea = "glossary" | "guidance" | "packSettings" | "fontSettings" | "fontFile" | "collaboration" | "gitAttributes" | "feedWorkflow" | "checkWorkflow";

/** The GitHub workflow that runs aeria-check on pull requests. */
export type CheckWorkflowDto = {
  /** The origin remote is a github.com repository. */
  github: boolean;
  /** The project is the repository's top folder, where GitHub reads workflows. */
  topLevel: boolean;
  /** This build can write the workflow (release builds only). */
  available: boolean;
  state: "missing" | "current" | "different";
  branchSettingsUrl: string | null;
};

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
  /** Whether the model accepts images; omitted when it does not. */
  vision?: boolean;
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

/** An image attached to a message, stored beside its conversation. */
export type ImageRef = { id: string; format: "png" | "jpeg"; width: number; height: number };

export type ChatMessage =
  | { role: "user"; content: string; automatic?: boolean; images?: ImageRef[] }
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
  file: "guidance" | "glossary" | "voices" | null;
  /** A job to start; `target` then holds its one-line summary. */
  job?: JobProposal | null;
  /** A domain Angelica asked to read; `target` then holds the link. */
  web?: string | null;
  /** Translations Angelica suggests marking reviewed; `target` holds her reason. */
  review?: { reason: string; items: { location: UnitLocationDto; source: string; target: string }[] } | null;
  /** A new token limit for a job; `target` holds its one-line summary. */
  jobLimit?: JobLimitProposal | null;
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

export type JobEstimate = {
  units: number;
  chunks: number;
  estimatedTokens: number;
  /** Average tokens per chunk of earlier jobs with the same model, when the estimate uses it. */
  historyChunkTokens?: number;
};

export type JobLimitProposal = { jobId: string; tokenLimit: number; previousLimit: number; usedTokens: number; projectedTokens: number | null };

/** How much work the localizer spends on each unit of a job. */
export type JobQuality = "fast" | "careful";

export type JobProposal = { scope: JobScope; instructions: string; concurrency: number; estimate: JobEstimate; tokenLimit: number; quality?: JobQuality };

export type JobSpec = { scope: JobScope; instructions: string; model: AiModelSelection; tokenLimit: number; concurrency: number; quality?: JobQuality };

export type JobStatus = "running" | "paused" | "completed" | "cancelled";

export type JobUnitStatus = "pending" | "running" | "drafted" | "finished" | "flagged" | "rejected" | "failed" | "conflict";

export type JobCounts = { total: number; pending: number; running: number; drafted: number; finished: number; flagged: number; rejected: number; failed: number; conflict: number };

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
  chunks: number;
  /** Chunks with no pending or running strings. */
  finishedChunks: number;
  /** Tokens the whole job will likely use; null until a chunk finished. */
  projectedTokens: number | null;
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

export type WorkerPhase = "idle" | "preparing" | "waiting" | "reasoning" | "writing" | "recording" | "backoff" | "stopped";

/** The localizer step a lane is on. */
export type WorkerStep = "study" | "learning" | "terms" | "contract" | "writing" | "reviewing" | "fixing" | "rechecking";

/** What one lane of a running job is doing; live state, never stored. */
export type WorkerActivity = {
  lane: number;
  phase: WorkerPhase;
  chunk: number | null;
  sheet: string | null;
  units: number;
  finishedUnits: number;
  /** The localizer step, while a chunk runs. */
  step: WorkerStep | null;
  /** The step's number, from 1, and the number of steps. */
  round: number;
  maxRounds: number;
  /** Requests of the current step that have not answered yet. */
  requests: number;
  chunkTokens: number;
  chunksDone: number;
  phaseStartedUnixMs: number;
  lastActivityUnixMs: number;
  retryAtUnixMs: number | null;
  lastError: string | null;
  /** The first and last source row of the current chunk. */
  firstRow: number | null;
  lastRow: number | null;
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
  /** The voice profile text; null when the file does not exist. */
  voices: string | null;
  /** Profiles the voice file ignores, with the reason. */
  voiceDiagnostics: { line: number; message: string }[];
  voicesError: string | null;
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
export type RunningActivity = "translation" | "sync" | "export";

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

/** A problem in macro text, in UTF-16 offsets. */
export type MacroDiagnosticDto = { from: number; to: number; message: string };

export type MacroParameterDto = { prefix: "n" | "s" | "gn" | "gs"; index: number };

export type MacroTagPart = "inline" | "open" | "close" | "separator" | "generic" | "raw";

export type MacroFamily = "translatableText" | "formatting" | "condition" | "runtimeValue" | "gameData" | "layout" | "opaque";

export type MacroArgDto = { name: string; role: string; value: string; parameter: MacroParameterDto | null };

/** One tag of macro text: an opening, closing, separator, or inline tag. */
/** A macro a translator can insert, in the form game strings use; `{row}` in `parts` is one of `rows`. */
export type MacroInsertionDto = {
  name: string;
  group: "player" | "choice" | "format";
  form: "insert" | "wrap" | "branches";
  parts: string[];
  rows: { row: number; name: string }[];
  summary: string;
};

/** A construct of several macros that reads as one value, such as the player's first name. */
export type MacroIdiomDto = { name: string; text: string; summary: string };

export type MacroTagDto = {
  from: number;
  to: number;
  name: string;
  part: MacroTagPart;
  family: MacroFamily | null;
  args: MacroArgDto[];
  /** The color an opening color tag sets, `#rrggbbaa`, when it is known. */
  color: string | null;
  /** What an opening `<if>` or `<switch>` tests, part by part. */
  condition: MacroConditionDto | null;
};

/** A number (named by its row when compared with a row global such as a class), a parameter, a time value, or other text. */
export type MacroOperandDto =
  | { kind: "int"; value: number; name: string | null }
  | { kind: "parameter"; code: string; meaning: string | null }
  | { kind: "time"; name: string }
  | { kind: "other"; text: string };

/** A condition of an `<if>` or the value of a `<switch>`; `left` alone tests for not zero or empty. */
export type MacroConditionDto = {
  left: MacroOperandDto;
  operator: "==" | "!=" | "<" | "<=" | ">" | ">=" | null;
  right: MacroOperandDto | null;
};

export type MacroViewDto = {
  diagnostics: MacroDiagnosticDto[];
  tags: MacroTagDto[];
};
