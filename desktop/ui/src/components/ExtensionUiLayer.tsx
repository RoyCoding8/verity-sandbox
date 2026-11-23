import { useEffect, useRef, useState } from "react";
import type { ExtensionUiAnswer } from "../api/tauri";
import type {
  ExtensionSnackbar,
  ExtensionWidget,
  QueuedExtensionDialog,
} from "../state/piStore";
import {
  CheckIcon,
  ClockIcon,
  CloseIcon,
  InfoIcon,
  XCircleIcon,
} from "./Icons";
import "./ModalsOverlay.css";

export interface ExtensionUiLayerProps {
  readonly dialogs: readonly QueuedExtensionDialog[];
  readonly snackbars: readonly ExtensionSnackbar[];
  readonly widgets: readonly ExtensionWidget[];
  readonly onRespond: (dialogId: string, answer: ExtensionUiAnswer) => void;
  readonly onDismissSnackbar: (id: string) => void;
  readonly onDismissWidget: (id: string) => void;
}

export function ExtensionUiLayer({
  dialogs,
  snackbars,
  widgets,
  onRespond,
  onDismissSnackbar,
  onDismissWidget,
}: ExtensionUiLayerProps) {
  const activeDialog = dialogs[0] ?? null;
  const activeDialogId = activeDialog?.id ?? null;
  const activeDialogDefaultOption = activeDialog?.options[0] ?? "";
  const activeDialogTimeoutMs = activeDialog?.timeoutMs ?? null;

  const [textValue, setTextValue] = useState("");
  const [selectedOption, setSelectedOption] = useState("");
  const [remainingMs, setRemainingMs] = useState<number | null>(null);

  const snackbarTimersRef = useRef<Map<string, number>>(new Map());
  const onDismissSnackbarRef = useRef(onDismissSnackbar);
  onDismissSnackbarRef.current = onDismissSnackbar;

  // Reset form state whenever the head of the FIFO dialog queue changes
  useEffect(() => {
    setTextValue("");
    setSelectedOption(activeDialogDefaultOption);
    setRemainingMs(activeDialogTimeoutMs);
  }, [activeDialogId, activeDialogDefaultOption, activeDialogTimeoutMs]);

  // Countdown + auto-cancel when a dialog specifies a timeoutMs deadline
  useEffect(() => {
    if (!activeDialogId || activeDialogTimeoutMs === null) return;
    const deadline = Date.now() + activeDialogTimeoutMs;
    const interval = window.setInterval(() => {
      const left = deadline - Date.now();
      if (left <= 0) {
        window.clearInterval(interval);
        onRespond(activeDialogId, { kind: "cancelled" });
      } else {
        setRemainingMs(left);
      }
    }, 250);
    return () => window.clearInterval(interval);
  }, [activeDialogId, activeDialogTimeoutMs, onRespond]);

  // Escape key cancels the active FIFO dialog
  useEffect(() => {
    if (!activeDialogId) return;
    const dialogId = activeDialogId;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onRespond(dialogId, { kind: "cancelled" });
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeDialogId, onRespond]);

  // Track per-snackbar timers so adding a new snackbar never resets existing timers
  useEffect(() => {
    const timers = snackbarTimersRef.current;
    const activeIds = new Set(snackbars.map((sb) => sb.id));

    for (const [id, timerId] of timers) {
      if (!activeIds.has(id)) {
        window.clearTimeout(timerId);
        timers.delete(id);
      }
    }

    for (const sb of snackbars) {
      if (!timers.has(sb.id)) {
        const delayMs = sb.tone === "error" ? 6500 : 4500;
        const timerId = window.setTimeout(() => {
          timers.delete(sb.id);
          onDismissSnackbarRef.current(sb.id);
        }, delayMs);
        timers.set(sb.id, timerId);
      }
    }
  }, [snackbars]);

  // Clean up all pending snackbar timers on unmount
  useEffect(() => {
    const timers = snackbarTimersRef.current;
    return () => {
      for (const timerId of timers.values()) {
        window.clearTimeout(timerId);
      }
      timers.clear();
    };
  }, []);

  const remainingSeconds =
    remainingMs !== null ? Math.max(0, Math.ceil(remainingMs / 1000)) : null;
  const countdownTone =
    remainingSeconds !== null && remainingSeconds <= 5
      ? "verity-badge--danger"
      : remainingSeconds !== null && remainingSeconds <= 10
        ? "verity-badge--warning"
        : "verity-badge--neutral";

  return (
    <>
      {widgets.length > 0 && (
        <div className="pi-extension-widgets" aria-label="Extension widgets">
          {widgets.map((widget) => (
            <div
              key={widget.id}
              className="pi-extension-widget vo-widget-card"
            >
              <div className="vo-widget-header">
                <div className="vo-widget-title-wrap">
                  <InfoIcon size={13} />
                  <div className="pi-extension-widget-title">
                    {widget.title}
                  </div>
                </div>
                <button
                  type="button"
                  className="md3-icon-btn md3-icon-btn-sm"
                  aria-label={`Dismiss ${widget.title}`}
                  onClick={() => onDismissWidget(widget.id)}
                >
                  <CloseIcon size={13} />
                </button>
              </div>
              <div className="vo-widget-body">{widget.body}</div>
            </div>
          ))}
        </div>
      )}

      {activeDialog && (
        <div
          className="md3-scrim"
          onClick={() => onRespond(activeDialog.id, { kind: "cancelled" })}
        >
          <div
            className="md3-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="ext-dialog-title"
            onClick={(e) => e.stopPropagation()}
          >
            <header className="md3-dialog-header">
              <div className="vo-dialog-heading">
                <span className="vo-dialog-icon" aria-hidden="true">
                  <InfoIcon size={17} />
                </span>
                <div className="vo-dialog-header-text">
                  <h2 id="ext-dialog-title" className="md3-dialog-title">
                    {activeDialog.title}
                  </h2>
                  <div className="vo-ext-meta">
                    <span className="verity-badge verity-badge--info">
                      {activeDialog.kind}
                    </span>
                    {dialogs.length > 1 && (
                      <span className="verity-badge verity-badge--neutral">
                        1 of {dialogs.length} queued
                      </span>
                    )}
                    {remainingSeconds !== null && (
                      <span className={`verity-badge ${countdownTone}`}>
                        <ClockIcon size={12} />
                        {remainingSeconds}s left
                      </span>
                    )}
                  </div>
                </div>
              </div>
              <button
                type="button"
                className="md3-icon-btn"
                aria-label="Cancel extension dialog"
                onClick={() =>
                  onRespond(activeDialog.id, { kind: "cancelled" })
                }
              >
                <CloseIcon size={16} />
              </button>
            </header>

            <div className="md3-dialog-body">
              {activeDialog.kind === "confirm" && (
                <p className="vo-ext-confirm-copy">
                  Confirm whether Pi should proceed with this extension action.
                </p>
              )}

              {activeDialog.kind === "select" && (
                <div
                  className="pi-option-list"
                  role="radiogroup"
                  aria-label={activeDialog.title}
                >
                  {activeDialog.options.map((option) => {
                    const isSelected = selectedOption === option;
                    return (
                      <button
                        key={option}
                        type="button"
                        role="radio"
                        aria-checked={isSelected}
                        className={`pi-option-btn vo-option-btn ${isSelected ? "selected" : ""}`}
                        onClick={() => setSelectedOption(option)}
                      >
                        <span className="vo-option-label">
                          <span className="vo-radio-dot" aria-hidden="true" />
                          <span>{option}</span>
                        </span>
                        {isSelected && <CheckIcon size={15} />}
                      </button>
                    );
                  })}
                </div>
              )}

              {activeDialog.kind === "input" && (
                <input
                  className="md3-input"
                  autoFocus
                  placeholder="Enter value..."
                  value={textValue}
                  onChange={(e) => setTextValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                      e.preventDefault();
                      onRespond(activeDialog.id, {
                        kind: "value",
                        data: textValue,
                      });
                    }
                  }}
                />
              )}

              {activeDialog.kind === "editor" && (
                <textarea
                  className="md3-textarea"
                  autoFocus
                  placeholder="Enter multiline text..."
                  value={textValue}
                  onChange={(e) => setTextValue(e.target.value)}
                />
              )}
            </div>

            <footer className="md3-dialog-footer">
              <button
                type="button"
                className="md3-btn-text"
                onClick={() =>
                  onRespond(activeDialog.id, { kind: "cancelled" })
                }
              >
                Cancel
              </button>

              {activeDialog.kind === "confirm" && (
                <>
                  <button
                    type="button"
                    className="md3-btn-outlined"
                    onClick={() =>
                      onRespond(activeDialog.id, {
                        kind: "confirmed",
                        data: false,
                      })
                    }
                  >
                    Decline
                  </button>
                  <button
                    type="button"
                    className="md3-btn-filled"
                    onClick={() =>
                      onRespond(activeDialog.id, {
                        kind: "confirmed",
                        data: true,
                      })
                    }
                  >
                    <CheckIcon size={14} />
                    Confirm
                  </button>
                </>
              )}

              {activeDialog.kind === "select" && (
                <button
                  type="button"
                  className="md3-btn-filled"
                  disabled={!selectedOption}
                  onClick={() =>
                    onRespond(activeDialog.id, {
                      kind: "value",
                      data: selectedOption,
                    })
                  }
                >
                  <CheckIcon size={14} />
                  Select
                </button>
              )}

              {(activeDialog.kind === "input" ||
                activeDialog.kind === "editor") && (
                <button
                  type="button"
                  className="md3-btn-filled"
                  onClick={() =>
                    onRespond(activeDialog.id, {
                      kind: "value",
                      data: textValue,
                    })
                  }
                >
                  <CheckIcon size={14} />
                  Submit
                </button>
              )}
            </footer>
          </div>
        </div>
      )}

      {snackbars.length > 0 && (
        <div className="pi-snackbar-stack" role="status" aria-live="polite">
          {snackbars.map((sb) => (
            <div key={sb.id} className={`pi-snackbar vo-snackbar ${sb.tone}`}>
              <span className="vo-snackbar-content">
                {sb.tone === "error" ? (
                  <XCircleIcon size={16} className="vo-snackbar-icon" />
                ) : (
                  <InfoIcon size={16} className="vo-snackbar-icon" />
                )}
                <span>{sb.message}</span>
              </span>
              <button
                type="button"
                className="vo-snackbar-dismiss"
                aria-label="Dismiss notification"
                onClick={() => onDismissSnackbar(sb.id)}
              >
                <CloseIcon size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
