import { useEffect, useMemo, useState } from "react";
import {
  CheckIcon,
  ChevronRightIcon,
  CloseIcon,
  CpuIcon,
  RefreshIcon,
  SearchIcon,
  SettingsIcon,
  SparklesIcon,
  TerminalIcon,
} from "./Icons";
import "./ModalsOverlay.css";
import "./SessionSettingsModal.css";

export type SessionSettingsTab = "behavior" | "stats" | "commands";

export interface SessionStatsView {
  sessionId?: string;
  userMessages?: number;
  assistantMessages?: number;
  toolCalls?: number;
  totalMessages?: number;
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
  totalTokens?: number;
  cost?: number;
  contextWindow?: number;
  contextUsagePercent?: number;
  raw?: Record<string, unknown>;
}

export interface SlashCommandDirectoryItem {
  name: string;
  displayName: string;
  description?: string;
  source: "builtin" | "skill" | "prompt" | "extension" | string;
  location?: string;
  usage?: string;
}

export interface SessionSettingsModalProps {
  open: boolean;
  initialTab?: SessionSettingsTab;
  onClose: () => void;
  autoCompactionEnabled: boolean;
  autoRetryEnabled: boolean;
  steeringMode: "all" | "one-at-a-time";
  followUpMode: "all" | "one-at-a-time";
  onToggleAutoCompaction: (enabled: boolean) => void;
  onToggleAutoRetry: (enabled: boolean) => void;
  onChangeSteeringMode: (mode: "all" | "one-at-a-time") => void;
  onChangeFollowUpMode: (mode: "all" | "one-at-a-time") => void;
  onCycleModel: () => void;
  onCycleThinkingLevel: () => void;
  stats: SessionStatsView | null;
  statsLoading?: boolean;
  onRefreshStats: () => void;
  commands: SlashCommandDirectoryItem[];
  onInsertCommand: (commandText: string) => void;
}

type CommandSourceFilter = "all" | "skill" | "prompt" | "extension" | "builtin";

const SOURCE_FILTERS: readonly {
  readonly id: CommandSourceFilter;
  readonly label: string;
}[] = [
  { id: "all", label: "All" },
  { id: "skill", label: "Skills" },
  { id: "prompt", label: "Prompts" },
  { id: "extension", label: "Extensions" },
  { id: "builtin", label: "Built-in" },
];

const MODE_OPTIONS: readonly {
  readonly value: "all" | "one-at-a-time";
  readonly label: string;
}[] = [
  { value: "all", label: "All" },
  { value: "one-at-a-time", label: "One at a time" },
];

const NUMBER_FORMAT = new Intl.NumberFormat();

function formatCount(value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return "—";
  return NUMBER_FORMAT.format(Math.round(value));
}

function formatCost(cost: number | undefined): string {
  if (cost === undefined || !Number.isFinite(cost)) return "—";
  if (cost === 0) return "$0.00";
  if (cost < 0.01) return `$${cost.toFixed(4)}`;
  return `$${cost.toFixed(2)}`;
}

function normalizeSource(source: string): string {
  const lowered = source.trim().toLowerCase();
  if (lowered === "built-in") return "builtin";
  return lowered;
}

function getSourceMeta(source: string): {
  label: string;
  badgeClass: string;
} {
  switch (normalizeSource(source)) {
    case "skill":
      return { label: "Skill", badgeClass: "ssm-source-badge--skill" };
    case "prompt":
      return { label: "Prompt", badgeClass: "ssm-source-badge--prompt" };
    case "extension":
      return { label: "Extension", badgeClass: "ssm-source-badge--extension" };
    case "builtin":
      return { label: "Built-in", badgeClass: "ssm-source-badge--builtin" };
    default: {
      const cleaned = source.trim();
      const label =
        cleaned.length > 0
          ? cleaned.charAt(0).toUpperCase() + cleaned.slice(1)
          : "Command";
      return { label, badgeClass: "ssm-source-badge--builtin" };
    }
  }
}

function computeTotalTokens(stats: SessionStatsView): number | undefined {
  if (stats.totalTokens !== undefined && Number.isFinite(stats.totalTokens)) {
    return stats.totalTokens;
  }
  const parts = [
    stats.inputTokens,
    stats.outputTokens,
    stats.cacheReadTokens,
    stats.cacheWriteTokens,
  ].filter((v): v is number => v !== undefined && Number.isFinite(v));
  if (parts.length === 0) return undefined;
  return parts.reduce((sum, n) => sum + n, 0);
}

function computeContextPercent(
  stats: SessionStatsView,
  totalTokens: number | undefined
): number | undefined {
  if (
    stats.contextUsagePercent !== undefined &&
    Number.isFinite(stats.contextUsagePercent)
  ) {
    return Math.min(100, Math.max(0, stats.contextUsagePercent));
  }
  if (
    totalTokens !== undefined &&
    stats.contextWindow !== undefined &&
    stats.contextWindow > 0
  ) {
    return Math.min(100, Math.max(0, (totalTokens / stats.contextWindow) * 100));
  }
  return undefined;
}

export function SessionSettingsModal({
  open,
  initialTab = "behavior",
  onClose,
  autoCompactionEnabled,
  autoRetryEnabled,
  steeringMode,
  followUpMode,
  onToggleAutoCompaction,
  onToggleAutoRetry,
  onChangeSteeringMode,
  onChangeFollowUpMode,
  onCycleModel,
  onCycleThinkingLevel,
  stats,
  statsLoading = false,
  onRefreshStats,
  commands,
  onInsertCommand,
}: SessionSettingsModalProps) {
  const [activeTab, setActiveTab] = useState<SessionSettingsTab>(initialTab);
  const [searchQuery, setSearchQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<CommandSourceFilter>("all");

  useEffect(() => {
    if (!open) return;
    setActiveTab(initialTab);
    setSearchQuery("");
    setSourceFilter("all");
  }, [open, initialTab]);

  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const sourceCounts = useMemo(() => {
    const counts: Record<CommandSourceFilter, number> = {
      all: commands.length,
      skill: 0,
      prompt: 0,
      extension: 0,
      builtin: 0,
    };
    for (const cmd of commands) {
      const normalized = normalizeSource(cmd.source);
      if (
        normalized === "skill" ||
        normalized === "prompt" ||
        normalized === "extension" ||
        normalized === "builtin"
      ) {
        counts[normalized] += 1;
      }
    }
    return counts;
  }, [commands]);

  const filteredCommands = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return commands.filter((cmd) => {
      if (sourceFilter !== "all" && normalizeSource(cmd.source) !== sourceFilter) {
        return false;
      }
      if (!query) return true;
      return (
        cmd.displayName.toLowerCase().includes(query) ||
        cmd.name.toLowerCase().includes(query) ||
        (cmd.description?.toLowerCase().includes(query) ?? false) ||
        (cmd.location?.toLowerCase().includes(query) ?? false)
      );
    });
  }, [commands, searchQuery, sourceFilter]);

  if (!open) return null;

  const totalTokens = stats ? computeTotalTokens(stats) : undefined;
  const contextPercent = stats
    ? computeContextPercent(stats, totalTokens)
    : undefined;
  const totalMessages = stats
    ? stats.totalMessages ??
      (stats.userMessages !== undefined || stats.assistantMessages !== undefined
        ? (stats.userMessages ?? 0) + (stats.assistantMessages ?? 0)
        : undefined)
    : undefined;

  const progressToneClass =
    contextPercent !== undefined && contextPercent >= 90
      ? "ssm-progress-fill--danger"
      : contextPercent !== undefined && contextPercent >= 75
        ? "ssm-progress-fill--warning"
        : "";

  return (
    <div className="md3-scrim" onClick={onClose}>
      <div
        className="md3-dialog ssm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="session-settings-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="md3-dialog-header">
          <div className="vo-dialog-heading">
            <span className="vo-dialog-icon" aria-hidden="true">
              <SettingsIcon size={17} />
            </span>
            <div className="vo-dialog-header-text">
              <h2 id="session-settings-title" className="md3-dialog-title">
                Session Settings
              </h2>
              <p className="md3-dialog-subtitle">
                Configure agent behavior, inspect token usage, and browse slash
                commands.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close session settings"
            onClick={onClose}
          >
            <CloseIcon size={16} />
          </button>
        </header>

        <nav className="ssm-tabs" role="tablist" aria-label="Session settings sections">
          <button
            type="button"
            role="tab"
            id="ssm-tab-behavior"
            aria-selected={activeTab === "behavior"}
            aria-controls="ssm-panel-behavior"
            className={`ssm-tab ${activeTab === "behavior" ? "ssm-tab--active" : ""}`}
            onClick={() => setActiveTab("behavior")}
          >
            <SettingsIcon size={14} />
            Behavior
          </button>
          <button
            type="button"
            role="tab"
            id="ssm-tab-stats"
            aria-selected={activeTab === "stats"}
            aria-controls="ssm-panel-stats"
            className={`ssm-tab ${activeTab === "stats" ? "ssm-tab--active" : ""}`}
            onClick={() => setActiveTab("stats")}
          >
            <CpuIcon size={14} />
            Session Stats
          </button>
          <button
            type="button"
            role="tab"
            id="ssm-tab-commands"
            aria-selected={activeTab === "commands"}
            aria-controls="ssm-panel-commands"
            className={`ssm-tab ${activeTab === "commands" ? "ssm-tab--active" : ""}`}
            onClick={() => setActiveTab("commands")}
          >
            <TerminalIcon size={14} />
            Commands &amp; Skills
            <span className="ssm-tab-count">{commands.length}</span>
          </button>
        </nav>

        <div className="md3-dialog-body">
          {activeTab === "behavior" && (
            <div
              id="ssm-panel-behavior"
              role="tabpanel"
              aria-labelledby="ssm-tab-behavior"
              className="ssm-panel"
            >
              <section className="ssm-section">
                <h3 className="ssm-section-label">Automation</h3>
                <div className="ssm-setting-list">
                  <div className="ssm-setting-row">
                    <div className="ssm-setting-copy">
                      <span
                        id="ssm-auto-compaction-label"
                        className="ssm-setting-title"
                      >
                        Auto-Compaction
                      </span>
                      <span className="ssm-setting-desc">
                        Automatically compact older turns when approaching the
                        context window limit.
                      </span>
                    </div>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={autoCompactionEnabled}
                      aria-labelledby="ssm-auto-compaction-label"
                      className={`ssm-switch ${
                        autoCompactionEnabled ? "ssm-switch--checked" : ""
                      }`}
                      onClick={() =>
                        onToggleAutoCompaction(!autoCompactionEnabled)
                      }
                    >
                      <span className="ssm-switch-thumb" aria-hidden="true">
                        {autoCompactionEnabled && <CheckIcon size={12} strokeWidth={2.5} />}
                      </span>
                    </button>
                  </div>

                  <div className="ssm-setting-row">
                    <div className="ssm-setting-copy">
                      <span
                        id="ssm-auto-retry-label"
                        className="ssm-setting-title"
                      >
                        Auto-Retry
                      </span>
                      <span className="ssm-setting-desc">
                        Automatically retry transient provider or network errors
                        with backoff.
                      </span>
                    </div>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={autoRetryEnabled}
                      aria-labelledby="ssm-auto-retry-label"
                      className={`ssm-switch ${
                        autoRetryEnabled ? "ssm-switch--checked" : ""
                      }`}
                      onClick={() => onToggleAutoRetry(!autoRetryEnabled)}
                    >
                      <span className="ssm-switch-thumb" aria-hidden="true">
                        {autoRetryEnabled && <CheckIcon size={12} strokeWidth={2.5} />}
                      </span>
                    </button>
                  </div>
                </div>
              </section>

              <section className="ssm-section">
                <h3 className="ssm-section-label">Queue Dispatch</h3>
                <div className="ssm-setting-list">
                  <div className="ssm-setting-row">
                    <div className="ssm-setting-copy">
                      <span
                        id="ssm-steering-mode-label"
                        className="ssm-setting-title"
                      >
                        Steering Mode
                      </span>
                      <span className="ssm-setting-desc">
                        Deliver queued steering messages together or one at a
                        time during an active turn.
                      </span>
                    </div>
                    <div
                      className="ssm-segmented"
                      role="radiogroup"
                      aria-labelledby="ssm-steering-mode-label"
                    >
                      {MODE_OPTIONS.map((option) => {
                        const active = steeringMode === option.value;
                        return (
                          <button
                            key={option.value}
                            type="button"
                            role="radio"
                            aria-checked={active}
                            className={`ssm-segmented-btn ${
                              active ? "ssm-segmented-btn--active" : ""
                            }`}
                            onClick={() => onChangeSteeringMode(option.value)}
                          >
                            {active && <CheckIcon size={12} strokeWidth={2.4} />}
                            {option.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>

                  <div className="ssm-setting-row">
                    <div className="ssm-setting-copy">
                      <span
                        id="ssm-followup-mode-label"
                        className="ssm-setting-title"
                      >
                        Follow-Up Mode
                      </span>
                      <span className="ssm-setting-desc">
                        Dispatch queued follow-up prompts all at once or one at
                        a time after a turn finishes.
                      </span>
                    </div>
                    <div
                      className="ssm-segmented"
                      role="radiogroup"
                      aria-labelledby="ssm-followup-mode-label"
                    >
                      {MODE_OPTIONS.map((option) => {
                        const active = followUpMode === option.value;
                        return (
                          <button
                            key={option.value}
                            type="button"
                            role="radio"
                            aria-checked={active}
                            className={`ssm-segmented-btn ${
                              active ? "ssm-segmented-btn--active" : ""
                            }`}
                            onClick={() => onChangeFollowUpMode(option.value)}
                          >
                            {active && <CheckIcon size={12} strokeWidth={2.4} />}
                            {option.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              </section>

              <section className="ssm-section">
                <h3 className="ssm-section-label">Quick Actions</h3>
                <div className="ssm-quick-actions">
                  <button
                    type="button"
                    className="ssm-action-card"
                    onClick={onCycleModel}
                  >
                    <span className="ssm-action-card-main">
                      <span className="ssm-action-icon" aria-hidden="true">
                        <CpuIcon size={16} />
                      </span>
                      <span className="ssm-action-text">
                        <span className="ssm-action-title">Cycle Model</span>
                        <span className="ssm-action-sub">
                          Switch to the next configured model
                        </span>
                      </span>
                    </span>
                    <ChevronRightIcon size={15} className="ssm-action-chevron" />
                  </button>

                  <button
                    type="button"
                    className="ssm-action-card"
                    onClick={onCycleThinkingLevel}
                  >
                    <span className="ssm-action-card-main">
                      <span className="ssm-action-icon" aria-hidden="true">
                        <SparklesIcon size={16} />
                      </span>
                      <span className="ssm-action-text">
                        <span className="ssm-action-title">
                          Cycle Thinking Level
                        </span>
                        <span className="ssm-action-sub">
                          Step through reasoning effort levels
                        </span>
                      </span>
                    </span>
                    <ChevronRightIcon size={15} className="ssm-action-chevron" />
                  </button>
                </div>
              </section>
            </div>
          )}

          {activeTab === "stats" && (
            <div
              id="ssm-panel-stats"
              role="tabpanel"
              aria-labelledby="ssm-tab-stats"
              className="ssm-panel"
            >
              <div className="ssm-stats-toolbar">
                <div className="ssm-session-id-wrap">
                  <span>Session</span>
                  <code className="vo-entry-id">
                    {stats?.sessionId ?? "No active session"}
                  </code>
                </div>
                <button
                  type="button"
                  className="md3-btn-tonal"
                  disabled={statsLoading}
                  onClick={onRefreshStats}
                >
                  <RefreshIcon
                    size={14}
                    className={statsLoading ? "ssm-spin" : undefined}
                  />
                  {statsLoading ? "Refreshing..." : "Refresh"}
                </button>
              </div>

              {stats === null ? (
                <div className="vo-empty-state">
                  <span className="vo-empty-state-icon" aria-hidden="true">
                    <CpuIcon size={18} />
                  </span>
                  <p className="vo-empty-state-title">
                    {statsLoading
                      ? "Loading session statistics..."
                      : "No session statistics available"}
                  </p>
                  <p className="vo-empty-state-desc">
                    Send a message or click Refresh to load token usage, context
                    utilization, and estimated cost for this session.
                  </p>
                </div>
              ) : (
                <>
                  <div className="ssm-context-card">
                    <div className="ssm-context-header">
                      <span className="ssm-context-title">Context Usage</span>
                      <span className="ssm-context-values">
                        <span className="ssm-context-percent">
                          {contextPercent !== undefined
                            ? `${contextPercent.toFixed(1)}%`
                            : "—"}
                        </span>
                        <span className="ssm-context-tokens">
                          {formatCount(totalTokens)}
                          {stats.contextWindow !== undefined
                            ? ` / ${formatCount(stats.contextWindow)} tokens`
                            : " tokens"}
                        </span>
                      </span>
                    </div>
                    <div
                      className="ssm-progress-track"
                      role="progressbar"
                      aria-label="Context window usage"
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-valuenow={
                        contextPercent !== undefined
                          ? Math.round(contextPercent)
                          : 0
                      }
                    >
                      <div
                        className={`ssm-progress-fill ${progressToneClass}`}
                        style={{
                          width: `${contextPercent ?? 0}%`,
                        }}
                      />
                    </div>
                  </div>

                  <div className="ssm-metrics-grid">
                    <div className="ssm-metric-card">
                      <span className="ssm-metric-label">Total Tokens</span>
                      <span className="ssm-metric-value">
                        {formatCount(totalTokens)}
                      </span>
                      <span className="ssm-metric-sub">
                        {stats.contextWindow !== undefined
                          ? `Window: ${formatCount(stats.contextWindow)}`
                          : "Across current session"}
                      </span>
                    </div>

                    <div className="ssm-metric-card">
                      <span className="ssm-metric-label">Estimated Cost</span>
                      <span className="ssm-metric-value">
                        {formatCost(stats.cost)}
                      </span>
                      <span className="ssm-metric-sub">
                        Session cumulative estimate
                      </span>
                    </div>

                    <div className="ssm-metric-card">
                      <span className="ssm-metric-label">Messages</span>
                      <span className="ssm-metric-value">
                        {formatCount(totalMessages)}
                      </span>
                      <span className="ssm-metric-sub">
                        {formatCount(stats.userMessages)} user ·{" "}
                        {formatCount(stats.assistantMessages)} assistant
                      </span>
                    </div>

                    <div className="ssm-metric-card">
                      <span className="ssm-metric-label">Tool Calls</span>
                      <span className="ssm-metric-value">
                        {formatCount(stats.toolCalls)}
                      </span>
                      <span className="ssm-metric-sub">
                        Executed in this session
                      </span>
                    </div>
                  </div>

                  <div className="ssm-breakdown-card">
                    <span className="ssm-section-label">Token Breakdown</span>
                    <div className="ssm-breakdown-grid">
                      <div className="ssm-breakdown-cell">
                        <span className="ssm-breakdown-label">Input</span>
                        <span className="ssm-breakdown-val">
                          {formatCount(stats.inputTokens)}
                        </span>
                      </div>
                      <div className="ssm-breakdown-cell">
                        <span className="ssm-breakdown-label">Output</span>
                        <span className="ssm-breakdown-val">
                          {formatCount(stats.outputTokens)}
                        </span>
                      </div>
                      <div className="ssm-breakdown-cell">
                        <span className="ssm-breakdown-label">Cache Read</span>
                        <span className="ssm-breakdown-val">
                          {formatCount(stats.cacheReadTokens)}
                        </span>
                      </div>
                      <div className="ssm-breakdown-cell">
                        <span className="ssm-breakdown-label">Cache Write</span>
                        <span className="ssm-breakdown-val">
                          {formatCount(stats.cacheWriteTokens)}
                        </span>
                      </div>
                    </div>
                  </div>
                </>
              )}
            </div>
          )}

          {activeTab === "commands" && (
            <div
              id="ssm-panel-commands"
              role="tabpanel"
              aria-labelledby="ssm-tab-commands"
              className="ssm-panel"
            >
              <div className="vo-search-row">
                <div className="vo-search-wrap">
                  <span className="vo-search-icon" aria-hidden="true">
                    <SearchIcon size={15} />
                  </span>
                  <input
                    type="search"
                    className="md3-input vo-search-input"
                    aria-label="Search commands and skills"
                    placeholder="Search slash commands, skills, or paths..."
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                  />
                </div>
                <span className="vo-search-count">
                  {filteredCommands.length} / {commands.length}
                </span>
              </div>

              <div
                className="ssm-filter-pills"
                role="group"
                aria-label="Filter commands by source"
              >
                {SOURCE_FILTERS.map((filter) => {
                  const active = sourceFilter === filter.id;
                  return (
                    <button
                      key={filter.id}
                      type="button"
                      aria-pressed={active}
                      className={`ssm-filter-pill ${
                        active ? "ssm-filter-pill--active" : ""
                      }`}
                      onClick={() => setSourceFilter(filter.id)}
                    >
                      <span>{filter.label}</span>
                      <span className="ssm-filter-pill-count">
                        {sourceCounts[filter.id]}
                      </span>
                    </button>
                  );
                })}
              </div>

              {filteredCommands.length === 0 ? (
                <div className="vo-empty-state">
                  <span className="vo-empty-state-icon" aria-hidden="true">
                    <TerminalIcon size={18} />
                  </span>
                  <p className="vo-empty-state-title">No matching commands</p>
                  <p className="vo-empty-state-desc">
                    {commands.length === 0
                      ? "No slash commands or skills are registered for this session."
                      : "Try clearing your search query or selecting a different source filter."}
                  </p>
                </div>
              ) : (
                <div className="ssm-command-list">
                  {filteredCommands.map((cmd) => {
                    const sourceMeta = getSourceMeta(cmd.source);
                    return (
                      <button
                        key={`${cmd.source}:${cmd.name}:${cmd.displayName}`}
                        type="button"
                        className="ssm-command-item"
                        onClick={() => {
                          onInsertCommand(`${cmd.displayName} `);
                          onClose();
                        }}
                      >
                        <div className="ssm-command-main">
                          <div className="ssm-command-top">
                            <span className="ssm-command-name">
                              {cmd.displayName}
                            </span>
                            <span
                              className={`ssm-source-badge ${sourceMeta.badgeClass}`}
                            >
                              {sourceMeta.label}
                            </span>
                            {cmd.usage && (
                              <span className="ssm-command-usage">
                                {cmd.usage}
                              </span>
                            )}
                          </div>
                          {cmd.description && (
                            <span className="ssm-command-desc">
                              {cmd.description}
                            </span>
                          )}
                          {cmd.location && (
                            <span
                              className="ssm-command-location"
                              title={cmd.location}
                            >
                              {cmd.location}
                            </span>
                          )}
                        </div>
                        <span className="ssm-command-insert">
                          Insert
                          <ChevronRightIcon size={13} />
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>

        <footer className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
