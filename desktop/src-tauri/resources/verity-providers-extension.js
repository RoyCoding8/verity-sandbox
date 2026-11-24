const DEFAULT_CONTEXT_WINDOW = 128000;
const DEFAULT_MAX_TOKENS = 16384;
const DEFAULT_COST = Object.freeze({
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
});
const KEYLESS_FALLBACK_API_KEY = "verity-no-key";

const API_MAP = Object.freeze({
  "openai-completions": "openai-completions",
  openai_compatible: "openai-completions",
  custom: "openai-completions",
  "openai-responses": "openai-responses",
  "anthropic-messages": "anthropic-messages",
  anthropic: "anthropic-messages",
  "google-generative-ai": "google-generative-ai",
  google: "google-generative-ai",
});

function resolvePiApi(rawApi) {
  if (typeof rawApi !== "string") {
    return "openai-completions";
  }
  const normalized = rawApi.trim();
  if (!normalized) {
    return "openai-completions";
  }
  return API_MAP[normalized] || normalized;
}

function buildModels(rawModels) {
  if (!Array.isArray(rawModels)) {
    return [];
  }
  const seen = new Set();
  const models = [];
  for (const item of rawModels) {
    if (typeof item !== "string") {
      continue;
    }
    const id = item.trim();
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    models.push({
      id,
      name: id,
      reasoning: true,
      input: ["text", "image"],
      cost: { ...DEFAULT_COST },
      contextWindow: DEFAULT_CONTEXT_WINDOW,
      maxTokens: DEFAULT_MAX_TOKENS,
    });
  }
  return models;
}

function resolveApiKeyConfig(entry) {
  if (typeof entry.api_key_env === "string") {
    const envName = entry.api_key_env.trim();
    if (envName) {
      return `$${envName}`;
    }
  }
  return KEYLESS_FALLBACK_API_KEY;
}

/**
 * Verity desktop Pi extension: reads `VERITY_DESKTOP_PROVIDERS` from the Pi
 * child-process environment and registers every configured custom provider
 * and model with Pi on startup without touching `~/.pi/agent/`.
 */
export default function verityProvidersExtension(pi) {
  if (!pi || typeof pi.registerProvider !== "function") {
    return;
  }

  const raw = process.env.VERITY_DESKTOP_PROVIDERS;
  if (typeof raw !== "string" || !raw.trim()) {
    return;
  }

  let providers;
  try {
    providers = JSON.parse(raw);
  } catch {
    return;
  }

  if (!Array.isArray(providers)) {
    return;
  }

  for (const entry of providers) {
    if (!entry || typeof entry !== "object") {
      continue;
    }

    const id = typeof entry.id === "string" ? entry.id.trim() : "";
    const baseUrl =
      typeof entry.base_url === "string" ? entry.base_url.trim() : "";
    if (!id || !baseUrl) {
      continue;
    }

    const name =
      typeof entry.name === "string" && entry.name.trim()
        ? entry.name.trim()
        : id;
    const models = buildModels(entry.models);

    const config = {
      name,
      baseUrl,
      apiKey: resolveApiKeyConfig(entry),
      api: resolvePiApi(entry.api),
    };

    if (models.length > 0) {
      config.models = models;
    }

    try {
      pi.registerProvider(id, config);
    } catch {
      // Ignore a single invalid provider entry so other custom providers still load.
    }
  }
}
