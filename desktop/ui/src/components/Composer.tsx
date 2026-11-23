import {
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from "react";
import type {
  ModelOption,
  PendingAttachment,
  SlashOrSkillCommand,
  ThinkingLevel,
} from "../api/tauri";

export interface ComposerProps {
  readonly draft: string;
  readonly attachments: readonly PendingAttachment[];
  readonly models: readonly ModelOption[];
  readonly selectedModel: ModelOption;
  readonly thinkingLevels: readonly ThinkingLevel[];
  readonly thinkingLevel: ThinkingLevel;
  readonly commands: readonly SlashOrSkillCommand[];
  readonly isStreaming: boolean;
  readonly pendingQueueCount: number;
  readonly onDraftChange: (value: string) => void;
  readonly onRecallUp: () => void;
  readonly onRecallDown: () => void;
  readonly onAddAttachment: (attachment: PendingAttachment) => void;
  readonly onRemoveAttachment: (filename: string) => void;
  readonly onSelectModel: (provider: string, modelId: string) => void;
  readonly onSelectThinkingLevel: (level: ThinkingLevel) => void;
  readonly onSend: () => void;
  readonly onSteer: () => void;
  readonly onFollowUp: () => void;
  readonly onClearQueue: () => void;
  readonly onAbort: () => void;
}

const MAX_IMAGE_BYTES = 10 * 1024 * 1024;

function isAllowedMime(mime: string): mime is PendingAttachment["mime_type"] {
  return (
    mime === "image/png" ||
    mime === "image/jpeg" ||
    mime === "image/webp" ||
    mime === "image/gif"
  );
}

function isThinkingLevel(value: string): value is ThinkingLevel {
  return (
    value === "off" ||
    value === "minimal" ||
    value === "low" ||
    value === "medium" ||
    value === "high" ||
    value === "xhigh"
  );
}

interface CompletionMatch {
  readonly triggerStart: number;
  readonly items: readonly SlashOrSkillCommand[];
}

function resolveCompletions(
  draft: string,
  commands: readonly SlashOrSkillCommand[]
): CompletionMatch | null {
  const tokenMatch = /(^|\s)([/$][^\s]*)$/.exec(draft);
  if (!tokenMatch) return null;
  const rawToken = tokenMatch[2];
  const trigger = rawToken[0];
  if (trigger !== "/" && trigger !== "$") return null;
  const query = rawToken.toLowerCase();
  const items = commands.filter(
    (cmd) =>
      cmd.trigger === trigger &&
      (cmd.name.toLowerCase().startsWith(query) ||
        cmd.description.toLowerCase().includes(query.slice(1)))
  );
  if (items.length === 0) return null;
  const triggerStart = draft.length - rawToken.length;
  return { triggerStart, items };
}

export function Composer({
  draft,
  attachments,
  models,
  selectedModel,
  thinkingLevels,
  thinkingLevel,
  commands,
  isStreaming,
  pendingQueueCount,
  onDraftChange,
  onRecallUp,
  onRecallDown,
  onAddAttachment,
  onRemoveAttachment,
  onSelectModel,
  onSelectThinkingLevel,
  onSend,
  onSteer,
  onFollowUp,
  onClearQueue,
  onAbort,
}: ComposerProps) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const [selectedCompletionIdx, setSelectedCompletionIdx] = useState(0);
  const [completionDismissed, setCompletionDismissed] = useState(false);
  const [attachError, setAttachError] = useState<string | null>(null);

  const completion = useMemo(() => {
    if (completionDismissed) return null;
    return resolveCompletions(draft, commands);
  }, [draft, commands, completionDismissed]);

  const activeCompletionIdx = completion
    ? Math.min(selectedCompletionIdx, completion.items.length - 1)
    : 0;

  function applyCompletion(cmd: SlashOrSkillCommand) {
    if (!completion) return;
    const next = draft.slice(0, completion.triggerStart) + cmd.insertText;
    onDraftChange(next);
    setSelectedCompletionIdx(0);
    textareaRef.current?.focus();
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (completion) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedCompletionIdx((prev) => (prev + 1) % completion.items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedCompletionIdx(
          (prev) => (prev - 1 + completion.items.length) % completion.items.length
        );
        return;
      }
      if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
        e.preventDefault();
        applyCompletion(completion.items[activeCompletionIdx]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setCompletionDismissed(true);
        return;
      }
    }

    const el = e.currentTarget;
    const atStart = el.selectionStart === 0 && el.selectionEnd === 0;
    const atEnd =
      el.selectionStart === draft.length && el.selectionEnd === draft.length;

    if (e.key === "ArrowUp" && atStart && !draft.includes("\n")) {
      e.preventDefault();
      onRecallUp();
      return;
    }
    if (e.key === "ArrowDown" && atEnd && !draft.includes("\n")) {
      e.preventDefault();
      onRecallDown();
      return;
    }

    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (isStreaming) {
        if (draft.trim()) onFollowUp();
      } else {
        onSend();
      }
    }
  }

  function handleFileChange(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;

    const rawMime = file.type;
    if (!isAllowedMime(rawMime)) {
      setAttachError("Only PNG, JPEG, WebP, and GIF images are supported.");
      return;
    }
    const mimeType = rawMime;
    if (file.size > MAX_IMAGE_BYTES) {
      setAttachError("Image exceeds the 10 MiB attachment limit.");
      return;
    }

    setAttachError(null);
    const reader = new FileReader();
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const comma = result.indexOf(",");
      const base64 = comma >= 0 ? result.slice(comma + 1) : result;
      onAddAttachment({
        filename: file.name,
        mime_type: mimeType,
        base64_data: base64,
        size_bytes: file.size,
      });
    };
    reader.readAsDataURL(file);
  }

  const canSubmit = draft.trim().length > 0 || attachments.length > 0;
  const selectedModelKey = `${selectedModel.provider}::${selectedModel.modelId}`;

  return (
    <div className="pi-composer-shell">
      <div className="pi-composer-container">
        {completion && (
          <div className="pi-completion-popover" role="listbox" aria-label="Command completions">
            {completion.items.map((item, idx) => (
              <button
                key={item.name}
                type="button"
                role="option"
                aria-selected={idx === activeCompletionIdx}
                className={`pi-completion-item ${idx === activeCompletionIdx ? "selected" : ""}`}
                onMouseDown={(e) => {
                  e.preventDefault();
                  applyCompletion(item);
                }}
              >
                <span className="pi-completion-name">{item.name}</span>
                <span className="pi-completion-desc">{item.description}</span>
              </button>
            ))}
          </div>
        )}

        {pendingQueueCount > 0 && (
          <div className="pi-composer-queue-bar">
            <span>
              {pendingQueueCount} queued {pendingQueueCount === 1 ? "message" : "messages"} waiting
            </span>
            <button
              type="button"
              className="md3-btn-text"
              style={{ padding: "2px 10px", fontSize: "12px" }}
              onClick={onClearQueue}
            >
              Clear Queue
            </button>
          </div>
        )}

        {attachments.length > 0 && (
          <div className="pi-composer-attachments">
            {attachments.map((att) => (
              <span key={att.filename} className="pi-attachment-chip">
                <img
                  src={`data:${att.mime_type};base64,${att.base64_data}`}
                  alt={att.filename}
                />
                <span>{att.filename}</span>
                <button
                  type="button"
                  className="md3-icon-btn md3-icon-btn-sm"
                  aria-label={`Remove ${att.filename}`}
                  onClick={() => onRemoveAttachment(att.filename)}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        )}

        {attachError && (
          <div className="md3-error-banner" style={{ marginBottom: 8 }}>
            {attachError}
          </div>
        )}

        <textarea
          ref={textareaRef}
          className="pi-composer-textarea"
          aria-label="Message Pi"
          placeholder={
            isStreaming
              ? "Type a steering note or follow-up while Pi is streaming..."
              : "Message Pi... (/ commands, $ skills, ↑/↓ recall)"
          }
          value={draft}
          rows={2}
          onChange={(e) => {
            setCompletionDismissed(false);
            setSelectedCompletionIdx(0);
            onDraftChange(e.target.value);
          }}
          onKeyDown={handleKeyDown}
        />

        <div className="pi-composer-footer">
          <div className="pi-composer-controls">
            <input
              ref={fileInputRef}
              type="file"
              accept="image/png,image/jpeg,image/webp,image/gif"
              style={{ display: "none" }}
              onChange={handleFileChange}
            />
            <button
              type="button"
              className="md3-icon-btn"
              title="Attach image"
              aria-label="Attach image"
              onClick={() => fileInputRef.current?.click()}
            >
              📎
            </button>

            <select
              className="pi-pill-select"
              aria-label="Select model"
              value={selectedModelKey}
              onChange={(e) => {
                const [provider, modelId] = e.target.value.split("::");
                if (provider && modelId) {
                  onSelectModel(provider, modelId);
                }
              }}
            >
              {models.map((m) => (
                <option key={`${m.provider}::${m.modelId}`} value={`${m.provider}::${m.modelId}`}>
                  {m.label}
                </option>
              ))}
            </select>

            <select
              className="pi-pill-select"
              aria-label="Thinking level"
              value={thinkingLevel}
              onChange={(e) => {
                if (isThinkingLevel(e.target.value)) {
                  onSelectThinkingLevel(e.target.value);
                }
              }}
            >
              {thinkingLevels.map((lvl) => (
                <option key={lvl} value={lvl}>
                  Thinking: {lvl}
                </option>
              ))}
            </select>
          </div>

          <div className="pi-composer-actions">
            {isStreaming ? (
              <>
                <button
                  type="button"
                  className="md3-btn-tonal"
                  disabled={!draft.trim()}
                  onClick={onSteer}
                >
                  Steer
                </button>
                <button
                  type="button"
                  className="md3-btn-outlined"
                  disabled={!draft.trim()}
                  onClick={onFollowUp}
                >
                  Follow-up
                </button>
                <button type="button" className="md3-btn-danger" onClick={onAbort}>
                  Stop
                </button>
              </>
            ) : (
              <button
                type="button"
                className="md3-btn-filled"
                disabled={!canSubmit}
                onClick={onSend}
              >
                Send
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
