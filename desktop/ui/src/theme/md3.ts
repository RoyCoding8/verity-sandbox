import {
  argbFromHex,
  hexFromArgb,
  themeFromSourceColor,
} from "@material/material-color-utilities";

export type ThemeMode = "light" | "dark";

export const DEFAULT_MD3_SEED = "#0B57D0";

export interface VeritySemanticTokens {
  readonly success: string;
  readonly onSuccess: string;
  readonly successContainer: string;
  readonly onSuccessContainer: string;
  readonly warning: string;
  readonly onWarning: string;
  readonly warningContainer: string;
  readonly onWarningContainer: string;
  readonly danger: string;
  readonly onDanger: string;
  readonly dangerContainer: string;
  readonly onDangerContainer: string;
  readonly info: string;
  readonly infoContainer: string;
  readonly onInfoContainer: string;
}

export interface Md3ColorTokens {
  readonly primary: string;
  readonly onPrimary: string;
  readonly primaryContainer: string;
  readonly onPrimaryContainer: string;
  readonly secondary: string;
  readonly onSecondary: string;
  readonly secondaryContainer: string;
  readonly onSecondaryContainer: string;
  readonly tertiary: string;
  readonly onTertiary: string;
  readonly tertiaryContainer: string;
  readonly onTertiaryContainer: string;
  readonly error: string;
  readonly onError: string;
  readonly errorContainer: string;
  readonly onErrorContainer: string;
  readonly background: string;
  readonly onBackground: string;
  readonly surface: string;
  readonly surfaceDim: string;
  readonly surfaceBright: string;
  readonly surfaceContainerLowest: string;
  readonly surfaceContainerLow: string;
  readonly surfaceContainer: string;
  readonly surfaceContainerHigh: string;
  readonly surfaceContainerHighest: string;
  readonly onSurface: string;
  readonly surfaceVariant: string;
  readonly onSurfaceVariant: string;
  readonly outline: string;
  readonly outlineVariant: string;
  readonly shadow: string;
  readonly scrim: string;
  readonly inverseSurface: string;
  readonly inverseOnSurface: string;
  readonly inversePrimary: string;
}

export interface VeritySurfaceEffects {
  readonly ambientGlow: string;
  readonly surfaceHighlight: string;
  readonly hairline: string;
  readonly hairlineSubtle: string;
  readonly scrim: string;
}

const LIGHT_WORKBENCH_TOKENS: Md3ColorTokens = {
  primary: "#0B57D0",
  onPrimary: "#FFFFFF",
  primaryContainer: "#D3E3FD",
  onPrimaryContainer: "#041E49",
  secondary: "#444746",
  onSecondary: "#FFFFFF",
  secondaryContainer: "#E9EEF6",
  onSecondaryContainer: "#1F1F1F",
  tertiary: "#146C2E",
  onTertiary: "#FFFFFF",
  tertiaryContainer: "#E6F4EA",
  onTertiaryContainer: "#0F5223",
  error: "#B3261E",
  onError: "#FFFFFF",
  errorContainer: "#FCE8E6",
  onErrorContainer: "#8C1D18",
  background: "#F8FAFD",
  onBackground: "#1F1F1F",
  surface: "#F8FAFD",
  surfaceDim: "#E9EEF6",
  surfaceBright: "#FFFFFF",
  surfaceContainerLowest: "#FFFFFF",
  surfaceContainerLow: "#F0F4F9",
  surfaceContainer: "#FFFFFF",
  surfaceContainerHigh: "#E9EEF6",
  surfaceContainerHighest: "#DDE3EA",
  onSurface: "#1F1F1F",
  surfaceVariant: "#E9EEF6",
  onSurfaceVariant: "#444746",
  outline: "rgba(15, 23, 42, 0.16)",
  outlineVariant: "rgba(15, 23, 42, 0.08)",
  shadow: "#0F172A",
  scrim: "#0F172A",
  inverseSurface: "#1F1F1F",
  inverseOnSurface: "#F8FAFD",
  inversePrimary: "#8AB4F8",
};

const DARK_WORKBENCH_TOKENS: Md3ColorTokens = {
  primary: "#8AB4F8",
  onPrimary: "#0B1A33",
  primaryContainer: "rgba(138, 180, 248, 0.14)",
  onPrimaryContainer: "#D2E3FC",
  secondary: "#B4BBC8",
  onSecondary: "#14171F",
  secondaryContainer: "rgba(255, 255, 255, 0.08)",
  onSecondaryContainer: "#ECEEF2",
  tertiary: "#81C995",
  onTertiary: "#042F22",
  tertiaryContainer: "rgba(129, 201, 149, 0.11)",
  onTertiaryContainer: "#A8DAB5",
  error: "#F28B82",
  onError: "#3B0A0A",
  errorContainer: "rgba(242, 139, 130, 0.11)",
  onErrorContainer: "#F6AEA9",
  background: "#0B0C0F",
  onBackground: "#ECEEF2",
  surface: "#0B0C0F",
  surfaceDim: "#08090C",
  surfaceBright: "#2A2E38",
  surfaceContainerLowest: "#08090C",
  surfaceContainerLow: "#121318",
  surfaceContainer: "#191B21",
  surfaceContainerHigh: "#21242C",
  surfaceContainerHighest: "#2A2E38",
  onSurface: "#ECEEF2",
  surfaceVariant: "#1F222A",
  onSurfaceVariant: "#9AA0AE",
  outline: "rgba(255, 255, 255, 0.14)",
  outlineVariant: "rgba(255, 255, 255, 0.07)",
  shadow: "#000000",
  scrim: "#000000",
  inverseSurface: "#ECEEF2",
  inverseOnSurface: "#0B0C0F",
  inversePrimary: "#0B57D0",
};

const LIGHT_SEMANTIC_TOKENS: VeritySemanticTokens = {
  success: "#146C2E",
  onSuccess: "#FFFFFF",
  successContainer: "#E6F4EA",
  onSuccessContainer: "#0F5223",
  warning: "#8C5300",
  onWarning: "#FFFFFF",
  warningContainer: "#FEF3C7",
  onWarningContainer: "#6E4000",
  danger: "#B3261E",
  onDanger: "#FFFFFF",
  dangerContainer: "#FCE8E6",
  onDangerContainer: "#8C1D18",
  info: "#0B57D0",
  infoContainer: "#E8F0FE",
  onInfoContainer: "#0842A0",
};

const DARK_SEMANTIC_TOKENS: VeritySemanticTokens = {
  success: "#81C995",
  onSuccess: "#042F22",
  successContainer: "rgba(129, 201, 149, 0.11)",
  onSuccessContainer: "#A8DAB5",
  warning: "#F2B866",
  onWarning: "#3B1E04",
  warningContainer: "rgba(242, 184, 102, 0.11)",
  onWarningContainer: "#F8D093",
  danger: "#F28B82",
  onDanger: "#3B0A0A",
  dangerContainer: "rgba(242, 139, 130, 0.11)",
  onDangerContainer: "#F6AEA9",
  info: "#8AB4F8",
  infoContainer: "rgba(138, 180, 248, 0.12)",
  onInfoContainer: "#AECBFA",
};

const LIGHT_SURFACE_EFFECTS: VeritySurfaceEffects = {
  ambientGlow:
    "radial-gradient(ellipse 68% 54% at 50% 42%, rgba(11, 87, 208, 0.065) 0%, rgba(138, 180, 248, 0.025) 45%, transparent 72%)",
  surfaceHighlight:
    "0 1px 3px rgba(15, 23, 42, 0.05), 0 0 0 1px rgba(15, 23, 42, 0.05)",
  hairline: "rgba(15, 23, 42, 0.08)",
  hairlineSubtle: "rgba(15, 23, 42, 0.05)",
  scrim: "rgba(15, 23, 42, 0.36)",
};

const DARK_SURFACE_EFFECTS: VeritySurfaceEffects = {
  ambientGlow:
    "radial-gradient(ellipse 64% 52% at 50% 42%, rgba(28, 58, 138, 0.24) 0%, rgba(18, 34, 82, 0.10) 45%, transparent 72%)",
  surfaceHighlight: "inset 0 1px 0 rgba(255, 255, 255, 0.05)",
  hairline: "rgba(255, 255, 255, 0.06)",
  hairlineSubtle: "rgba(255, 255, 255, 0.04)",
  scrim: "rgba(5, 6, 9, 0.68)",
};

const TOKEN_TO_CSS_VAR: ReadonlyArray<readonly [keyof Md3ColorTokens, string]> = [
  ["primary", "--md-sys-color-primary"],
  ["onPrimary", "--md-sys-color-on-primary"],
  ["primaryContainer", "--md-sys-color-primary-container"],
  ["onPrimaryContainer", "--md-sys-color-on-primary-container"],
  ["secondary", "--md-sys-color-secondary"],
  ["onSecondary", "--md-sys-color-on-secondary"],
  ["secondaryContainer", "--md-sys-color-secondary-container"],
  ["onSecondaryContainer", "--md-sys-color-on-secondary-container"],
  ["tertiary", "--md-sys-color-tertiary"],
  ["onTertiary", "--md-sys-color-on-tertiary"],
  ["tertiaryContainer", "--md-sys-color-tertiary-container"],
  ["onTertiaryContainer", "--md-sys-color-on-tertiary-container"],
  ["error", "--md-sys-color-error"],
  ["onError", "--md-sys-color-on-error"],
  ["errorContainer", "--md-sys-color-error-container"],
  ["onErrorContainer", "--md-sys-color-on-error-container"],
  ["background", "--md-sys-color-background"],
  ["onBackground", "--md-sys-color-on-background"],
  ["surface", "--md-sys-color-surface"],
  ["surfaceDim", "--md-sys-color-surface-dim"],
  ["surfaceBright", "--md-sys-color-surface-bright"],
  ["surfaceContainerLowest", "--md-sys-color-surface-container-lowest"],
  ["surfaceContainerLow", "--md-sys-color-surface-container-low"],
  ["surfaceContainer", "--md-sys-color-surface-container"],
  ["surfaceContainerHigh", "--md-sys-color-surface-container-high"],
  ["surfaceContainerHighest", "--md-sys-color-surface-container-highest"],
  ["onSurface", "--md-sys-color-on-surface"],
  ["surfaceVariant", "--md-sys-color-surface-variant"],
  ["onSurfaceVariant", "--md-sys-color-on-surface-variant"],
  ["outline", "--md-sys-color-outline"],
  ["outlineVariant", "--md-sys-color-outline-variant"],
  ["shadow", "--md-sys-color-shadow"],
  ["scrim", "--md-sys-color-scrim"],
  ["inverseSurface", "--md-sys-color-inverse-surface"],
  ["inverseOnSurface", "--md-sys-color-inverse-on-surface"],
  ["inversePrimary", "--md-sys-color-inverse-primary"],
];

const SEMANTIC_TO_CSS_VAR: ReadonlyArray<
  readonly [keyof VeritySemanticTokens, string]
> = [
  ["success", "--verity-success"],
  ["onSuccess", "--verity-on-success"],
  ["successContainer", "--verity-success-container"],
  ["onSuccessContainer", "--verity-on-success-container"],
  ["warning", "--verity-warning"],
  ["onWarning", "--verity-on-warning"],
  ["warningContainer", "--verity-warning-container"],
  ["onWarningContainer", "--verity-on-warning-container"],
  ["danger", "--verity-danger"],
  ["onDanger", "--verity-on-danger"],
  ["dangerContainer", "--verity-danger-container"],
  ["onDangerContainer", "--verity-on-danger-container"],
  ["info", "--verity-info"],
  ["infoContainer", "--verity-info-container"],
  ["onInfoContainer", "--verity-on-info-container"],
];

const SURFACE_EFFECT_TO_CSS_VAR: ReadonlyArray<
  readonly [keyof VeritySurfaceEffects, string]
> = [
  ["ambientGlow", "--verity-ambient-glow"],
  ["surfaceHighlight", "--verity-surface-highlight"],
  ["hairline", "--verity-hairline"],
  ["hairlineSubtle", "--verity-hairline-subtle"],
  ["scrim", "--verity-scrim"],
];

/**
 * Computes Material Design 3 tonal color tokens for light or dark mode.
 * When a custom seed hex is provided, primary accent roles are derived from that seed
 * while preserving the tonal surface ladder and neutral ink contrast.
 */
export function computeMd3Tokens(
  mode: ThemeMode,
  seedHex: string = DEFAULT_MD3_SEED
): Md3ColorTokens {
  const base = mode === "dark" ? DARK_WORKBENCH_TOKENS : LIGHT_WORKBENCH_TOKENS;
  if (seedHex.trim().toUpperCase() === DEFAULT_MD3_SEED) {
    return base;
  }

  const theme = themeFromSourceColor(argbFromHex(seedHex));
  const scheme = mode === "dark" ? theme.schemes.dark : theme.schemes.light;

  return {
    ...base,
    primary: hexFromArgb(scheme.primary),
    onPrimary: hexFromArgb(scheme.onPrimary),
    primaryContainer: hexFromArgb(scheme.primaryContainer),
    onPrimaryContainer: hexFromArgb(scheme.onPrimaryContainer),
    inversePrimary: hexFromArgb(scheme.inversePrimary),
  };
}

/**
 * Returns semantic status tokens (success, warning, danger, info) for the given theme mode.
 */
export function computeVeritySemanticTokens(
  mode: ThemeMode
): VeritySemanticTokens {
  return mode === "dark" ? DARK_SEMANTIC_TOKENS : LIGHT_SEMANTIC_TOKENS;
}

/**
 * Returns adaptive surface effect, hairline, and scrim tokens for the given theme mode.
 */
export function computeVeritySurfaceEffects(
  mode: ThemeMode
): VeritySurfaceEffects {
  return mode === "dark" ? DARK_SURFACE_EFFECTS : LIGHT_SURFACE_EFFECTS;
}

/**
 * Applies `--md-sys-color-*`, `--verity-*` semantic status tokens, and atmospheric
 * surface effect, hairline, and scrim tokens to target.
 */
export function applyMd3Theme(
  mode: ThemeMode,
  seedHex: string = DEFAULT_MD3_SEED,
  target: HTMLElement = document.documentElement
): Md3ColorTokens {
  const tokens = computeMd3Tokens(mode, seedHex);
  for (const [key, cssVar] of TOKEN_TO_CSS_VAR) {
    target.style.setProperty(cssVar, tokens[key]);
  }

  const semantic = computeVeritySemanticTokens(mode);
  for (const [key, cssVar] of SEMANTIC_TO_CSS_VAR) {
    target.style.setProperty(cssVar, semantic[key]);
  }

  const effects = computeVeritySurfaceEffects(mode);
  for (const [key, cssVar] of SURFACE_EFFECT_TO_CSS_VAR) {
    target.style.setProperty(cssVar, effects[key]);
  }

  target.dataset.theme = mode;
  target.style.colorScheme = mode;
  return tokens;
}

/**
 * Detects the preferred color scheme when no explicit theme is stored,
 * defaulting to dark mode unless the system explicitly prefers light mode.
 */
export function detectPreferredTheme(): ThemeMode {
  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: light)").matches
  ) {
    return "light";
  }
  return "dark";
}
