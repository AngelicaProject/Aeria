import { diffWords } from "../textDiff";
import type { JobProposal, ProposalRecord, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import type { MessageKey } from "../i18n/translate";
import { jobFilterLabels } from "./AngelicaJobs";

type AngelicaProposalsProps = {
  proposals: ProposalRecord[];
  busy: boolean;
  onApply: (ids: string[]) => void;
  onReject: (ids: string[]) => void;
  onReveal?: ((binding: SourceBinding) => void) | undefined;
};

const statusLabels: Readonly<Record<ProposalRecord["status"], MessageKey>> = {
  pending: "angelica.proposal.pending",
  applied: "angelica.proposal.applied",
  rejected: "angelica.proposal.rejected",
  conflict: "angelica.proposal.conflict",
  failed: "angelica.proposal.failed",
};

export function bindingOf(proposal: ProposalRecord): SourceBinding | null {
  const location = proposal.location;
  return location ? { sheetName: location.sheet, rowId: location.row, subrowId: location.subrow, columnIndex: location.column ?? 0 } : null;
}

const fileNames: Readonly<Record<NonNullable<ProposalRecord["file"]>, string>> = {
  guidance: "aeria-guidance.md",
  glossary: "aeria-glossary.csv",
};

/** Pending proposals and the ones that could not be applied. */
export function AngelicaProposals({ proposals, busy, onApply, onReject, onReveal }: AngelicaProposalsProps) {
  const { t } = useI18n();
  const shown = proposals.filter((proposal) => proposal.status === "pending" || proposal.status === "conflict" || proposal.status === "failed");
  if (shown.length === 0) return null;
  const pending = shown.filter((proposal) => proposal.status === "pending").map((proposal) => proposal.id);
  // Jobs start one by one, never with the translations in bulk.
  const bulk = shown.filter((proposal) => proposal.status === "pending" && !proposal.job && !proposal.web && !proposal.review).map((proposal) => proposal.id);

  return (
    <details className="angelica-proposals" open>
      <summary>
        <span>{t("angelica.proposals", { count: pending.length })}</span>
        {bulk.length > 1 ? (
          <span className="angelica-proposals-bulk">
            <button className="button button-ghost" type="button" disabled={busy} onClick={(event) => { event.preventDefault(); onReject(bulk); }}>{t("angelica.proposal.rejectAll")}</button>
            <button className="button button-secondary" type="button" disabled={busy} onClick={(event) => { event.preventDefault(); onApply(bulk); }}>{t("angelica.proposal.applyAll")}</button>
          </span>
        ) : null}
      </summary>
      <ul>
        {shown.map((proposal) => {
          if (proposal.review) return <ReviewProposalCard key={proposal.id} proposal={proposal} review={proposal.review} busy={busy} onApply={onApply} onReject={onReject} onReveal={onReveal} />;
          if (proposal.web) return <WebProposalCard key={proposal.id} proposal={proposal} domain={proposal.web} busy={busy} onApply={onApply} onReject={onReject} />;
          if (proposal.job) return <JobProposalCard key={proposal.id} proposal={proposal} job={proposal.job} busy={busy} onApply={onApply} onReject={onReject} />;
          const binding = bindingOf(proposal);
          const location = binding ? `${binding.sheetName}:${binding.rowId}:${binding.subrowId}:${binding.columnIndex}` : null;
          return (
            <li key={proposal.id} className={`angelica-proposal ${proposal.status}`}>
              <div className="angelica-proposal-head">
                {binding && location
                  ? <button className="link-button mono" type="button" onClick={() => onReveal?.(binding)}>{location}</button>
                  : <span className="mono">{proposal.file ? t("angelica.proposal.file", { file: fileNames[proposal.file] }) : ""}</span>}
                {proposal.expected.reviewState === "reviewed" ? <span className="angelica-chip">{t("angelica.proposal.replacesReviewed")}</span> : null}
                {proposal.status !== "pending" ? <span className="angelica-chip">{t(statusLabels[proposal.status])}</span> : null}
              </div>
              <div className={proposal.file ? "angelica-proposal-diff file" : "angelica-proposal-diff"}>
                {proposal.expected.target === null
                  ? <ins>{proposal.target}</ins>
                  : diffWords(proposal.expected.target, proposal.target).map((part, index) => part.kind === "same"
                    ? <span key={index}>{part.text}</span>
                    : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <del key={index}>{part.text}</del>)}
              </div>
              {proposal.message ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{proposal.message}</p> : null}
              {proposal.status === "pending" ? (
                <div className="angelica-proposal-actions">
                  <button className="button button-ghost" type="button" disabled={busy} onClick={() => onReject([proposal.id])}>{t("angelica.proposal.reject")}</button>
                  <button className="button button-primary" type="button" disabled={busy} onClick={() => onApply([proposal.id])}>{t("angelica.proposal.apply")}</button>
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </details>
  );
}

function JobProposalCard({ proposal, job, busy, onApply, onReject }: { proposal: ProposalRecord; job: JobProposal; busy: boolean; onApply: (ids: string[]) => void; onReject: (ids: string[]) => void }) {
  const { t } = useI18n();
  const sheets = job.scope.sheets.length > 0 ? job.scope.sheets.join(", ") : t("angelica.job.allSheets");
  return (
    <li className={`angelica-proposal angelica-job-proposal ${proposal.status}`}>
      <div className="angelica-proposal-head">
        <UiIcon icon="sparkles" size="xs" />
        <strong>{t("angelica.job.proposal")}</strong>
        {proposal.status !== "pending" ? <span className="angelica-chip">{t(statusLabels[proposal.status])}</span> : null}
      </div>
      <dl className="angelica-job-facts">
        <dt>{t("angelica.job.sheets")}</dt><dd>{sheets}</dd>
        <dt>{t("angelica.job.strings")}</dt><dd>{t(jobFilterLabels[job.scope.filter])}: {job.estimate.units}</dd>
        <dt>{t("angelica.job.estimate")}</dt><dd>{t("angelica.job.estimateValue", { chunks: job.estimate.chunks, tokens: job.estimate.estimatedTokens, limit: job.tokenLimit })}</dd>
        {job.instructions ? <><dt>{t("angelica.job.instructions")}</dt><dd>{job.instructions}</dd></> : null}
      </dl>
      <p className="field-hint">{t("angelica.job.proposalHint")}</p>
      {proposal.message ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{proposal.message}</p> : null}
      {proposal.status === "pending" ? (
        <div className="angelica-proposal-actions">
          <button className="button button-ghost" type="button" disabled={busy} onClick={() => onReject([proposal.id])}>{t("angelica.proposal.reject")}</button>
          <button className="button button-primary" type="button" disabled={busy} onClick={() => onApply([proposal.id])}><UiIcon icon="play" size="sm" />{t("angelica.job.start")}</button>
        </div>
      ) : null}
    </li>
  );
}

function WebProposalCard({ proposal, domain, busy, onApply, onReject }: { proposal: ProposalRecord; domain: string; busy: boolean; onApply: (ids: string[]) => void; onReject: (ids: string[]) => void }) {
  const { t } = useI18n();
  return (
    <li className={`angelica-proposal angelica-web-proposal ${proposal.status}`}>
      <div className="angelica-proposal-head">
        <UiIcon icon="externalLink" size="xs" />
        <strong>{t("angelica.web.request", { domain })}</strong>
        {proposal.status !== "pending" ? <span className="angelica-chip">{t(statusLabels[proposal.status])}</span> : null}
      </div>
      <code className="angelica-web-link">{proposal.target}</code>
      <p className="field-hint">{t("angelica.web.hint")}</p>
      {proposal.message ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{proposal.message}</p> : null}
      {proposal.status === "pending" ? (
        <div className="angelica-proposal-actions">
          <button className="button button-ghost" type="button" disabled={busy} onClick={() => onReject([proposal.id])}>{t("angelica.proposal.reject")}</button>
          <button className="button button-primary" type="button" disabled={busy} onClick={() => onApply([proposal.id])}>{t("angelica.web.allow")}</button>
        </div>
      ) : null}
    </li>
  );
}

function ReviewProposalCard({ proposal, review, busy, onApply, onReject, onReveal }: { proposal: ProposalRecord; review: NonNullable<ProposalRecord["review"]>; busy: boolean; onApply: (ids: string[]) => void; onReject: (ids: string[]) => void; onReveal?: ((binding: SourceBinding) => void) | undefined }) {
  const { t } = useI18n();
  return (
    <li className={`angelica-proposal angelica-review-proposal ${proposal.status}`}>
      <div className="angelica-proposal-head">
        <UiIcon icon="check" size="xs" />
        <strong>{t("angelica.review.title", { count: review.items.length })}</strong>
        {proposal.status !== "pending" ? <span className="angelica-chip">{t(statusLabels[proposal.status])}</span> : null}
      </div>
      <p className="angelica-review-reason">{review.reason}</p>
      <details className="angelica-review-items">
        <summary>{t("angelica.review.show")}</summary>
        <ul>
          {review.items.map((item) => {
            const binding: SourceBinding = { sheetName: item.location.sheet, rowId: item.location.row, subrowId: item.location.subrow, columnIndex: item.location.column ?? 0 };
            return (
              <li key={`${binding.sheetName}:${binding.rowId}:${binding.subrowId}:${binding.columnIndex}`}>
                <button className="link-button mono" type="button" onClick={() => onReveal?.(binding)}>{`${binding.sheetName}:${binding.rowId}:${binding.subrowId}:${binding.columnIndex}`}</button>
                <span className="angelica-review-source">{item.source}</span>
                <span className="angelica-review-target">{item.target}</span>
              </li>
            );
          })}
        </ul>
      </details>
      {proposal.message ? <p className={proposal.status === "applied" ? "field-hint" : "ai-test-result failed"}>{proposal.message}</p> : null}
      {proposal.status === "pending" ? (
        <div className="angelica-proposal-actions">
          <button className="button button-ghost" type="button" disabled={busy} onClick={() => onReject([proposal.id])}>{t("angelica.proposal.reject")}</button>
          <button className="button button-primary" type="button" disabled={busy} onClick={() => onApply([proposal.id])}><UiIcon icon="check" size="sm" />{t("angelica.review.approve", { count: review.items.length })}</button>
        </div>
      ) : null}
    </li>
  );
}
