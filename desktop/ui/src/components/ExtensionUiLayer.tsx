import { useEffect, useState } from "react";
import type { ExtensionUiAnswer } from "../api/tauri";
import type {
  ExtensionSnackbar,
  ExtensionWidget,
  QueuedExtensionDialog,
} from "../state/piStore";

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
  const [textValue, setTextValue] = useState("");
  const [selectedOption, setSelectedOption] = useState("");
  const [remainingMs, setRemainingMs] = useState<number | null>(null);

  // Reset form state whenever the head of the FIFO dialog queue changes
  useEffect(() => {
    setTextValue("");
    setSelectedOption(activeDialog?.options[0] ?? "");
    setRemainingMs(activeDialog?.timeoutMs ?? null);
  }, [activeDialog?.id, activeDialog?.options, activeDialog?.timeoutMs]);

  // Countdown + auto-cancel when a dialog specifies a timeoutMs deadline
  useEffect(() => {
    if (!activeDialog || activeDialog.timeoutMs === null) return;
    const deadline = Date.now() + activeDialog.timeoutMs;
    const interval = window.setInterval(() => {
      const left = deadline - Date.now();
      if (left <= 0) {
        window.clearInterval(interval);
        onRespond(activeDialog.id, { kind: "cancelled" });
      } else {
        setRemainingMs(left);
      }
    }, 250);
    return () => window.clearInterval(interval);
  }, [activeDialog, onRespond]);

  // Auto-dismiss info snackbars after 4.5s
  useEffect(() => {
    if (snackbars.length === 0) return;
    const timers = snackbars.map((sb) =>
      window.setTimeout(() => onDismissSnackbar(sb.id), sb.tone === "error" ? 6500 : 4500)
    );
    return () => {
      for (const t of timers) window.clearTimeout(t);
    };
  }, [snackbars, onDismissSnackbar]);

  return (
    <>
      {widgets.length > 0 && (
        <div className="pi-extension-widgets" aria-label="Extension widgets">
          {widgets.map((widget) => (
            <div key={widget.id} className="pi-extension-widget">
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <div className="pi-extension-widget-title">{widget.title}</div>
                <button
                  type="button"
                  className="md3-icon-btn md3-icon-btn-sm"
                  aria-label={`Dismiss ${widget.title}`}
                  onClick={() => onDismissWidget(widget.id)}
                >
                  ×
                </button>
              </div>
              <div style={{ fontSize: 13 }}>{widget.body}</div>
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
            <div className="md3-dialog-header">
              <div>
                <h2 id="ext-dialog-title" className="md3-dialog-title">
                  {activeDialog.title}
                </h2>
                <p className="md3-dialog-subtitle">
                  Extension request · {activeDialog.kind}
                  {dialogs.length > 1 ? ` · 1 of ${dialogs.length} queued` : ""}
                  {remainingMs !== null ? ` · ${Math.ceil(remainingMs / 1000)}s left` : ""}
                </p>
              </div>
              <button
                type="button"
                className="md3-icon-btn"
                aria-label="Cancel extension dialog"
                onClick={() => onRespond(activeDialog.id, { kind: "cancelled" })}
              >
                ×
              </button>
            </div>

            <div className="md3-dialog-body">
              {activeDialog.kind === "confirm" && (
                <p style={{ margin: 0, color: "var(--md-sys-color-on-surface-variant)" }}>
                  Confirm whether Pi should proceed with this extension action.
                </p>
              )}

              {activeDialog.kind === "select" && (
                <div className="pi-option-list" role="radiogroup" aria-label={activeDialog.title}>
                  {activeDialog.options.map((option) => (
                    <button
                      key={option}
                      type="button"
                      className={`pi-option-btn ${selectedOption === option ? "selected" : ""}`}
                      onClick={() => setSelectedOption(option)}
                    >
                      <span>{option}</span>
                      {selectedOption === option && <span aria-hidden="true">✓</span>}
                    </button>
                  ))}
                </div>
              )}

              {activeDialog.kind === "input" && (
                <input
                  className="md3-input"
                  autoFocus
                  placeholder="Enter value..."
                  value={textValue}
                  onChange={(e) => setTextValue(e.target.value)}
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

            <div className="md3-dialog-footer">
              <button
                type="button"
                className="md3-btn-text"
                onClick={() => onRespond(activeDialog.id, { kind: "cancelled" })}
              >
                Cancel
              </button>

              {activeDialog.kind === "confirm" && (
                <>
                  <button
                    type="button"
                    className="md3-btn-outlined"
                    onClick={() =>
                      onRespond(activeDialog.id, { kind: "confirmed", data: false })
                    }
                  >
                    Decline
                  </button>
                  <button
                    type="button"
                    className="md3-btn-filled"
                    onClick={() =>
                      onRespond(activeDialog.id, { kind: "confirmed", data: true })
                    }
                  >
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
                    onRespond(activeDialog.id, { kind: "value", data: selectedOption })
                  }
                >
                  Select
                </button>
              )}

              {(activeDialog.kind === "input" || activeDialog.kind === "editor") && (
                <button
                  type="button"
                  className="md3-btn-filled"
                  onClick={() =>
                    onRespond(activeDialog.id, { kind: "value", data: textValue })
                  }
                >
                  Submit
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {snackbars.length > 0 && (
        <div className="pi-snackbar-stack" role="status" aria-live="polite">
          {snackbars.map((sb) => (
            <div key={sb.id} className={`pi-snackbar ${sb.tone}`}>
              <span>{sb.message}</span>
              <button
                type="button"
                className="md3-btn-text"
                style={{ color: "inherit", padding: "2px 8px" }}
                onClick={() => onDismissSnackbar(sb.id)}
              >
                Dismiss
              </button>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
