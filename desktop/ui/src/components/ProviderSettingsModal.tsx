import { useEffect, useState, type FormEvent } from "react";
import {
  validateExternalSignInUrl,
  validateProviderBaseUrl,
  type ProviderApiKind,
  type ProviderProfileInput,
  type PublicProviderProfile,
} from "../api/tauri";

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

  useEffect(() => {
    if (!open) return;
    function onKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  function loadForEdit(profile: PublicProviderProfile) {
    setError(null);
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

  async function handleDelete(id: string) {
    setError(null);
    try {
      await onDelete(id);
      if (form.id === id) setForm(EMPTY_FORM);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
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
        <div className="md3-dialog-header">
          <div>
            <h2 id="provider-settings-title" className="md3-dialog-title">
              Model Providers
            </h2>
            <p className="md3-dialog-subtitle">
              API keys are stored atomically with 0600 permissions and never echoed back to the UI.
            </p>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close provider settings"
            onClick={onClose}
          >
            ×
          </button>
        </div>

        <div className="md3-dialog-body">
          <div className="pi-provider-list">
            {providers.map((provider) => (
              <div key={provider.id} className="pi-provider-card">
                <div className="pi-tree-node-content">
                  <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
                    <strong>{provider.name}</strong>
                    <code style={{ fontSize: 12, opacity: 0.75 }}>{provider.id}</code>
                    <span className="chip chip-info">{provider.api}</span>
                    <span className={`chip ${provider.has_api_key ? "chip-pass" : "chip-neutral"}`}>
                      {provider.has_api_key ? "Key saved" : "No key"}
                    </span>
                  </div>
                  <div style={{ fontSize: 12.5, marginTop: 4, opacity: 0.85 }}>
                    {provider.base_url}
                  </div>
                  {provider.models.length > 0 && (
                    <div style={{ fontSize: 12, marginTop: 4, opacity: 0.72 }}>
                      Models: {provider.models.join(", ")}
                    </div>
                  )}
                </div>
                <div style={{ display: "flex", gap: 6 }}>
                  <button
                    type="button"
                    className="md3-btn-tonal"
                    onClick={() => loadForEdit(provider)}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className="md3-btn-danger"
                    onClick={() => void handleDelete(provider.id)}
                  >
                    Delete
                  </button>
                </div>
              </div>
            ))}
          </div>

          <form onSubmit={(e) => void handleSubmit(e)} style={{ display: "flex", flexDirection: "column", gap: 12 }}>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <h3 style={{ margin: "6px 0 0", fontSize: 15 }}>
                {form.isEditing ? `Edit "${form.id}"` : "Add provider"}
              </h3>
              {form.isEditing && (
                <button
                  type="button"
                  className="md3-btn-text"
                  onClick={() => setForm(EMPTY_FORM)}
                >
                  Reset form
                </button>
              )}
            </div>

            {error && <div className="md3-error-banner">{error}</div>}

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

              <label className="md3-field">
                <span className="md3-field-label">API Protocol</span>
                <select
                  className="md3-select"
                  value={form.api}
                  onChange={(e) => setForm({ ...form, api: e.target.value as ProviderApiKind })}
                >
                  <option value="openai-completions">openai-completions</option>
                  <option value="openai-responses">openai-responses</option>
                  <option value="anthropic-messages">anthropic-messages</option>
                </select>
              </label>

              <label className="md3-field">
                <span className="md3-field-label">Base URL</span>
                <input
                  className="md3-input"
                  placeholder="https://api.example.com/v1"
                  value={form.baseUrl}
                  onChange={(e) => setForm({ ...form, baseUrl: e.target.value })}
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">Models (comma-separated)</span>
                <input
                  className="md3-input"
                  placeholder="claude-sonnet-4-5, gpt-5"
                  value={form.modelsCsv}
                  onChange={(e) => setForm({ ...form, modelsCsv: e.target.value })}
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">External Sign-in URL (optional, https://)</span>
                <input
                  className="md3-input"
                  placeholder="https://console.example.com"
                  value={form.signInUrl}
                  onChange={(e) => setForm({ ...form, signInUrl: e.target.value })}
                />
              </label>
            </div>

            <label className="md3-field">
              <span className="md3-field-label">
                API Key {form.isEditing ? "(leave blank to keep existing key)" : "(optional)"}
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
              <label style={{ display: "inline-flex", alignItems: "center", gap: 8, fontSize: 13 }}>
                <input
                  type="checkbox"
                  checked={form.clearApiKey}
                  onChange={(e) =>
                    setForm({ ...form, clearApiKey: e.target.checked, apiKey: "" })
                  }
                />
                <span>Clear stored API key on save</span>
              </label>
            )}

            <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
              <button type="button" className="md3-btn-text" onClick={onClose}>
                Close
              </button>
              <button type="submit" className="md3-btn-filled" disabled={submitting}>
                {submitting ? "Saving..." : form.isEditing ? "Update provider" : "Save provider"}
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
}
