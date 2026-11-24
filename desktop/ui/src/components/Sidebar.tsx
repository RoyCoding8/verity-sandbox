import { useState, type FormEvent, type KeyboardEvent } from "react";
import "./Sidebar.css";
import type { PiProject, PiSessionSummary } from "../api/tauri";
import type { ThemeMode } from "../theme/md3";
import {
  CloseIcon,
  CopyIcon,
  DownloadIcon,
  EditIcon,
  FolderIcon,
  GitBranchIcon,
  GitForkIcon,
  MessageIcon,
  MoonIcon,
  PlusIcon,
  SettingsIcon,
  SunIcon,
  TerminalIcon,
  VerityLogoMark,
} from "./Icons";

export interface SidebarProps {
  readonly projects: readonly PiProject[];
  readonly activeProjectId: string;
  readonly sessions: readonly PiSessionSummary[];
  readonly activeSessionId: string;
  readonly theme: ThemeMode;
  readonly isDesktop: boolean;
  readonly onSelectProject: (projectPath: string) => void;
  readonly onPickAndAddProject: () => void;
  readonly onAddProject: (projectPath: string) => void;
  readonly onRemoveProject: (projectPath: string) => void;
  readonly onSelectSession: (session: PiSessionSummary) => void;
  readonly onCreateSession: () => void;
  readonly onRenameSession: (sessionId: string, name: string) => void;
  readonly onOpenHistoryTree: () => void;
  readonly onOpenFork: () => void;
  readonly onCloneSession: () => void;
  readonly onExportHtml: () => void;
  readonly onOpenProviders: () => void;
  readonly onToggleTheme: () => void;
}

export function Sidebar({
  projects,
  activeProjectId,
  sessions,
  activeSessionId,
  theme,
  isDesktop,
  onSelectProject,
  onPickAndAddProject,
  onAddProject,
  onRemoveProject,
  onSelectSession,
  onCreateSession,
  onRenameSession,
  onOpenHistoryTree,
  onOpenFork,
  onCloneSession,
  onExportHtml,
  onOpenProviders,
  onToggleTheme,
}: SidebarProps) {
  const [expanded, setExpanded] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const [manualPathOpen, setManualPathOpen] = useState(false);
  const [manualPathValue, setManualPathValue] = useState("");

  function beginRename(session: PiSessionSummary) {
    setEditingId(session.id);
    setEditValue(session.name);
  }

  function commitRename(sessionId: string) {
    const trimmed = editValue.trim();
    setEditingId(null);
    if (trimmed) {
      onRenameSession(sessionId, trimmed);
    }
  }

  function handleRenameKeyDown(
    e: KeyboardEvent<HTMLInputElement>,
    sessionId: string
  ) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitRename(sessionId);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditingId(null);
    }
  }

  function handleManualPathSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const trimmed = manualPathValue.trim();
    if (!trimmed) return;
    onAddProject(trimmed);
    setManualPathValue("");
    setManualPathOpen(false);
  }

  function handleManualPathKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      e.preventDefault();
      setManualPathOpen(false);
      setManualPathValue("");
    }
  }

  const themeLabel =
    theme === "dark" ? "Switch to light theme" : "Switch to dark theme";

  return (
    <aside
      className={
        expanded
          ? "verity-sidebar verity-sidebar--expanded"
          : "verity-sidebar"
      }
      aria-label="Verity workspace navigation"
    >
      <div className="verity-sidebar-header">
        <button
          type="button"
          className="verity-rail-logo-btn"
          aria-expanded={expanded}
          aria-label={expanded ? "Collapse navigation rail" : "Expand navigation rail"}
          title={expanded ? "Collapse navigation" : "Expand navigation"}
          onClick={() => setExpanded((prev) => !prev)}
        >
          <VerityLogoMark size={22} />
          {expanded && <span className="verity-brand-title">Verity</span>}
        </button>
        {expanded && (
          <span className="verity-brand-tag">
            {isDesktop ? "Desktop" : "Preview"}
          </span>
        )}
      </div>

      {expanded && (
        <section className="verity-project-block" aria-label="Projects">
          <div className="verity-project-header">
            <span className="verity-project-header-title">Projects</span>
            <div className="verity-project-header-actions">
              <button
                type="button"
                className={
                  manualPathOpen
                    ? "verity-project-icon-btn verity-project-icon-btn--active"
                    : "verity-project-icon-btn"
                }
                title="Enter project path manually"
                aria-label="Enter project path manually"
                aria-pressed={manualPathOpen}
                onClick={() => setManualPathOpen((prev) => !prev)}
              >
                <TerminalIcon size={12} />
              </button>
              <button
                type="button"
                className="verity-project-add-btn"
                title="Add project folder"
                aria-label="Add project"
                onClick={onPickAndAddProject}
              >
                <PlusIcon size={12} />
                <span>Add Project</span>
              </button>
            </div>
          </div>

          {manualPathOpen && (
            <form className="verity-project-path-form" onSubmit={handleManualPathSubmit}>
              <input
                type="text"
                className="verity-project-path-input"
                aria-label="Project folder path"
                placeholder="/path/to/project"
                value={manualPathValue}
                autoFocus
                onChange={(e) => setManualPathValue(e.target.value)}
                onKeyDown={handleManualPathKeyDown}
              />
            </form>
          )}

          <div className="verity-project-list" role="list">
            {projects.map((project) => {
              const isActive = project.id === activeProjectId;
              return (
                <div
                  key={project.id}
                  role="listitem"
                  tabIndex={0}
                  title={project.path}
                  aria-current={isActive ? "true" : undefined}
                  className={
                    isActive
                      ? "verity-project-item active"
                      : "verity-project-item"
                  }
                  onClick={() => {
                    if (!isActive) {
                      onSelectProject(project.path);
                    }
                  }}
                  onKeyDown={(e) => {
                    if (e.target !== e.currentTarget) return;
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      if (!isActive) {
                        onSelectProject(project.path);
                      }
                    }
                  }}
                >
                  <span className="verity-project-item-main">
                    <FolderIcon size={14} className="verity-project-item-icon" />
                    <span className="verity-project-item-name">{project.name}</span>
                  </span>
                  {projects.length > 1 && (
                    <button
                      type="button"
                      className="verity-project-remove-btn"
                      title={`Remove ${project.name}`}
                      aria-label={`Remove ${project.name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        onRemoveProject(project.path);
                      }}
                    >
                      <CloseIcon size={12} />
                    </button>
                  )}
                </div>
              );
            })}
          </div>
        </section>
      )}

      <nav className="verity-rail-nav" aria-label="Session actions">
        <button
          type="button"
          className="verity-rail-btn"
          title="New session"
          aria-label="New session"
          onClick={onCreateSession}
        >
          <PlusIcon size={18} />
          {expanded && <span className="verity-rail-btn-label">New session</span>}
        </button>

        <button
          type="button"
          className={
            expanded
              ? "verity-rail-btn verity-rail-btn--active"
              : "verity-rail-btn"
          }
          title={expanded ? "Hide sessions" : "Sessions"}
          aria-label={expanded ? "Hide sessions" : "Show sessions"}
          aria-pressed={expanded}
          onClick={() => setExpanded((prev) => !prev)}
        >
          <MessageIcon size={17} />
          {expanded && <span className="verity-rail-btn-label">Sessions</span>}
        </button>

        <button
          type="button"
          className="verity-rail-btn"
          title="History tree"
          aria-label="Open history tree"
          onClick={onOpenHistoryTree}
        >
          <GitBranchIcon size={17} />
          {expanded && <span className="verity-rail-btn-label">History tree</span>}
        </button>

        <button
          type="button"
          className="verity-rail-btn"
          title="Fork session"
          aria-label="Fork session"
          onClick={onOpenFork}
        >
          <GitForkIcon size={17} />
          {expanded && <span className="verity-rail-btn-label">Fork session</span>}
        </button>

        {expanded && (
          <button
            type="button"
            className="verity-rail-btn"
            title="Clone session"
            aria-label="Clone session"
            onClick={onCloneSession}
          >
            <CopyIcon size={16} />
            <span className="verity-rail-btn-label">Clone session</span>
          </button>
        )}

        <button
          type="button"
          className="verity-rail-btn"
          title="Export HTML"
          aria-label="Export session as HTML"
          onClick={onExportHtml}
        >
          <DownloadIcon size={17} />
          {expanded && <span className="verity-rail-btn-label">Export HTML</span>}
        </button>
      </nav>

      {expanded ? (
        <>
          <div className="verity-sidebar-section-label">
            <span>Recent</span>
            <span className="verity-session-count">{sessions.length}</span>
          </div>

          <div className="verity-session-list" role="list">
            {sessions.length === 0 ? (
              <div className="verity-session-empty">No sessions yet</div>
            ) : (
              sessions.map((session) => {
                const isActive = session.id === activeSessionId;
                const isEditing = editingId === session.id;
                return (
                  <div
                    key={session.id}
                    role="listitem"
                    tabIndex={isEditing ? -1 : 0}
                    aria-current={isActive ? "page" : undefined}
                    className={
                      isActive
                        ? "verity-session-item active"
                        : "verity-session-item"
                    }
                    onClick={() => {
                      if (!isEditing && !isActive) onSelectSession(session);
                    }}
                    onKeyDown={(e) => {
                      if (isEditing || e.target !== e.currentTarget) return;
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        if (!isActive) onSelectSession(session);
                      }
                    }}
                    onDoubleClick={() => beginRename(session)}
                  >
                    <div className="verity-session-main">
                      {isEditing ? (
                        <input
                          className="verity-inline-rename"
                          aria-label="Rename session"
                          value={editValue}
                          autoFocus
                          onClick={(e) => e.stopPropagation()}
                          onChange={(e) => setEditValue(e.target.value)}
                          onBlur={() => commitRename(session.id)}
                          onKeyDown={(e) => handleRenameKeyDown(e, session.id)}
                        />
                      ) : (
                        <div className="verity-session-title">{session.name}</div>
                      )}
                    </div>
                    {!isEditing && (
                      <div className="verity-session-actions">
                        <button
                          type="button"
                          className="md3-icon-btn md3-icon-btn-sm"
                          title="Rename session"
                          aria-label={`Rename ${session.name}`}
                          onClick={(e) => {
                            e.stopPropagation();
                            beginRename(session);
                          }}
                        >
                          <EditIcon size={13} />
                        </button>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        </>
      ) : (
        <div className="verity-rail-spacer" />
      )}

      <div className="verity-sidebar-footer">
        <button
          type="button"
          className="verity-rail-btn verity-footer-settings-btn"
          title="Provider settings"
          aria-label="Provider settings"
          onClick={onOpenProviders}
        >
          <SettingsIcon size={17} />
          {expanded && (
            <span className="verity-rail-btn-label">Provider settings</span>
          )}
        </button>

        <button
          type="button"
          className="verity-rail-btn verity-footer-theme-btn"
          title={themeLabel}
          aria-label={themeLabel}
          onClick={onToggleTheme}
        >
          {theme === "dark" ? <SunIcon size={17} /> : <MoonIcon size={17} />}
        </button>
      </div>
    </aside>
  );
}
