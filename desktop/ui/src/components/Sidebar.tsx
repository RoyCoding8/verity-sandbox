import { useState, type KeyboardEvent } from "react";
import type { PiProject, PiSessionSummary } from "../api/tauri";
import type { ThemeMode } from "../theme/md3";

export interface SidebarProps {
  readonly projects: readonly PiProject[];
  readonly activeProjectId: string;
  readonly sessions: readonly PiSessionSummary[];
  readonly activeSessionId: string;
  readonly theme: ThemeMode;
  readonly isDesktop: boolean;
  readonly onSelectProject: (projectId: string) => void;
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
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

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

  function handleRenameKeyDown(e: KeyboardEvent<HTMLInputElement>, sessionId: string) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitRename(sessionId);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditingId(null);
    }
  }

  return (
    <aside className="pi-sidebar" aria-label="Pi sessions and projects">
      <div className="pi-sidebar-header">
        <div className="pi-brand">
          <div className="pi-brand-mark" aria-hidden="true">
            π
          </div>
          <div>
            <div className="pi-brand-title">Pi Workspace</div>
            <div className="pi-brand-sub">
              {isDesktop ? "Tauri RPC" : "Browser Preview"}
            </div>
          </div>
        </div>
        <button
          type="button"
          className="md3-icon-btn"
          title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          aria-label="Toggle color theme"
          onClick={onToggleTheme}
        >
          {theme === "dark" ? "☀" : "☾"}
        </button>
      </div>

      <div>
        <div className="pi-sidebar-section-label">
          <span>Project</span>
        </div>
        <select
          className="pi-project-select"
          aria-label="Active project"
          value={activeProjectId}
          onChange={(e) => onSelectProject(e.target.value)}
        >
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
      </div>

      <button
        type="button"
        className="md3-btn-filled"
        onClick={onCreateSession}
      >
        + New session
      </button>

      <div className="pi-sidebar-section-label">
        <span>Sessions</span>
        <span>{sessions.length}</span>
      </div>

      <div className="pi-session-list" role="list">
        {sessions.map((session) => {
          const isActive = session.id === activeSessionId;
          const isEditing = editingId === session.id;
          return (
            <div
              key={session.id}
              role="listitem"
              className={`pi-session-item ${isActive ? "active" : ""}`}
              onClick={() => {
                if (!isEditing) onSelectSession(session);
              }}
              onDoubleClick={() => beginRename(session)}
            >
              <div className="pi-session-main">
                {isEditing ? (
                  <input
                    className="pi-inline-rename"
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
                    <div className="pi-session-title">{session.name}</div>
                    <div className="pi-session-meta">
                      <span>{session.messageCount} msgs</span>
                      <span>·</span>
                      <span>{session.updatedAt}</span>
                    </div>
                  </>
                )}
              </div>
              {!isEditing && (
                <div className="pi-session-actions">
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
                    ✎
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>

      <div className="pi-sidebar-toolbar" aria-label="Session actions">
        <button
          type="button"
          className="md3-btn-tonal"
          onClick={onOpenHistoryTree}
        >
          Tree
        </button>
        <button
          type="button"
          className="md3-btn-tonal"
          onClick={onOpenFork}
        >
          Fork
        </button>
        <button
          type="button"
          className="md3-btn-outlined"
          onClick={onCloneSession}
        >
          Clone
        </button>
        <button
          type="button"
          className="md3-btn-outlined"
          onClick={onExportHtml}
        >
          Export HTML
        </button>
      </div>

      <div className="pi-sidebar-footer">
        <button
          type="button"
          className="md3-btn-text"
          style={{ width: "100%", justifyContent: "flex-start" }}
          onClick={onOpenProviders}
        >
          ⚙ Provider settings
        </button>
      </div>
    </aside>
  );
}
