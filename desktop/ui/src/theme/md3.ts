import {
  argbFromHex,
  hexFromArgb,
  themeFromSourceColor,
} from "@material/material-color-utilities";

export type ThemeMode = "light" | "dark";

export const DEFAULT_MD3_SEED = "#1A5FB4";

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

/**
 * Pure computation of Google Material Design 3 light or dark color tokens
 * from a hex seed color using `@material/material-color-utilities`.
 */
export function computeMd3Tokens(
  mode: ThemeMode,
  seedHex: string = DEFAULT_MD3_SEED
): Md3ColorTokens {
  const theme = themeFromSourceColor(argbFromHex(seedHex));
  const scheme = mode === "dark" ? theme.schemes.dark : theme.schemes.light;
  const neutral = theme.palettes.neutral;

  const surfaceTones =
    mode === "dark"
      ? {
          surface: 6,
          dim: 6,
          bright: 24,
          lowest: 4,
          low: 10,
          container: 12,
          high: 17,
          highest: 22,
        }
      : {
          surface: 98,
          dim: 87,
          bright: 98,
          lowest: 100,
          low: 96,
          container: 94,
          high: 92,
          highest: 90,
        };

  return {
    primary: hexFromArgb(scheme.primary),
    onPrimary: hexFromArgb(scheme.onPrimary),
    primaryContainer: hexFromArgb(scheme.primaryContainer),
    onPrimaryContainer: hexFromArgb(scheme.onPrimaryContainer),
    secondary: hexFromArgb(scheme.secondary),
    onSecondary: hexFromArgb(scheme.onSecondary),
    secondaryContainer: hexFromArgb(scheme.secondaryContainer),
    onSecondaryContainer: hexFromArgb(scheme.onSecondaryContainer),
    tertiary: hexFromArgb(scheme.tertiary),
    onTertiary: hexFromArgb(scheme.onTertiary),
    tertiaryContainer: hexFromArgb(scheme.tertiaryContainer),
    onTertiaryContainer: hexFromArgb(scheme.onTertiaryContainer),
    error: hexFromArgb(scheme.error),
    onError: hexFromArgb(scheme.onError),
    errorContainer: hexFromArgb(scheme.errorContainer),
    onErrorContainer: hexFromArgb(scheme.onErrorContainer),
    background: hexFromArgb(scheme.background),
    onBackground: hexFromArgb(scheme.onBackground),
    surface: hexFromArgb(neutral.tone(surfaceTones.surface)),
    surfaceDim: hexFromArgb(neutral.tone(surfaceTones.dim)),
    surfaceBright: hexFromArgb(neutral.tone(surfaceTones.bright)),
    surfaceContainerLowest: hexFromArgb(neutral.tone(surfaceTones.lowest)),
    surfaceContainerLow: hexFromArgb(neutral.tone(surfaceTones.low)),
    surfaceContainer: hexFromArgb(neutral.tone(surfaceTones.container)),
    surfaceContainerHigh: hexFromArgb(neutral.tone(surfaceTones.high)),
    surfaceContainerHighest: hexFromArgb(neutral.tone(surfaceTones.highest)),
    onSurface: hexFromArgb(scheme.onSurface),
    surfaceVariant: hexFromArgb(scheme.surfaceVariant),
    onSurfaceVariant: hexFromArgb(scheme.onSurfaceVariant),
    outline: hexFromArgb(scheme.outline),
    outlineVariant: hexFromArgb(scheme.outlineVariant),
    shadow: hexFromArgb(scheme.shadow),
    scrim: hexFromArgb(scheme.scrim),
    inverseSurface: hexFromArgb(scheme.inverseSurface),
    inverseOnSurface: hexFromArgb(scheme.inverseOnSurface),
    inversePrimary: hexFromArgb(scheme.inversePrimary),
  };
}

/**
 * Applies dynamically generated MD3 `--md-sys-color-*` variables to `document.documentElement`.
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
  target.dataset.theme = mode;
  target.style.colorScheme = mode;
  return tokens;
}

/**
 * Detects the system preferred color scheme when no explicit theme is stored.
 */
export function detectPreferredTheme(): ThemeMode {
  if (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  ) {
    return "dark";
  }
  return "light";
}
