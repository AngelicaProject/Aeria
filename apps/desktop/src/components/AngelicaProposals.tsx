import { diffWords } from "../textDiff";
import type { ProposalRecord, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import type { MessageKey } from "../i18n/translate";

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

  return (
    <details className="angelica-proposals" open>
      <summary>
        <span>{t("angelica.proposals", { count: pending.length })}</span>
        {pending.length > 1 ? (
          <span className="angelica-proposals-bulk">
            <button className="button button-ghost" type="button" disabled={busy} onClick={(event) => { event.preventDefault(); onReject(pending); }}>{t("angelica.proposal.rejectAll")}</button>
            <button className="button button-secondary" type="button" disabled={busy} onClick={(event) => { event.preventDefault(); onApply(pending); }}>{t("angelica.proposal.applyAll")}</button>
          </span>
        ) : null}
      </summary>
      <ul>
        {shown.map((proposal) => {
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
