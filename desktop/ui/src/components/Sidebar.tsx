import { useMemo, useState, type FormEvent, type KeyboardEvent } from "react";
import "./Sidebar.css";
import type { PiProject, PiSessionSummary } from "../api/tauri";
import type { ThemeMode } from "../theme/md3";
import {
  CheckIcon,
  ChevronRightIcon,
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
  TrashIcon,
  VerityLogoMark,
} from "./Icons";

function ArchiveIcon({ size = 13 }: { readonly size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect width="20" height="5" x="2" y="3" rx="1" />
      <path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" />
      <path d="M10 12h4" />
    </svg>
  );
}

function UnarchiveIcon({ size = 13 }: { readonly size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect width="20" height="5" x="2" y="3" rx="1" />
      <path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" />
      <path d="M12 17v-5" />
      <path d="m9.5 14.5 2.5-2.5 2.5 2.5" />
    </svg>
  );
}

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
  readonly onCreateSession: (projectPath?: string) => void;
  readonly onRenameSession: (sessionId: string, name: string) => void;
  readonly onArchiveSession: (sessionId: string, archived: boolean) => void;
  readonly onDeleteSession: (sessionId: string) => void;
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
  onArchiveSession,
  onDeleteSession,
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
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [collapsedProjects, setCollapsedProjects] = useState<Readonly<Record<string, boolean>>>({});
  const [archivedExpanded, setArchivedExpanded] = useState(false);
  const [manualPathOpen, setManualPathOpen] = useState(false);
  const [manualPathValue, setManualPathValue] = useState("");

  const archivedSessions = useMemo(
    () => sessions.filter((s) => Boolean(s.archived)),
    [sessions]
  );

  const projectNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const project of projects) {
      map.set(project.id, project.name);
    }
    return map;
  }, [projects]);

  function beginRename(session: PiSessionSummary) {
    setConfirmDeleteId(null);
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

  function toggleProjectCollapse(projectId: string) {
    setCollapsedProjects((prev) => ({
      ...prev,
      [projectId]: !prev[projectId],
    }));
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

  function renderSessionRow(session: PiSessionSummary, isArchivedList: boolean) {
    const isActive = session.id === activeSessionId;
    const isEditing = !isArchivedList && editingId === session.id;
    const isConfirmingDelete = confirmDeleteId === session.id;
    const projectLabel = isArchivedList ? projectNameById.get(session.projectId) : undefined;

    return (
      <div
        key={session.id}
        role="listitem"
        tabIndex={isEditing ? -1 : 0}
        aria-current={isActive ? "page" : undefined}
        className={[
          "verity-session-item",
          isActive ? "active" : "",
          isArchivedList ? "verity-session-item--archived" : "",
          isConfirmingDelete ? "verity-session-item--confirming" : "",
        ]
          .filter(Boolean)
          .join(" ")}
        onClick={() => {
          if (!isEditing && !isConfirmingDelete && !isActive) {
            onSelectSession(session);
          }
        }}
        onKeyDown={(e) => {
          if (isEditing || isConfirmingDelete || e.target !== e.currentTarget) return;
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            if (!isActive) onSelectSession(session);
          }
        }}
        onDoubleClick={() => {
          if (!isArchivedList && !isConfirmingDelete) {
            beginRename(session);
          }
        }}
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
            <>
              <div className="verity-session-title" title={session.name}>
                {session.name}
              </div>
              {projectLabel && (
                <div className="verity-session-subtitle">{projectLabel}</div>
              )}
            </>
          )}
        </div>

        {!isEditing && isConfirmingDelete && (
          <div
            className="verity-session-delete-confirm"
            onClick={(e) => e.stopPropagation()}
          >
            <span className="verity-delete-confirm-label">Delete?</span>
            <button
              type="button"
              className="verity-session-action-btn verity-session-action-btn--danger"
              title="Confirm delete chat"
              aria-label={`Confirm delete ${session.name}`}
              onClick={() => {
                setConfirmDeleteId(null);
                onDeleteSession(session.id);
              }}
            >
              <CheckIcon size={12} />
            </button>
            <button
              type="button"
              className="verity-session-action-btn"
              title="Cancel delete"
              aria-label="Cancel delete"
              onClick={() => setConfirmDeleteId(null)}
            >
              <CloseIcon size={12} />
            </button>
          </div>
        )}

        {!isEditing && !isConfirmingDelete && (
          <div className="verity-session-actions" onClick={(e) => e.stopPropagation()}>
            {!isArchivedList && (
              <button
                type="button"
                className="verity-session-action-btn"
                title="Rename chat"
                aria-label={`Rename ${session.name}`}
                onClick={() => beginRename(session)}
              >
                <EditIcon size={12} />
              </button>
            )}
            <button
              type="button"
              className="verity-session-action-btn"
              title={isArchivedList ? "Restore chat" : "Archive chat"}
              aria-label={
                isArchivedList
                  ? `Restore ${session.name}`
                  : `Archive ${session.name}`
              }
              onClick={() => onArchiveSession(session.id, !isArchivedList)}
            >
              {isArchivedList ? (
                <UnarchiveIcon size={12} />
              ) : (
                <ArchiveIcon size={12} />
              )}
            </button>
            <button
              type="button"
              className="verity-session-action-btn verity-session-action-btn--danger"
              title="Delete chat"
              aria-label={`Delete ${session.name}`}
              onClick={() => {
                setEditingId(null);
                setConfirmDeleteId(session.id);
              }}
            >
              <TrashIcon size={12} />
            </button>
          </div>
        )}
      </div>
    );
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

      <nav className="verity-rail-nav" aria-label="Session actions">
        <button
          type="button"
          className="verity-rail-btn"
          title="New session"
          aria-label="New session"
          onClick={() => onCreateSession()}
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
          title={expanded ? "Hide projects & chats" : "Projects & chats"}
          aria-label={expanded ? "Hide projects and chats" : "Show projects and chats"}
          aria-pressed={expanded}
          onClick={() => setExpanded((prev) => !prev)}
        >
          <MessageIcon size={17} />
          {expanded && <span className="verity-rail-btn-label">Projects &amp; chats</span>}
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
        <section className="verity-projects-section" aria-label="Projects and chats">
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
                <span>Add</span>
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

          <div className="verity-project-groups">
            {projects.map((project) => {
              const isProjectActive = project.id === activeProjectId;
              const isCollapsed = Boolean(collapsedProjects[project.id]);
              const projectChats = sessions.filter(
                (s) => s.projectId === project.id && !s.archived
              );

              return (
                <div key={project.id} className="verity-project-group">
                  <div
                    role="button"
                    tabIndex={0}
                    title={project.path}
                    aria-current={isProjectActive ? "true" : undefined}
                    className={
                      isProjectActive
                        ? "verity-project-group-header active"
                        : "verity-project-group-header"
                    }
                    onClick={() => {
                      if (!isProjectActive) {
                        onSelectProject(project.path);
                      }
                      if (isCollapsed) {
                        toggleProjectCollapse(project.id);
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.target !== e.currentTarget) return;
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        if (!isProjectActive) {
                          onSelectProject(project.path);
                        }
                        if (isCollapsed) {
                          toggleProjectCollapse(project.id);
                        }
                      }
                    }}
                  >
                    <button
                      type="button"
                      className="verity-project-chevron-btn"
                      aria-expanded={!isCollapsed}
                      aria-label={
                        isCollapsed
                          ? `Expand ${project.name} chats`
                          : `Collapse ${project.name} chats`
                      }
                      onClick={(e) => {
                        e.stopPropagation();
                        toggleProjectCollapse(project.id);
                      }}
                    >
                      <ChevronRightIcon
                        size={13}
                        className={
                          isCollapsed
                            ? "verity-chevron"
                            : "verity-chevron verity-chevron--expanded"
                        }
                      />
                    </button>

                    <span className="verity-project-item-main">
                      <FolderIcon size={14} className="verity-project-item-icon" />
                      <span className="verity-project-item-name">{project.name}</span>
                      {isProjectActive && (
                        <span
                          className="verity-project-active-dot"
                          aria-hidden="true"
                        />
                      )}
                    </span>

                    <div
                      className="verity-project-group-actions"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <button
                        type="button"
                        className="verity-project-action-btn"
                        title={`New chat in ${project.name}`}
                        aria-label={`New chat in ${project.name}`}
                        onClick={() => {
                          if (isCollapsed) {
                            toggleProjectCollapse(project.id);
                          }
                          onCreateSession(project.path);
                        }}
                      >
                        <PlusIcon size={12} />
                      </button>
                      {projects.length > 1 && (
                        <button
                          type="button"
                          className="verity-project-action-btn verity-project-remove-btn"
                          title={`Remove ${project.name}`}
                          aria-label={`Remove ${project.name}`}
                          onClick={() => onRemoveProject(project.path)}
                        >
                          <CloseIcon size={12} />
                        </button>
                      )}
                    </div>
                  </div>

                  {!isCollapsed && (
                    <div className="verity-project-sessions" role="list">
                      {projectChats.length === 0 ? (
                        <div className="verity-session-empty">No chats yet</div>
                      ) : (
                        projectChats.map((session) => renderSessionRow(session, false))
                      )}
                    </div>
                  )}
                </div>
              );
            })}

            {archivedSessions.length > 0 && (
              <div className="verity-archived-section">
                <button
                  type="button"
                  className="verity-archived-toggle"
                  aria-expanded={archivedExpanded}
                  onClick={() => setArchivedExpanded((prev) => !prev)}
                >
                  <ChevronRightIcon
                    size={12}
                    className={
                      archivedExpanded
                        ? "verity-chevron verity-chevron--expanded"
                        : "verity-chevron"
                    }
                  />
                  <ArchiveIcon size={12} />
                  <span>Archived ({archivedSessions.length})</span>
                </button>

                {archivedExpanded && (
                  <div className="verity-archived-list" role="list">
                    {archivedSessions.map((session) => renderSessionRow(session, true))}
                  </div>
                )}
              </div>
            )}
          </div>
        </section>
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
