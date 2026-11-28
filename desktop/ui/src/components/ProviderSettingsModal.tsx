import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  tauriApi,
  validateExternalSignInUrl,
  validateProviderBaseUrl,
  type ProviderApiKind,
  type ProviderProfileInput,
  type PublicProviderProfile,
} from "../api/tauri";
import {
  AlertIcon,
  CheckIcon,
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
  readonly models: readonly string[];
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
  models: [],
  apiKey: "",
  clearApiKey: false,
  isEditing: false,
};

const FORM_ID = "provider-settings-form";

type ModelScanState =
  | { readonly kind: "idle" }
  | { readonly kind: "loading" }
  | { readonly kind: "ready"; readonly models: readonly string[]; readonly selected: string }
  | { readonly kind: "error"; readonly message: string };

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
  {
    value: "google-generative-ai",
    label: "google-generative-ai",
    description: "Google Generative AI (manual model IDs)",
  },
];

function isProviderApiKind(value: string): value is ProviderApiKind {
  return (
    value === "openai-completions" ||
    value === "openai-responses" ||
    value === "anthropic-messages" ||
    value === "google-generative-ai"
  );
}

function deduplicateModels(raw: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const item of raw) {
    const trimmed = item.trim();
    if (trimmed && !seen.has(trimmed)) {
      seen.add(trimmed);
      result.push(trimmed);
    }
  }
  return result;
}

export function ProviderSettingsModal({
  open,
  providers,
  onSave,
  onDelete,
  onClose,
}: ProviderSettingsModalProps) {
  const [form, setForm] = useState<ProviderFormState>(EMPTY_FORM);
  const [addingModel, setAddingModel] = useState(false);
  const [draftModelId, setDraftModelId] = useState("");
  const [selectedModelId, setSelectedModelId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [scan, setScan] = useState<ModelScanState>({ kind: "idle" });
  const scanGeneration = useRef(0);

  const draftModelInputRef = useRef<HTMLInputElement>(null);

  const isBusy = submitting || deletingId !== null;

  const editingProvider = useMemo(
    () =>
      form.isEditing
        ? providers.find((provider) => provider.id === form.id) ?? null
        : null,
    [form.id, form.isEditing, providers]
  );
  const hasStoredApiKey = Boolean(editingProvider?.has_api_key);

  const modelOptions = useMemo<readonly Md3SelectOption[]>(
    () =>
      form.models.map((modelId) => ({
        value: modelId,
        label: modelId,
      })),
    [form.models]
  );

  function resetFormState() {
    invalidateScan();
    setForm(EMPTY_FORM);
    setAddingModel(false);
    setDraftModelId("");
    setSelectedModelId("");
    setError(null);
  }

  function invalidateScan() {
    scanGeneration.current += 1;
    setScan({ kind: "idle" });
  }

  function updateForm(next: ProviderFormState) {
    invalidateScan();
    setForm(next);
  }

  function closeModal() {
    invalidateScan();
    onClose();
  }

  async function scanModels() {
    const generation = ++scanGeneration.current;
    const baseError = validateProviderBaseUrl(form.baseUrl);
    if (baseError) {
      setScan({ kind: "error", message: baseError });
      return;
    }
    setScan({ kind: "loading" });
    try {
      const models = deduplicateModels(await tauriApi.scanProviderModels({
        provider_id: form.isEditing ? form.id : undefined,
        api: form.api,
        base_url: form.baseUrl.trim(),
        api_key: form.clearApiKey ? "" : form.apiKey.trim() || undefined,
      }));
      if (generation === scanGeneration.current) {
        setScan({ kind: "ready", models, selected: models[0] ?? "" });
      }
    } catch (error: unknown) {
      if (generation === scanGeneration.current) {
        setScan({ kind: "error", message: error instanceof Error ? error.message : String(error) });
      }
    }
  }

  // Reset form edits, error banner, and pending delete confirmation on close/reopen
  useEffect(() => {
    if (!open) {
      invalidateScan();
      setForm(EMPTY_FORM);
      setAddingModel(false);
      setDraftModelId("");
      setSelectedModelId("");
      setError(null);
      setSubmitting(false);
      setPendingDeleteId(null);
      setDeletingId(null);
      return;
    }
    function onKeyDown(e: KeyboardEvent) {
      if (e.defaultPrevented) return;
      if (e.key === "Escape") closeModal();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  useEffect(() => () => { scanGeneration.current += 1; }, [open]);

  useEffect(() => {
    if (addingModel) {
      draftModelInputRef.current?.focus();
    }
  }, [addingModel]);

  if (!open) return null;

  function loadForEdit(profile: PublicProviderProfile) {
    invalidateScan();
    const uniqueModels = deduplicateModels(profile.models);
    setError(null);
    setPendingDeleteId(null);
    setAddingModel(false);
    setDraftModelId("");
    setSelectedModelId(uniqueModels[0] ?? "");
    setForm({
      id: profile.id,
      name: profile.name,
      api: profile.api,
      baseUrl: profile.base_url,
      signInUrl: profile.sign_in_url ?? "",
      models: uniqueModels,
      apiKey: "",
      clearApiKey: false,
      isEditing: true,
    });
  }

  function handleSaveModel() {
    const trimmed = draftModelId.trim();
    if (!trimmed) return;
    setForm((prev) => {
      if (prev.models.includes(trimmed)) {
        return prev;
      }
      return { ...prev, models: [...prev.models, trimmed] };
    });
    setSelectedModelId(trimmed);
    setDraftModelId("");
    setAddingModel(false);
  }

  function handleRemoveSelectedModel() {
    if (!selectedModelId) return;
    const nextModels = form.models.filter((m) => m !== selectedModelId);
    setForm((prev) => ({
      ...prev,
      models: prev.models.filter((m) => m !== selectedModelId),
    }));
    setSelectedModelId(nextModels[0] ?? "");
  }

  function handleDraftModelKeyDown(e: ReactKeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      handleSaveModel();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setAddingModel(false);
      setDraftModelId("");
    }
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    if (isBusy) return;

    const id = form.id.trim();
    const name = form.name.trim();
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(id)) {
      setError("Provider ID must be 1–64 ASCII letters, digits, '-' or '_'.");
      return;
    }
    if (!form.isEditing && providers.some((p) => p.id === id)) {
      setError(
        `Provider ID "${id}" already exists. Choose a different ID or edit the existing provider.`
      );
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

    const models = deduplicateModels(form.models);

    // Tri-state api_key: "" clears key, non-empty replaces key, undefined preserves existing key
    const apiKeyPayload =
      hasStoredApiKey && form.clearApiKey
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
      resetFormState();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  async function handleConfirmDelete(id: string) {
    if (isBusy) return;
    setDeletingId(id);
    setError(null);
    try {
      await onDelete(id);
      if (form.id === id) {
        resetFormState();
      }
      setPendingDeleteId(null);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="md3-scrim" onClick={closeModal}>
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
                API keys are stored locally and never echoed back to the UI.
                File permissions are restricted to the owner on Unix; Windows uses inherited permissions.
              </p>
            </div>
          </div>
          <button
            type="button"
            className="md3-icon-btn"
            aria-label="Close provider settings"
            onClick={closeModal}
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
                const safeSignInUrl =
                  provider.sign_in_url &&
                  validateExternalSignInUrl(provider.sign_in_url) === null
                    ? provider.sign_in_url
                    : null;
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
                      {safeSignInUrl && !isConfirmingDelete && (
                        <a
                          href={safeSignInUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="md3-btn-outlined vo-provider-signin-link"
                        >
                          Sign in
                        </a>
                      )}
                      {isConfirmingDelete ? (
                        <div className="vo-confirm-delete-group">
                          <button
                            type="button"
                            className="md3-btn-danger"
                            disabled={isBusy}
                            onClick={() => void handleConfirmDelete(provider.id)}
                          >
                            <TrashIcon size={14} />
                            {isDeleting ? "Deleting..." : "Confirm delete"}
                          </button>
                          <button
                            type="button"
                            className="md3-btn-text"
                            disabled={isBusy}
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
                            disabled={isBusy}
                            onClick={() => loadForEdit(provider)}
                          >
                            <EditIcon size={14} />
                            Edit
                          </button>
                          <button
                            type="button"
                            className="md3-btn-danger"
                            disabled={isBusy}
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
                  disabled={isBusy}
                  onClick={resetFormState}
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
                  disabled={form.isEditing || isBusy}
                  onChange={(e) => updateForm({ ...form, id: e.target.value })}
                />
              </label>

              <label className="md3-field">
                <span className="md3-field-label">Display Name</span>
                <input
                  className="md3-input"
                  placeholder="e.g. OpenRouter"
                  value={form.name}
                  disabled={isBusy}
                  onChange={(e) => updateForm({ ...form, name: e.target.value })}
                />
              </label>

              <div className="md3-field">
                <span className="md3-field-label">API Protocol</span>
                <Md3Select
                  variant="field"
                  placement="bottom"
                  align="start"
                  ariaLabel="API Protocol"
                  disabled={isBusy}
                  value={form.api}
                  options={API_PROTOCOL_OPTIONS}
                  onChange={(nextValue) => {
                    if (isProviderApiKind(nextValue)) {
                      updateForm({ ...form, api: nextValue });
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
                  disabled={isBusy}
                  onChange={(e) =>
                    updateForm({ ...form, baseUrl: e.target.value })
                  }
                />
              </label>

              <div className="md3-field vo-models-field">
                <div className="vo-models-field-header">
                  <span className="md3-field-label">
                    Models ({form.models.length})
                  </span>
                  <button
                    type="button"
                    className="md3-btn-tonal vo-add-model-btn"
                    disabled={isBusy}
                    onClick={() => {
                      setAddingModel(true);
                      setDraftModelId("");
                    }}
                  >
                    <PlusIcon size={14} />
                    Add model
                  </button>
                </div>

                {addingModel && (
                  <div className="vo-model-add-row">
                    <input
                      ref={draftModelInputRef}
                      className="md3-input"
                      aria-label="New model ID"
                      placeholder="e.g. claude-sonnet-5-5"
                      value={draftModelId}
                      disabled={isBusy}
                      onChange={(e) => setDraftModelId(e.target.value)}
                      onKeyDown={handleDraftModelKeyDown}
                    />
                    <button
                      type="button"
                      className="md3-btn-filled"
                      disabled={isBusy || draftModelId.trim().length === 0}
                      onClick={handleSaveModel}
                    >
                      <CheckIcon size={14} />
                      Save model
                    </button>
                    <button
                      type="button"
                      className="md3-btn-text"
                      disabled={isBusy}
                      onClick={() => {
                        setAddingModel(false);
                        setDraftModelId("");
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                )}

                <div className="vo-model-scan">
                  <button
                    type="button"
                    className="md3-btn-tonal"
                    disabled={isBusy || scan.kind === "loading" || form.api === "google-generative-ai"}
                    onClick={() => void scanModels()}
                  >
                    {scan.kind === "loading" ? "Scanning..." : "Scan for models"}
                  </button>
                  <div role="status" aria-live="polite">
                    {form.api === "google-generative-ai"
                      ? "Scanning is unavailable for Google Generative AI. Add model IDs manually."
                      : scan.kind === "loading" ? "Requesting model catalog..."
                      : scan.kind === "error" ? scan.message
                      : scan.kind === "ready" ? (scan.models.length === 0 ? "No models found. You can add an ID manually." : `Found ${scan.models.length} models. Select one to add.`)
                      : "Scan the endpoint without saving this provider."}
                  </div>
                  {scan.kind === "ready" && scan.models.length > 0 && (
                    <div className="vo-model-select-row">
                      <div className="vo-model-select-wrap">
                        <Md3Select
                          variant="field"
                          ariaLabel="Discovered models"
                          value={scan.selected}
                          options={scan.models.map((id) => ({ value: id, label: id }))}
                          disabled={isBusy}
                          onChange={(selected) => setScan({ ...scan, selected })}
                        />
                      </div>
                      <button
                        type="button"
                        className="md3-btn-tonal"
                        disabled={isBusy || !scan.selected || form.models.includes(scan.selected)}
                        onClick={() => {
                          const selected = scan.selected;
                          setForm((prev) => ({ ...prev, models: deduplicateModels([...prev.models, selected]) }));
                          setSelectedModelId(selected);
                        }}
                      >
                        Add selected model
                      </button>
                    </div>
                  )}
                </div>

                <div className="vo-model-select-row">
                  <div className="vo-model-select-wrap">
                    <Md3Select
                      variant="field"
                      placement="bottom"
                      align="start"
                      ariaLabel="Saved models"
                      placeholder="No models added yet"
                      disabled={form.models.length === 0 || isBusy}
                      value={selectedModelId}
                      options={modelOptions}
                      onChange={setSelectedModelId}
                    />
                  </div>
                  <button
                    type="button"
                    className="md3-btn-danger"
                    disabled={!selectedModelId || isBusy}
                    onClick={handleRemoveSelectedModel}
                  >
                    <TrashIcon size={14} />
                    Remove
                  </button>
                </div>
              </div>

              <label className="md3-field vo-models-field">
                <span className="md3-field-label">
                  External Sign-in URL (optional, https://)
                </span>
                <input
                  className="md3-input"
                  placeholder="https://console.example.com"
                  value={form.signInUrl}
                  disabled={isBusy}
                  onChange={(e) =>
                    updateForm({ ...form, signInUrl: e.target.value })
                  }
                />
              </label>
            </div>

            <label className="md3-field">
              <span className="md3-field-label">
                API Key{" "}
                {hasStoredApiKey
                  ? "(leave blank to keep existing key)"
                  : "(optional)"}
              </span>
              <input
                type="password"
                className="md3-input"
                autoComplete="off"
                placeholder="sk-..."
                value={form.apiKey}
                disabled={form.clearApiKey || isBusy}
                onChange={(e) => updateForm({ ...form, apiKey: e.target.value })}
              />
            </label>

            {hasStoredApiKey && (
              <label className="vo-checkbox-row">
                <input
                  type="checkbox"
                  checked={form.clearApiKey}
                  disabled={isBusy}
                  onChange={(e) =>
                    updateForm({
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
          <button type="button" className="md3-btn-text" onClick={closeModal}>
            Close
          </button>
          <button
            type="submit"
            form={FORM_ID}
            className="md3-btn-filled"
            disabled={isBusy}
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
