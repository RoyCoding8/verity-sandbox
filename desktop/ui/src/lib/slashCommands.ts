import type { PiCommandInfo, PiCommandSource } from "../api/tauri";

export type BuiltinSlashCommandName =
  | "new"
  | "name"
  | "compact"
  | "fork"
  | "tree"
  | "export"
  | "session"
  | "copy"
  | "model"
  | "thinking"
  | "settings";

export interface MergedSlashCommand {
  readonly trigger: "/";
  readonly name: string;
  readonly normalizedName: string;
  readonly description: string;
  readonly usage?: string;
  readonly insertText: string;
  readonly source: PiCommandSource;
  readonly location?: string;
}

export interface BashPrefixInvocation {
  readonly kind: "bash";
  readonly command: string;
  readonly excludeFromContext: boolean;
}

export interface BuiltinSlashInvocation {
  readonly command: BuiltinSlashCommandName;
  readonly args: string;
}

/**
 * Strips any leading `/` characters and surrounding whitespace from a slash command name.
 */
export function normalizeSlashName(name: string): string {
  return name.trim().replace(/^\/+/, "").trim();
}

/**
 * Ensures a slash command name has exactly one leading `/`.
 */
export function displaySlashName(name: string): string {
  const normalized = normalizeSlashName(name);
  return `/${normalized}`;
}

export const BUILTIN_SLASH_COMMANDS: readonly MergedSlashCommand[] = [
  {
    trigger: "/",
    name: "/new",
    normalizedName: "new",
    description: "Start a new session",
    usage: "/new",
    insertText: "/new",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/name",
    normalizedName: "name",
    description: "Rename the current session",
    usage: "/name <title>",
    insertText: "/name ",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/compact",
    normalizedName: "compact",
    description: "Compact conversation context with optional focus instructions",
    usage: "/compact [instructions]",
    insertText: "/compact ",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/fork",
    normalizedName: "fork",
    description: "Fork conversation from an earlier message",
    usage: "/fork",
    insertText: "/fork",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/tree",
    normalizedName: "tree",
    description: "Open session branching history tree",
    usage: "/tree",
    insertText: "/tree",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/export",
    normalizedName: "export",
    description: "Export current session to HTML",
    usage: "/export",
    insertText: "/export",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/session",
    normalizedName: "session",
    description: "Show session statistics and context usage",
    usage: "/session",
    insertText: "/session",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/copy",
    normalizedName: "copy",
    description: "Copy the last assistant response to clipboard",
    usage: "/copy",
    insertText: "/copy",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/model",
    normalizedName: "model",
    description: "Cycle or switch active model",
    usage: "/model [name]",
    insertText: "/model ",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/thinking",
    normalizedName: "thinking",
    description: "Cycle or set thinking level",
    usage: "/thinking [level]",
    insertText: "/thinking ",
    source: "builtin",
  },
  {
    trigger: "/",
    name: "/settings",
    normalizedName: "settings",
    description: "Open session settings, stats, and skills drawer",
    usage: "/settings",
    insertText: "/settings",
    source: "builtin",
  },
];

const BUILTIN_COMMAND_SET = new Set<string>(
  BUILTIN_SLASH_COMMANDS.map((cmd) => cmd.normalizedName)
);

function isBuiltinCommandName(name: string): name is BuiltinSlashCommandName {
  return BUILTIN_COMMAND_SET.has(name);
}

function inferCommandSource(cmd: PiCommandInfo, normalizedName: string): PiCommandSource {
  if (cmd.source && cmd.source.trim().length > 0) {
    return cmd.source;
  }
  if (normalizedName.startsWith("skill:")) {
    return "skill";
  }
  if (normalizedName.startsWith("prompt:")) {
    return "prompt";
  }
  if (isBuiltinCommandName(normalizedName.toLowerCase())) {
    return "builtin";
  }
  return "extension";
}

/**
 * Merges built-in slash commands with live Pi commands (`get_commands`),
 * deduplicating by normalized lowercase name and preserving `source` and `location`.
 */
export function mergeSlashCommands(
  piCommands: readonly PiCommandInfo[]
): MergedSlashCommand[] {
  const byKey = new Map<string, MergedSlashCommand>();

  for (const builtin of BUILTIN_SLASH_COMMANDS) {
    byKey.set(builtin.normalizedName.toLowerCase(), builtin);
  }

  for (const cmd of piCommands) {
    if (cmd.trigger === "$" || cmd.name.trim().startsWith("$")) {
      continue;
    }
    const normalizedName = normalizeSlashName(cmd.name);
    if (!normalizedName) continue;

    const key = normalizedName.toLowerCase();
    const displayName = displaySlashName(normalizedName);
    const existing = byKey.get(key);
    const source = inferCommandSource(cmd, normalizedName);

    if (existing) {
      byKey.set(key, {
        ...existing,
        description: cmd.description.trim() || existing.description,
        source: existing.source === "builtin" ? "builtin" : source,
        location: cmd.location ?? existing.location,
      });
    } else {
      const rawInsert = cmd.insertText?.trim();
      const insertText =
        rawInsert && rawInsert.length > 0
          ? rawInsert.startsWith("/")
            ? cmd.insertText
            : displaySlashName(rawInsert) + (cmd.insertText.endsWith(" ") ? " " : "")
          : `${displayName} `;
      byKey.set(key, {
        trigger: "/",
        name: displayName,
        normalizedName,
        description: cmd.description.trim(),
        insertText,
        source,
        location: cmd.location,
      });
    }
  }

  return Array.from(byKey.values());
}

/**
 * Filters slash commands against user input with or without a leading `/`,
 * ranking exact matches first, then prefix matches on `normalizedName`,
 * then substring matches on `normalizedName`, then description matches.
 */
export function filterSlashCommands(
  commands: readonly MergedSlashCommand[],
  input: string
): MergedSlashCommand[] {
  const query = normalizeSlashName(input).toLowerCase();
  if (!query) {
    return [...commands];
  }

  const ranked: Array<{ readonly rank: number; readonly index: number; readonly cmd: MergedSlashCommand }> = [];

  for (let i = 0; i < commands.length; i += 1) {
    const cmd = commands[i];
    const normLower = cmd.normalizedName.toLowerCase();
    const descLower = cmd.description.toLowerCase();

    if (normLower === query) {
      ranked.push({ rank: 0, index: i, cmd });
    } else if (normLower.startsWith(query)) {
      ranked.push({ rank: 1, index: i, cmd });
    } else if (normLower.includes(query)) {
      ranked.push({ rank: 2, index: i, cmd });
    } else if (descLower.includes(query)) {
      ranked.push({ rank: 3, index: i, cmd });
    }
  }

  ranked.sort((a, b) => (a.rank !== b.rank ? a.rank - b.rank : a.index - b.index));
  return ranked.map((entry) => entry.cmd);
}

/**
 * Parses `!cmd` (include in context) and `!!cmd` (exclude from context) bash shortcuts.
 */
export function parseBashPrefix(input: string): BashPrefixInvocation | null {
  const trimmed = input.trim();
  if (trimmed.startsWith("!!")) {
    const command = trimmed.slice(2).trim();
    if (!command) return null;
    return {
      kind: "bash",
      command,
      excludeFromContext: true,
    };
  }
  if (trimmed.startsWith("!")) {
    const command = trimmed.slice(1).trim();
    if (!command) return null;
    return {
      kind: "bash",
      command,
      excludeFromContext: false,
    };
  }
  return null;
}

/**
 * Parses built-in slash command invocations such as `/new`, `/name <title>`,
 * `/compact [instructions]`, `/fork`, `/tree`, `/export`, `/session`,
 * `/copy`, `/model [name]`, `/thinking [level]`, and `/settings`.
 */
export function parseBuiltinSlashInvocation(
  input: string
): BuiltinSlashInvocation | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith("/")) {
    return null;
  }
  const rest = trimmed.slice(1);
  if (!rest || rest.startsWith("/")) {
    return null;
  }
  const wsIdx = rest.search(/\s/);
  const rawName = (wsIdx === -1 ? rest : rest.slice(0, wsIdx)).toLowerCase();
  if (!isBuiltinCommandName(rawName)) {
    return null;
  }
  const args = wsIdx === -1 ? "" : rest.slice(wsIdx).trim();
  return {
    command: rawName,
    args,
  };
}
