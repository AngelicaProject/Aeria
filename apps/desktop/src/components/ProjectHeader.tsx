import type { ProjectSummaryDto } from "../types";

type ProjectHeaderProps = {
  project: ProjectSummaryDto;
  closing: boolean;
  disabled: boolean;
  onClose: () => void;
};

export function ProjectHeader({ project, closing, disabled, onClose }: ProjectHeaderProps) {
  return (
    <header className="project-header">
      <div className="brand-lockup">
        <span className="brand-name">Aeria</span>
        <span className="brand-context">translation workspace</span>
      </div>
      <div className="project-summary" title={`${project.repositoryRoot}\n${project.sourcePackagePath}`}>
        <span className="language-pair">
          {project.sourceLanguage} <span aria-hidden="true">→</span> {project.targetLanguage}
        </span>
        <span className="project-meta">
          {project.gameVersion || "Unknown game version"} · {project.scope || "project"}
        </span>
      </div>
      <button className="secondary-button close-button" type="button" onClick={onClose} disabled={disabled}>
        {closing ? "Closing…" : "Close project"}
      </button>
    </header>
  );
}
