import type { ProjectSummaryDto } from "../types";
import { displayPath } from "../pathDisplay";

type ProjectHeaderProps = {
  project: ProjectSummaryDto;
  closing: boolean;
  disabled: boolean;
  onClose: () => void;
};

export function ProjectHeader({ project, closing, disabled, onClose }: ProjectHeaderProps) {
  return (
    <header className="project-header">
      <div className="project-summary" title={`${displayPath(project.repositoryRoot)}\n${displayPath(project.sourcePackagePath)}`}>
        <strong className="project-path">{displayPath(project.repositoryRoot)}</strong>
      </div>
      <div className="project-facts">
        <span className="language-pair">Source {project.sourceLanguage}</span>
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
