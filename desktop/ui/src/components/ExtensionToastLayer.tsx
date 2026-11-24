import {
  AlertIcon,
  CloseIcon,
  InfoIcon,
  SparklesIcon,
  XCircleIcon,
} from "./Icons";
import "./ExtensionToastLayer.css";

export interface ExtensionToastItem {
  id: string;
  message: string;
  level: "info" | "warning" | "error";
  createdAt: number;
}

export interface ExtensionToastLayerProps {
  toasts: ExtensionToastItem[];
  onDismissToast: (id: string) => void;
  widgets: Record<string, string[]>;
}

function ToastLevelIcon({ level }: { readonly level: ExtensionToastItem["level"] }) {
  switch (level) {
    case "error":
      return <XCircleIcon size={16} className="etl-toast-icon" />;
    case "warning":
      return <AlertIcon size={16} className="etl-toast-icon" />;
    case "info":
      return <InfoIcon size={16} className="etl-toast-icon" />;
    default: {
      const _exhaustive: never = level;
      return _exhaustive;
    }
  }
}

export function ExtensionToastLayer({
  toasts,
  onDismissToast,
  widgets,
}: ExtensionToastLayerProps) {
  const widgetEntries = Object.entries(widgets);

  if (toasts.length === 0 && widgetEntries.length === 0) {
    return null;
  }

  return (
    <>
      {widgetEntries.length > 0 && (
        <aside
          className="etl-widget-stack"
          aria-label="Extension widgets"
        >
          {widgetEntries.map(([widgetKey, lines]) => (
            <section
              key={widgetKey}
              className="etl-widget-card"
              aria-label={widgetKey}
            >
              <div className="etl-widget-header">
                <SparklesIcon size={13} />
                <span className="etl-widget-key" title={widgetKey}>
                  {widgetKey}
                </span>
              </div>
              {lines.length > 0 && (
                <div className="etl-widget-lines">
                  {lines.map((line, index) => (
                    <p key={`${widgetKey}-${index}`} className="etl-widget-line">
                      {line}
                    </p>
                  ))}
                </div>
              )}
            </section>
          ))}
        </aside>
      )}

      {toasts.length > 0 && (
        <div
          className="etl-toast-stack"
          aria-live="polite"
          aria-relevant="additions removals"
        >
          {toasts.map((toast) => (
            <div
              key={toast.id}
              className={`etl-toast etl-toast--${toast.level}`}
              role={toast.level === "error" ? "alert" : "status"}
            >
              <div className="etl-toast-body">
                <ToastLevelIcon level={toast.level} />
                <span className="etl-toast-message">{toast.message}</span>
              </div>
              <button
                type="button"
                className="etl-toast-dismiss"
                aria-label="Dismiss notification"
                onClick={() => onDismissToast(toast.id)}
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
