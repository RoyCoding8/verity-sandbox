import { useEffect, useState, type FormEvent } from "react";
import {
  validateExternalSignInUrl,
  validateProviderBaseUrl,
  type ProviderApiKind,
  type ProviderProfileInput,
  type PublicProviderProfile,
} from "../api/tauri";
import {
  AlertIcon,
  CloseIcon,
  EditIcon,
  PlusIcon,
  ShieldCheckIcon,
  TrashIcon,
} from "./Icons";
import { Md3Select, type Md3SelectOption } from "./Md3Select";
import "./ModalsOverlay.css";

export interface ProviderSettingsModalProps {
  readonly open: boolean;
  readonly providers: readonly PublicProviderProfile[];
  readonly onSave: (input: ProviderProfileInput) => Promise<void>;
  readonly onDelete: (id: string) => Promise<void>;
  readonly onClose: () => void;
}

interface ProviderFormState {
  readonly id: string;
  readonly name: string;
  readonly api: ProviderApiKind;
  readonly baseUrl: string;
  readonly signInUrl: string;
  readonly modelsCsv: string;
  readonly apiKey: string;
  readonly clearApiKey: boolean;
  readonly isEditing: boolean;
}

const EMPTY_FORM: ProviderFormState = {
  id: "",
  name: "",
  api: "openai-completions",
  baseUrl: "",
  signInUrl: "",
  modelsCsv: "",
  apiKey: "",
  clearApiKey: false,
  isEditing: false,
};

const FORM_ID = "provider-settings-form";

const API_PROTOCOL_OPTIONS: readonly Md3SelectOption[] = [
  {
    value: "openai-completions",
    label: "openai-completions",
    description: "Chat Completions API",
  },
  {
    value: "openai-responses",
    label: "openai-responses",
    description: "OpenAI Responses API",
  },
  {
    value: "anthropic-messages",
    label: "anthropic-messages",
    description: "Anthropic Messages API",
  },
];

function isProviderApiKind(value: string): value is ProviderApiKind {
  return (
    value === "openai-completions" ||
    value === "openai-responses" ||
    value === "anthropic-messages"
  );
}

export function ProviderSettingsModal({
  open,
  providers,
  onSave,
  onDelete,
  onClose,
}: ProviderSettingsModalProps) {
  const [form, setForm] = useState<ProviderFormState>(EMPTY_FORM);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  // Reset form edits, error banner, and pending delete confirmation on close/reopen
  useEffect(() => {
    if (!open) {
      setForm(EMPTY_FORM);
      setError(null);
      setSubmitting(false);
      setPendingDeleteId(null);
      setDeletingId(null);
      return;
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  function loadForEdit(profile: PublicProviderProfile) {
    setError(null);
    setPendingDeleteId(null);
    setForm({
      id: profile.id,
      name: profile.name,
      api: profile.api,
      baseUrl: profile.base_url,
      signInUrl: profile.sign_in_url ?? "",
      modelsCsv: profile.models.join(", "),
      apiKey: "",
      clearApiKey: false,
      isEditing: true,
    });
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const id = form.id.trim();
    const name = form.name.trim();
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) {
      setError("Provider ID must be 1–64 ASCII letters, digits, '-' or '_'.");
      return;
    }
    if (!name) {
      setError("Provider display name is required.");
      return;
    }
    const baseErr = validateProviderBaseUrl(form.baseUrl);
    if (baseErr) {
      setError(baseErr);
      return;
    }
    const signInErr = validateExternalSignInUrl(form.signInUrl);
    if (signInErr) {
      setError(signInErr);
      return;
    }

    const models = form.modelsCsv
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);

    // Tri-state api_key: "" clears key, non-empty replaces key, undefined preserves existing key
    const apiKeyPayload = form.clearApiKey
      ? ""
      : form.apiKey.trim().length > 0
        ? form.apiKey.trim()
        : undefined;

    setSubmitting(true);
    setError(null);
    try {
      await onSave({
        id,
        name,
        api: form.api,
        base_url: form.baseUrl.trim(),
        sign_in_url: form.signInUrl.trim() || null,
        models,
        api_key: apiKeyPayload,
      });
      setForm(EMPTY_FORM);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  async function handleConfirmDelete(id: string) {
    setDeletingId(id);
    setError(null);
    try {
      await onDelete(id);
      if (form.id === id) setForm(EMPTY_FORM);
      setPendingDeleteId(null);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="md3-scrim" onClick={onClose}>
      <div
        className="md3-dialog md3-dialog-wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-settings-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="md3-dialog-header">
          <div className="vo-dialog-heading">
            <span className="vo-dialog-icon" aria-hidden="true">
              <ShieldCheckIcon size={17} />
            </span>
            <div className="vo-dialog-header-text">
              <h2 id="provider-settings-title" className="md3-dialog-title">
                Model Providers
              </h2>
              <p className="md3-dialog-subtitle">
                API keys are stored atomically with 0600 permissions and never
                echoed back to the UI.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close provider settings"
            onClick={onClose}
          >
            <CloseIcon size={16} />
          </button>
        </header>

        <div className="md3-dialog-body">
          {providers.length === 0 ? (
            <div className="vo-empty-state">
              <span className="vo-empty-state-icon" aria-hidden="true">
                <PlusIcon size={18} />
              </span>
              <p className="vo-empty-state-title">No providers configured</p>
              <p className="vo-empty-state-desc">
                Add a model provider below to configure its base URL, models,
                and API key.
              </p>
            </div>
          ) : (
            <div className="vo-provider-list">
              {providers.map((provider) => {
                const isConfirmingDelete = pendingDeleteId === provider.id;
                const isDeleting = deletingId === provider.id;
                const isBeingEdited = form.isEditing && form.id === provider.id;
                const cardClass = isConfirmingDelete
                  ? "vo-provider-card vo-provider-card--confirm-delete"
                  : isBeingEdited
                    ? "vo-provider-card vo-provider-card--editing"
                    : "vo-provider-card";

                return (
                  <div key={provider.id} className={cardClass}>
                    <div className="vo-provider-info">
                      <div className="vo-provider-header">
                        <strong className="vo-provider-name">
                          {provider.name}
                        </strong>
                        <code className="vo-entry-id">{provider.id}</code>
                        <span className="verity-badge verity-badge--info">
                          {provider.api}
                        </span>
                        <span
                          className={`verity-badge ${
                            provider.has_api_key
                              ? "verity-badge--success"
                              : "verity-badge--neutral"
                          }`}
                        >
                          {provider.has_api_key && (
                            <ShieldCheckIcon size={12} />
                          )}
                          {provider.has_api_key ? "Key saved" : "No key"}
                        </span>
                      </div>
                      <div
                        className="vo-provider-url"
                        title={provider.base_url}
                      >
                        {provider.base_url}
                      </div>
                      {provider.models.length > 0 && (
                        <div className="vo-provider-models">
                          Models: {provider.models.join(", ")}
                        </div>
                      )}
                    </div>

                    <div className="vo-provider-actions">
                      {isConfirmingDelete ? (
                        <div className="vo-confirm-delete-group">
                          <button
                            type="button"
                            className="md3-btn-danger"
                            disabled={isDeleting}
                            onClick={() => void handleConfirmDelete(provider.id)}
                          >
                            <TrashIcon size={14} />
                            {isDeleting ? "Deleting..." : "Confirm delete"}
                          </button>
                          <button
                            type="button"
                            className="md3-btn-text"
                            disabled={isDeleting}
                            onClick={() => setPendingDeleteId(null)}
                          >
                            Cancel
                          </button>
                        </div>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="md3-btn-tonal"
                            disabled={deletingId !== null}
                            onClick={() => loadForEdit(provider)}
                          >
                            <EditIcon size={14} />
                            Edit
                          </button>
                          <button
                            type="button"
                            className="md3-btn-danger"
                            disabled={deletingId !== null}
                            onClick={() => {
                              setError(null);
                              setPendingDeleteId(provider.id);
                            }}
                          >
                            <TrashIcon size={14} />
                            Delete
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          <form
            id={FORM_ID}
            className="vo-provider-form"
            onSubmit={(e) => void handleSubmit(e)}
          >
            <div className="vo-provider-form-header">
              <h3 className="vo-provider-form-title">
                {form.isEditing ? (
                  <EditIcon size={15} />
                ) : (
                  <PlusIcon size={15} />
                )}
                {form.isEditing ? `Edit "${form.id}"` : "Add provider"}
              </h3>
              {form.isEditing && (
                <button
                  type="button"
                  className="md3-btn-text"
                  onClick={() => {
                    setForm(EMPTY_FORM);
                    setError(null);
                  }}
                >
                  Reset form
                </button>
              )}
            </div>

            {error && (
              <div className="md3-error-banner vo-error-banner" role="alert">
                <AlertIcon size={15} className="vo-error-banner-icon" />
                <span>{error}</span>
              </div>
            )}

            <div className="md3-form-grid">
              <label className="md3-field">
                <span className="md3-field-label">Provider ID</span>
                <input
                  className="md3-input"
                  placeholder="e.g. openrouter"
                  value={form.id}
                  disabled={form.isEditing}
                  onChange={(e) => setForm({ ...form, id: e.target.value })}
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">Display Name</span>
                <input
                  className="md3-input"
                  placeholder="e.g. OpenRouter"
                  value={form.name}
                  onChange={(e) => setForm({ ...form, name: e.target.value })}
                />
              </label>

              <div className="md3-field">
                <span className="md3-field-label">API Protocol</span>
                <Md3Select
                  variant="field"
                  placement="bottom"
                  align="start"
                  ariaLabel="API Protocol"
                  value={form.api}
                  options={API_PROTOCOL_OPTIONS}
                  onChange={(nextValue) => {
                    if (isProviderApiKind(nextValue)) {
                      setForm({ ...form, api: nextValue });
                    }
                  }}
                />
              </div>

              <label className="md3-field">
                <span className="md3-field-label">Base URL</span>
                <input
                  className="md3-input"
                  placeholder="https://api.example.com/v1"
                  value={form.baseUrl}
                  onChange={(e) =>
                    setForm({ ...form, baseUrl: e.target.value })
                  }
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">
                  Models (comma-separated)
                </span>
                <input
                  className="md3-input"
                  placeholder="claude-sonnet-4-5, gpt-5"
                  value={form.modelsCsv}
                  onChange={(e) =>
                    setForm({ ...form, modelsCsv: e.target.value })
                  }
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">
                  External Sign-in URL (optional, https://)
                </span>
                <input
                  className="md3-input"
                  placeholder="https://console.example.com"
                  value={form.signInUrl}
                  onChange={(e) =>
                    setForm({ ...form, signInUrl: e.target.value })
                  }
                />
              </label>
            </div>

            <label className="md3-field">
              <span className="md3-field-label">
                API Key{" "}
                {form.isEditing
                  ? "(leave blank to keep existing key)"
                  : "(optional)"}
              </span>
              <input
                type="password"
                className="md3-input"
                autoComplete="off"
                placeholder="sk-..."
                value={form.apiKey}
                disabled={form.clearApiKey}
                onChange={(e) => setForm({ ...form, apiKey: e.target.value })}
              />
            </label>

            {form.isEditing && (
              <label className="vo-checkbox-row">
                <input
                  type="checkbox"
                  checked={form.clearApiKey}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      clearApiKey: e.target.checked,
                      apiKey: "",
                    })
                  }
                />
                <span>Clear stored API key on save</span>
              </label>
            )}
          </form>
        </div>

        <footer className="md3-dialog-footer">
          <button type="button" className="md3-btn-text" onClick={onClose}>
            Close
          </button>
          <button
            type="submit"
            form={FORM_ID}
            className="md3-btn-filled"
            disabled={submitting}
          >
            {form.isEditing ? <EditIcon size={14} /> : <PlusIcon size={14} />}
            {submitting
              ? "Saving..."
              : form.isEditing
                ? "Update provider"
                : "Save provider"}
          </button>
        </footer>
      </div>
    </div>
  );
}
