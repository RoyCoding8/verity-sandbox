import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from "react";
import type {
  ModelOption,
  PendingAttachment,
  SlashOrSkillCommand,
  ThinkingLevel,
} from "../api/tauri";
import {
  CloseIcon,
  CpuIcon,
  PlusIcon,
  SendIcon,
  SparklesIcon,
  StopIcon,
} from "./Icons";
import { Md3Select, type Md3SelectOption } from "./Md3Select";
import "./Composer.css";

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
  readonly variant?: "hero" | "docked";
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
const MAX_TEXTAREA_HEIGHT_PX = 180;

function isAllowedMime(mime: string): mime is PendingAttachment["mime_type"] {
  return (
    mime === "image/png" ||
    mime === "image/jpeg" ||
    mime === "image/webp" ||
    mime === "image/gif"
  );
}

function modelOptionKey(model: Pick<ModelOption, "provider" | "modelId">): string {
  return `${model.provider}::${model.modelId}`;
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

function formatThinkingLevel(level: ThinkingLevel): string {
  switch (level) {
    case "off":
      return "Off";
    case "minimal":
      return "Minimal";
    case "low":
      return "Low";
    case "medium":
      return "Medium";
    case "high":
      return "High";
    case "xhigh":
      return "XHigh";
    default: {
      const _exhaustive: never = level;
      return _exhaustive;
    }
  }
}

interface CompletionMatch {
  readonly trigger: "/" | "$";
  readonly triggerStart: number;
  readonly triggerEnd: number;
  readonly items: readonly SlashOrSkillCommand[];
}

function resolveCompletions(
  draft: string,
  caretPos: number,
  commands: readonly SlashOrSkillCommand[]
): CompletionMatch | null {
  const clampedCaret = Math.max(0, Math.min(caretPos, draft.length));
  const beforeCaret = draft.slice(0, clampedCaret);
  const tokenMatch = /(^|\s)([/$][^\s]*)$/.exec(beforeCaret);
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
  if (
    items.length === 1 &&
    items[0].name.toLowerCase() === query &&
    items[0].insertText === items[0].name
  ) {
    return null;
  }
  const triggerStart = beforeCaret.length - rawToken.length;
  return { trigger, triggerStart, triggerEnd: clampedCaret, items };
}

function resolveAttachmentFilename(
  file: File,
  mimeType: PendingAttachment["mime_type"],
  index: number
): string {
  const trimmed = file.name.trim();
  if (trimmed.length > 0) {
    return trimmed;
  }
  const ext = mimeType.slice("image/".length);
  const suffix = index > 0 ? `-${index + 1}` : "";
  return `pasted-image-${Date.now()}${suffix}.${ext}`;
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
  variant = "docked",
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
  const containerRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const completionListRef = useRef<HTMLDivElement | null>(null);
  const activeReadersRef = useRef<Set<FileReader>>(new Set());
  const isMountedRef = useRef(true);
  const lastTextareaWidthRef = useRef<number>(-1);
  const pendingCaretRef = useRef<number | null>(null);

  const [caretPos, setCaretPos] = useState(draft.length);
  const [selectedCompletionIdx, setSelectedCompletionIdx] = useState(0);
  const [completionDismissed, setCompletionDismissed] = useState(false);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);

  const completion = useMemo(() => {
    if (completionDismissed) return null;
    return resolveCompletions(draft, caretPos, commands);
  }, [draft, caretPos, commands, completionDismissed]);

  const activeCompletionIdx = completion
    ? Math.min(selectedCompletionIdx, completion.items.length - 1)
    : 0;

  const canSubmit = draft.trim().length > 0 || attachments.length > 0;
  const selectedModelKey = modelOptionKey(selectedModel);

  const effectiveModels = useMemo<readonly ModelOption[]>(() => {
    const byKey = new Map<string, ModelOption>();
    for (const model of models) {
      const key = modelOptionKey(model);
      if (!byKey.has(key)) {
        byKey.set(key, model);
      }
    }
    const selectedKey = modelOptionKey(selectedModel);
    if (!byKey.has(selectedKey)) {
      byKey.set(selectedKey, selectedModel);
    }
    return Array.from(byKey.values());
  }, [models, selectedModel]);

  const modelOptions = useMemo<readonly Md3SelectOption[]>(
    () =>
      effectiveModels.map((m) => ({
        value: modelOptionKey(m),
        label: m.label,
        description: m.provider,
      })),
    [effectiveModels]
  );

  const effectiveThinkingLevels = useMemo<readonly ThinkingLevel[]>(() => {
    const seen = new Set<ThinkingLevel>();
    const list: ThinkingLevel[] = [];
    for (const lvl of thinkingLevels) {
      if (!seen.has(lvl)) {
        seen.add(lvl);
        list.push(lvl);
      }
    }
    if (!seen.has(thinkingLevel)) {
      list.push(thinkingLevel);
    }
    return list;
  }, [thinkingLevels, thinkingLevel]);

  const thinkingOptions = useMemo<readonly Md3SelectOption[]>(
    () =>
      effectiveThinkingLevels.map((lvl) => ({
        value: lvl,
        label: formatThinkingLevel(lvl),
      })),
    [effectiveThinkingLevels]
  );

  const syncTextareaHeight = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    const nextHeight = Math.min(el.scrollHeight, MAX_TEXTAREA_HEIGHT_PX);
    el.style.height = `${nextHeight}px`;
    el.style.overflowY = el.scrollHeight > MAX_TEXTAREA_HEIGHT_PX ? "auto" : "hidden";
  }, []);

  useLayoutEffect(() => {
    syncTextareaHeight();
    if (pendingCaretRef.current !== null && textareaRef.current) {
      const pos = Math.min(pendingCaretRef.current, draft.length);
      pendingCaretRef.current = null;
      textareaRef.current.setSelectionRange(pos, pos);
    }
  }, [draft, syncTextareaHeight]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el || typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      const width = entry.contentRect.width;
      if (width !== lastTextareaWidthRef.current) {
        lastTextareaWidthRef.current = width;
        syncTextareaHeight();
      }
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
    };
  }, [syncTextareaHeight]);

  useEffect(() => {
    isMountedRef.current = true;
    const readers = activeReadersRef.current;
    return () => {
      isMountedRef.current = false;
      for (const reader of readers) {
        if (reader.readyState === FileReader.LOADING) {
          reader.abort();
        }
      }
      readers.clear();
    };
  }, []);

  useEffect(() => {
    if (!completion || !completionListRef.current) return;
    const selectedEl = completionListRef.current.children[activeCompletionIdx];
    if (selectedEl instanceof HTMLElement) {
      selectedEl.scrollIntoView({ block: "nearest" });
    }
  }, [completion, activeCompletionIdx]);

  useEffect(() => {
    if (!completion) {
      return;
    }
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        containerRef.current &&
        target instanceof Node &&
        !containerRef.current.contains(target)
      ) {
        setCompletionDismissed(true);
      }
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
    };
  }, [completion]);

  useEffect(() => {
    if (!isStreaming) {
      return;
    }
    const handleWindowKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) {
        return;
      }
      if (event.isComposing || event.keyCode === 229) {
        return;
      }
      if (completion) {
        return;
      }
      if (
        document.querySelector(
          ".md3-scrim, .md3-select-menu, .pi-completion-popover"
        )
      ) {
        return;
      }
      event.preventDefault();
      onAbort();
    };
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => {
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, [completion, isStreaming, onAbort]);

  const processImageFiles = useCallback(
    (files: readonly File[]) => {
      if (files.length === 0) return;

      let firstError: string | null = null;
      const validFiles: Array<{
        readonly file: File;
        readonly mimeType: PendingAttachment["mime_type"];
        readonly filename: string;
      }> = [];

      files.forEach((file, index) => {
        const rawMime = file.type;
        if (!isAllowedMime(rawMime)) {
          firstError ??= "Only PNG, JPEG, WebP, and GIF images are supported.";
          return;
        }
        if (file.size === 0) {
          firstError ??= `Image "${file.name || "clipboard"}" is empty.`;
          return;
        }
        if (file.size > MAX_IMAGE_BYTES) {
          firstError ??= "Image exceeds the 10 MiB attachment limit.";
          return;
        }
        validFiles.push({
          file,
          mimeType: rawMime,
          filename: resolveAttachmentFilename(file, rawMime, index),
        });
      });

      setAttachError(firstError);

      for (const { file, mimeType, filename } of validFiles) {
        const reader = new FileReader();
        activeReadersRef.current.add(reader);

        reader.onload = () => {
          activeReadersRef.current.delete(reader);
          if (!isMountedRef.current) return;
          const result = typeof reader.result === "string" ? reader.result : "";
          const comma = result.indexOf(",");
          const base64 = (comma >= 0 ? result.slice(comma + 1) : result).trim();
          if (!base64) {
            setAttachError(`Failed to read image "${filename}".`);
            return;
          }
          onAddAttachment({
            filename,
            mime_type: mimeType,
            base64_data: base64,
            size_bytes: file.size,
          });
        };

        reader.onerror = () => {
          activeReadersRef.current.delete(reader);
          if (!isMountedRef.current) return;
          setAttachError(`Failed to read image "${filename}".`);
        };

        reader.onabort = () => {
          activeReadersRef.current.delete(reader);
        };

        reader.readAsDataURL(file);
      }
    },
    [onAddAttachment]
  );

  function applyCompletion(cmd: SlashOrSkillCommand) {
    if (!completion) return;
    const next =
      draft.slice(0, completion.triggerStart) +
      cmd.insertText +
      draft.slice(completion.triggerEnd);
    const nextCaret = completion.triggerStart + cmd.insertText.length;
    pendingCaretRef.current = nextCaret;
    setCaretPos(nextCaret);
    setCompletionDismissed(true);
    setSelectedCompletionIdx(0);
    onDraftChange(next);
    textareaRef.current?.focus();
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.nativeEvent.isComposing || e.keyCode === 229) {
      return;
    }

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
        if (!e.repeat) {
          applyCompletion(completion.items[activeCompletionIdx]);
        }
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setCompletionDismissed(true);
        return;
      }
    } else if (e.key === "Escape" && isStreaming) {
      e.preventDefault();
      e.stopPropagation();
      onAbort();
      return;
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
      if (e.repeat) {
        return;
      }
      if (isStreaming) {
        if (canSubmit) {
          if (e.ctrlKey || e.metaKey) {
            onSteer();
          } else {
            onFollowUp();
          }
        }
      } else if (canSubmit) {
        onSend();
      }
    }
  }

  function handleFileChange(e: ChangeEvent<HTMLInputElement>) {
    const files = e.target.files ? Array.from(e.target.files) : [];
    e.target.value = "";
    processImageFiles(files);
  }

  function handlePaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    const items = e.clipboardData?.items;
    if (!items || items.length === 0) return;

    const imageFiles: File[] = [];
    for (let i = 0; i < items.length; i += 1) {
      const item = items[i];
      if (item && item.kind === "file" && item.type.startsWith("image/")) {
        const file = item.getAsFile();
        if (file) {
          imageFiles.push(file);
        }
      }
    }

    if (imageFiles.length > 0) {
      e.preventDefault();
      processImageFiles(imageFiles);
    }
  }

  function handleDragOver(e: DragEvent<HTMLDivElement>) {
    if (!Array.from(e.dataTransfer.types).includes("Files")) {
      return;
    }
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    if (!isDragOver) {
      setIsDragOver(true);
    }
  }

  function handleDragLeave(e: DragEvent<HTMLDivElement>) {
    const related = e.relatedTarget;
    if (related instanceof Node && e.currentTarget.contains(related)) {
      return;
    }
    setIsDragOver(false);
  }

  function handleDrop(e: DragEvent<HTMLDivElement>) {
    if (!Array.from(e.dataTransfer.types).includes("Files")) {
      return;
    }
    e.preventDefault();
    setIsDragOver(false);
    const files = Array.from(e.dataTransfer.files);
    if (files.length > 0) {
      processImageFiles(files);
    }
  }

  return (
    <div
      className={
        variant === "hero"
          ? "pi-composer-shell pi-composer-shell--hero"
          : "pi-composer-shell"
      }
    >
      <div
        ref={containerRef}
        className={`pi-composer-container${isDragOver ? " is-drag-over" : ""}`}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        {completion && (
          <div
            className="pi-completion-popover"
            role="listbox"
            aria-label="Command completions"
          >
            <div className="pi-completion-header">
              <span>{completion.trigger === "/" ? "Commands" : "Workspace skills"}</span>
              <span>Tab or Enter to insert · Esc to dismiss</span>
            </div>
            <div ref={completionListRef} className="pi-completion-list">
              {completion.items.map((item, idx) => (
                <button
                  key={item.name}
                  type="button"
                  role="option"
                  aria-selected={idx === activeCompletionIdx}
                  className={`pi-completion-item${idx === activeCompletionIdx ? " selected" : ""}`}
                  onMouseEnter={() => setSelectedCompletionIdx(idx)}
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
                <span className="pi-attachment-name">{att.filename}</span>
                <button
                  type="button"
                  className="pi-attachment-remove"
                  aria-label={`Remove ${att.filename}`}
                  onClick={() => onRemoveAttachment(att.filename)}
                >
                  <CloseIcon size={12} />
                </button>
              </span>
            ))}
          </div>
        )}

        {attachError && (
          <div className="md3-error-banner pi-composer-error" role="alert">
            <span className="pi-composer-error-text">{attachError}</span>
            <button
              type="button"
              className="pi-composer-error-dismiss"
              aria-label="Dismiss attachment error"
              title="Dismiss"
              onClick={() => setAttachError(null)}
            >
              <CloseIcon size={12} />
            </button>
          </div>
        )}

        <div className="pi-composer-row">
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept="image/png,image/jpeg,image/webp,image/gif"
            style={{ display: "none" }}
            onChange={handleFileChange}
          />
          <button
            type="button"
            className="pi-composer-attach-btn"
            title="Attach image"
            aria-label="Attach image"
            onClick={() => fileInputRef.current?.click()}
          >
            <PlusIcon size={18} />
          </button>

          <textarea
            ref={textareaRef}
            className="pi-composer-textarea"
            aria-label="Ask Verity"
            placeholder={
              isStreaming
                ? "Follow-up (Enter) or steer (Ctrl+Enter)..."
                : "Ask Verity or type / for commands"
            }
            value={draft}
            rows={1}
            onChange={(e) => {
              setCaretPos(e.target.selectionStart ?? e.target.value.length);
              setCompletionDismissed(false);
              setSelectedCompletionIdx(0);
              onDraftChange(e.target.value);
            }}
            onSelect={(e) => {
              setCaretPos(e.currentTarget.selectionStart ?? e.currentTarget.value.length);
            }}
            onClick={(e) => {
              setCaretPos(e.currentTarget.selectionStart ?? e.currentTarget.value.length);
            }}
            onKeyUp={(e) => {
              setCaretPos(e.currentTarget.selectionStart ?? e.currentTarget.value.length);
            }}
            onPaste={handlePaste}
            onKeyDown={handleKeyDown}
          />

          <div className="pi-composer-inline-actions">
            {!isStreaming && (
              <>
                <Md3Select
                  className="pi-composer-model-select"
                  variant="pill"
                  placement="top"
                  align="end"
                  ariaLabel="Select model"
                  placeholder={selectedModel.label}
                  leadingIcon={<CpuIcon size={14} />}
                  value={selectedModelKey}
                  options={modelOptions}
                  onChange={(nextValue) => {
                    const matched = effectiveModels.find(
                      (m) => modelOptionKey(m) === nextValue
                    );
                    if (matched) {
                      onSelectModel(matched.provider, matched.modelId);
                      return;
                    }
                    const sepIdx = nextValue.indexOf("::");
                    if (sepIdx > 0 && sepIdx + 2 < nextValue.length) {
                      onSelectModel(
                        nextValue.slice(0, sepIdx),
                        nextValue.slice(sepIdx + 2)
                      );
                    }
                  }}
                />

                <Md3Select
                  className="pi-composer-thinking-select"
                  variant="pill"
                  placement="top"
                  align="end"
                  ariaLabel="Thinking level"
                  leadingIcon={<SparklesIcon size={14} />}
                  value={thinkingLevel}
                  options={thinkingOptions}
                  onChange={(nextValue) => {
                    if (isThinkingLevel(nextValue)) {
                      onSelectThinkingLevel(nextValue);
                    }
                  }}
                />
              </>
            )}

            {isStreaming ? (
              <>
                <button
                  type="button"
                  className="pi-composer-stream-pill"
                  disabled={!canSubmit}
                  title="Steer current turn (Ctrl+Enter)"
                  aria-label="Steer current turn (Ctrl+Enter)"
                  onClick={onSteer}
                >
                  <span>Steer</span>
                  <kbd className="pi-composer-kbd">Ctrl+↵</kbd>
                </button>
                <button
                  type="button"
                  className="pi-composer-stream-pill"
                  disabled={!canSubmit}
                  title="Queue follow-up (Enter)"
                  aria-label="Queue follow-up (Enter)"
                  onClick={onFollowUp}
                >
                  <span>Follow-up</span>
                  <kbd className="pi-composer-kbd">↵</kbd>
                </button>
                <button
                  type="button"
                  className="pi-composer-stop-circle"
                  title="Stop (Esc)"
                  aria-label="Stop generation (Esc)"
                  onClick={onAbort}
                >
                  <StopIcon size={14} />
                </button>
              </>
            ) : (
              <button
                type="button"
                className="pi-composer-send-circle"
                disabled={!canSubmit}
                title="Send message"
                aria-label="Send message"
                onClick={onSend}
              >
                <SendIcon size={15} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
