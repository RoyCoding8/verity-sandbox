import { useMemo, useState, type KeyboardEvent } from "react";
import "./Sidebar.css";
import type { PiProject, PiSessionSummary } from "../api/tauri";
import type { ThemeMode } from "../theme/md3";
import {
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
  VerityLogoMark,
} from "./Icons";
import { Md3Select, type Md3SelectOption } from "./Md3Select";

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
  const [expanded, setExpanded] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  const projectOptions = useMemo<readonly Md3SelectOption[]>(
    () =>
      projects.map((project) => ({
        value: project.id,
        label: project.name,
      })),
    [projects]
  );

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
        <div className="verity-project-block">
          <Md3Select
            variant="tonal"
            placement="bottom"
            align="start"
            ariaLabel="Active project"
            leadingIcon={<FolderIcon size={14} />}
            value={activeProjectId}
            options={projectOptions}
            onChange={onSelectProject}
          />
        </div>
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
                    className={
                      isActive
                        ? "verity-session-item active"
                        : "verity-session-item"
                    }
                    onClick={() => {
                      if (!isEditing) onSelectSession(session);
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
