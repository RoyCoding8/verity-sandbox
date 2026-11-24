import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PiChatMessage, PiToolCallState } from "../api/tauri";
import type { RetryBannerState } from "../state/piStore";
import {
  AlertIcon,
  CheckIcon,
  ChevronRightIcon,
  CopyIcon,
  GitBranchIcon,
  InfoIcon,
  ShieldCheckIcon,
  TerminalIcon,
  VerityLogoMark,
} from "./Icons";
import "./ChatTranscript.css";

type TranscriptMessage = PiChatMessage & {
  readonly stopReason?: string | null;
  readonly errorMessage?: string | null;
};

export interface ChatTranscriptProps {
  readonly messages: readonly TranscriptMessage[];
  readonly isStreaming: boolean;
  readonly liveTools: readonly PiToolCallState[];
  readonly retryBanner: RetryBannerState | null;
  readonly onAbortRetry: () => void;
  readonly onPickStarterPrompt?: (prompt: string) => void;
}

// ============================================================
// External Links & Attachment Boundary Validation
// ============================================================

const EXTERNAL_URL_RE = /^https?:\/\//i;

const ALLOWED_RASTER_MIMES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif",
]);

function isAllowedExternalUrl(url: string): boolean {
  return EXTERNAL_URL_RE.test(url);
}

function isAllowedRasterMime(mime: string): boolean {
  return ALLOWED_RASTER_MIMES.has(mime.trim().toLowerCase());
}

function handleExternalLinkClick(
  event: MouseEvent<HTMLAnchorElement>,
  url: string
): void {
  event.preventDefault();
  if (!isAllowedExternalUrl(url) || typeof window === "undefined") {
    return;
  }
  if ("__TAURI_INTERNALS__" in window) {
    void invoke("pi_open_external_url", { url }).catch(() => {
      window.open(url, "_blank", "noopener,noreferrer");
    });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

// ============================================================
// Lightweight Syntax Highlighter (Pure TS, Precompiled Regexes)
// ============================================================

type SyntaxLang = "rust" | "ts" | "lean" | "json" | "bash" | "plain";
type TokenKind = "plain" | "keyword" | "string" | "comment" | "number" | "type" | "property";

interface CodeToken {
  readonly kind: TokenKind;
  readonly text: string;
}

function normalizeLang(raw: string): SyntaxLang {
  const lower = raw.trim().toLowerCase();
  if (lower === "rust" || lower === "rs") return "rust";
  if (
    lower === "ts" ||
    lower === "tsx" ||
    lower === "typescript" ||
    lower === "js" ||
    lower === "jsx" ||
    lower === "javascript"
  ) {
    return "ts";
  }
  if (lower === "lean" || lower === "lean4") return "lean";
  if (lower === "json" || lower === "jsonc") return "json";
  if (lower === "bash" || lower === "sh" || lower === "shell" || lower === "zsh") {
    return "bash";
  }
  return "plain";
}

interface TokenRule {
  readonly kind: TokenKind;
  readonly pattern: string;
}

const LANGUAGE_RULES: Record<Exclude<SyntaxLang, "plain">, readonly TokenRule[]> = {
  rust: [
    { kind: "comment", pattern: "//[^\\n]*|/\\*[\\s\\S]*?\\*/" },
    {
      kind: "string",
      pattern:
        'b?r(?<rawHashes>#*)"[\\s\\S]*?"\\k<rawHashes>|"(?:\\\\.|[^"\\\\])*"|\'(?:\\\\.|[^\'\\\\])\'',
    },
    { kind: "property", pattern: "#!?\\[[^\\]]*\\]" },
    {
      kind: "keyword",
      pattern:
        "\\b(?:as|async|await|break|const|continue|crate|dyn|else|enum|extern|false|fn|for|if|impl|in|let|loop|match|mod|move|mut|pub|ref|return|self|Self|static|struct|super|trait|true|type|unsafe|use|where|while)\\b",
    },
    {
      kind: "type",
      pattern:
        "\\b(?:bool|char|f32|f64|i8|i16|i32|i64|i128|isize|str|u8|u16|u32|u64|u128|usize|String|Vec|Option|Result|Box|Arc|Rc|HashMap|HashSet|PathBuf|Path|[A-Z][A-Za-z0-9_]*)\\b",
    },
    {
      kind: "number",
      pattern:
        "\\b(?:0x[0-9a-fA-F_]+|0b[01_]+|0o[0-7_]+|\\d[\\d_]*(?:\\.\\d[\\d_]*)?(?:[uif](?:8|16|32|64|128|size))?)\\b",
    },
  ],
  ts: [
    { kind: "comment", pattern: "//[^\\n]*|/\\*[\\s\\S]*?\\*/" },
    { kind: "string", pattern: "`(?:\\\\.|[^`\\\\])*`|\"(?:\\\\.|[^\"\\\\])*\"|'(?:\\\\.|[^'\\\\])*'" },
    {
      kind: "keyword",
      pattern:
        "\\b(?:as|async|await|break|case|catch|class|const|continue|default|delete|do|else|enum|export|extends|false|finally|for|from|function|if|implements|import|in|instanceof|interface|keyof|let|new|null|of|readonly|return|satisfies|static|super|switch|this|throw|true|try|type|typeof|undefined|var|void|while|yield)\\b",
    },
    {
      kind: "type",
      pattern:
        "\\b(?:string|number|boolean|unknown|never|any|bigint|symbol|object|Promise|Record|Array|ReadonlyArray|Partial|Pick|Omit|Map|Set|[A-Z][A-Za-z0-9_]*)\\b",
    },
    { kind: "number", pattern: "\\b(?:0x[0-9a-fA-F_]+|\\d[\\d_]*(?:\\.\\d[\\d_]*)?n?)\\b" },
  ],
  lean: [
    { kind: "comment", pattern: "--[^\\n]*|/-[\\s\\S]*?-/" },
    { kind: "string", pattern: '"(?:\\\\.|[^"\\\\])*"' },
    {
      kind: "keyword",
      pattern:
        "\\b(?:abbrev|apply|axiom|by|calc|cases|class|decide|def|deriving|do|else|end|exact|example|exists|false|forall|fun|have|if|import|in|induction|inductive|instance|intro|intros|lemma|let|match|namespace|noncomputable|omega|open|private|protected|rfl|rw|section|set_option|show|simp|sorry|structure|then|theorem|true|variable|where|with)\\b",
    },
    {
      kind: "type",
      pattern: "\\b(?:Prop|Type|Sort|Nat|Int|Bool|String|Char|List|Array|Option|Fin|Unit|Empty|[A-Z][A-Za-z0-9_]*)\\b",
    },
    { kind: "number", pattern: "\\b\\d+(?:\\.\\d+)?\\b" },
  ],
  json: [
    { kind: "comment", pattern: "//[^\\n]*" },
    { kind: "property", pattern: '"(?:\\\\.|[^"\\\\])*"(?=\\s*:)' },
    { kind: "string", pattern: '"(?:\\\\.|[^"\\\\])*"' },
    { kind: "keyword", pattern: "\\b(?:true|false|null)\\b" },
    { kind: "number", pattern: "-?\\b\\d+(?:\\.\\d+)?(?:[eE][+-]?\\d+)?\\b" },
  ],
  bash: [
    { kind: "comment", pattern: "#[^\\n]*" },
    { kind: "string", pattern: '"(?:\\\\.|[^"\\\\])*"|\'[^\']*\'' },
    { kind: "property", pattern: "\\$\\{[^}]+\\}|\\$[A-Za-z_][A-Za-z0-9_]*|--?[A-Za-z][A-Za-z0-9_-]*" },
    {
      kind: "keyword",
      pattern:
        "\\b(?:case|cd|do|done|echo|elif|else|esac|exit|export|false|fi|for|function|if|in|local|return|set|then|true|unset|while)\\b",
    },
    { kind: "number", pattern: "\\b\\d+\\b" },
  ],
};

interface CompiledRule {
  readonly kind: TokenKind;
  readonly groupIndex: number;
}

interface CompiledLanguage {
  readonly regex: RegExp;
  readonly rules: readonly CompiledRule[];
}

function countCaptureGroups(pattern: string): number {
  const probe = new RegExp(`(?:${pattern})|`).exec("");
  return probe ? probe.length - 1 : 0;
}

function compileLanguage(rules: readonly TokenRule[]): CompiledLanguage {
  const compiledRules: CompiledRule[] = [];
  let nextGroupIndex = 1;
  for (const rule of rules) {
    compiledRules.push({ kind: rule.kind, groupIndex: nextGroupIndex });
    nextGroupIndex += 1 + countCaptureGroups(rule.pattern);
  }
  const regex = new RegExp(rules.map((rule) => `(${rule.pattern})`).join("|"), "g");
  return { regex, rules: compiledRules };
}

const COMPILED_LANGUAGES: Record<Exclude<SyntaxLang, "plain">, CompiledLanguage> = {
  rust: compileLanguage(LANGUAGE_RULES.rust),
  ts: compileLanguage(LANGUAGE_RULES.ts),
  lean: compileLanguage(LANGUAGE_RULES.lean),
  json: compileLanguage(LANGUAGE_RULES.json),
  bash: compileLanguage(LANGUAGE_RULES.bash),
};

function tokenizeCode(code: string, rawLang: string): readonly CodeToken[] {
  const lang = normalizeLang(rawLang);
  if (lang === "plain" || code.length === 0) {
    return [{ kind: "plain", text: code }];
  }

  const compiled = COMPILED_LANGUAGES[lang];
  const { regex, rules } = compiled;
  regex.lastIndex = 0;

  const tokens: CodeToken[] = [];
  let lastIndex = 0;
  let match = regex.exec(code);

  while (match !== null) {
    if (match.index > lastIndex) {
      tokens.push({ kind: "plain", text: code.slice(lastIndex, match.index) });
    }
    let matchedKind: TokenKind = "plain";
    for (let i = 0; i < rules.length; i += 1) {
      if (match[rules[i].groupIndex] !== undefined) {
        matchedKind = rules[i].kind;
        break;
      }
    }
    tokens.push({ kind: matchedKind, text: match[0] });
    lastIndex = match.index + match[0].length;
    if (match[0].length === 0) {
      regex.lastIndex += 1;
    }
    match = regex.exec(code);
  }

  regex.lastIndex = 0;

  if (lastIndex < code.length) {
    tokens.push({ kind: "plain", text: code.slice(lastIndex) });
  }
  return tokens;
}

// ============================================================
// Pure Markdown Block & Inline Renderer (Zero innerHTML)
// ============================================================

type TableAlign = "left" | "center" | "right";
type HeadingLevel = 1 | 2 | 3 | 4 | 5 | 6;

interface ListItemNode {
  readonly text: string;
  readonly children: readonly ListBlock[];
}

interface ListBlock {
  readonly kind: "list";
  readonly ordered: boolean;
  readonly start?: number;
  readonly items: readonly ListItemNode[];
}

type MarkdownBlock =
  | { readonly kind: "code"; readonly lang: string; readonly code: string }
  | { readonly kind: "heading"; readonly level: HeadingLevel; readonly text: string }
  | { readonly kind: "hr" }
  | {
      readonly kind: "table";
      readonly headers: readonly string[];
      readonly alignments: readonly TableAlign[];
      readonly rows: readonly (readonly string[])[];
    }
  | { readonly kind: "blockquote"; readonly lines: readonly string[] }
  | ListBlock
  | { readonly kind: "paragraph"; readonly text: string };

const FENCE_OPEN_RE = /^ {0,3}(`{3,}|~{3,})(.*)$/;
const HR_RE = /^\s*([-*_])(?:\s*\1){2,}\s*$/;
const HEADING_RE = /^ {0,3}(#{1,6})\s+(.*)$/;
const LIST_ITEM_RE = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;
const TABLE_SEP_RE = /^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)+\|?\s*$/;
const SINGLE_COL_SEP_RE = /^\s*\|\s*:?-+:?\s*\|\s*$/;

interface FenceOpen {
  readonly char: "`" | "~";
  readonly length: number;
  readonly lang: string;
}

function parseFenceOpen(line: string): FenceOpen | null {
  const match = FENCE_OPEN_RE.exec(line);
  if (!match) return null;
  const marker = match[1];
  const info = match[2];
  const char = marker[0] === "~" ? "~" : "`";
  if (char === "`" && info.includes("`")) {
    return null;
  }
  return { char, length: marker.length, lang: info.trim() };
}

function isFenceClose(line: string, fenceChar: "`" | "~", fenceLength: number): boolean {
  const withoutIndent = line.replace(/^ {0,3}/, "");
  let count = 0;
  while (count < withoutIndent.length && withoutIndent[count] === fenceChar) {
    count += 1;
  }
  if (count < fenceLength) return false;
  return /^[ \t]*$/.test(withoutIndent.slice(count));
}

function toHeadingLevel(count: number): HeadingLevel {
  switch (count) {
    case 1:
      return 1;
    case 2:
      return 2;
    case 3:
      return 3;
    case 4:
      return 4;
    case 5:
      return 5;
    default:
      return 6;
  }
}

function splitTableRow(line: string): readonly string[] {
  let trimmed = line.trim();
  if (trimmed.startsWith("|")) {
    trimmed = trimmed.slice(1);
  }
  if (trimmed.endsWith("|") && !trimmed.endsWith("\\|")) {
    trimmed = trimmed.slice(0, -1);
  }

  const cells: string[] = [];
  let current = "";
  let codeTicks = 0;

  for (let i = 0; i < trimmed.length; i += 1) {
    const ch = trimmed[i];

    if (ch === "`") {
      let run = 1;
      while (i + run < trimmed.length && trimmed[i + run] === "`") {
        run += 1;
      }
      if (codeTicks === 0) {
        codeTicks = run;
      } else if (codeTicks === run) {
        codeTicks = 0;
      }
      current += trimmed.slice(i, i + run);
      i += run - 1;
      continue;
    }

    if (codeTicks === 0 && ch === "\\" && trimmed[i + 1] === "|") {
      current += "|";
      i += 1;
      continue;
    }

    if (codeTicks === 0 && ch === "|") {
      cells.push(current.trim());
      current = "";
      continue;
    }

    current += ch;
  }

  cells.push(current.trim());
  return cells;
}

function isTableSeparator(line: string): boolean {
  return TABLE_SEP_RE.test(line) || SINGLE_COL_SEP_RE.test(line);
}

function parseTableAlignments(line: string): readonly TableAlign[] {
  return splitTableRow(line).map((cell) => {
    const starts = cell.startsWith(":");
    const ends = cell.endsWith(":");
    if (starts && ends) return "center";
    if (ends) return "right";
    return "left";
  });
}

function parseListBlock(
  lines: readonly string[],
  startIndex: number,
  baseIndent: number
): { readonly block: ListBlock; readonly nextIndex: number } {
  const firstMatch = LIST_ITEM_RE.exec(lines[startIndex]);
  const ordered = firstMatch ? /^\d+\.$/.test(firstMatch[2]) : false;
  const parsedStart = ordered && firstMatch ? Number.parseInt(firstMatch[2], 10) : 1;
  const start = ordered && Number.isFinite(parsedStart) && parsedStart !== 1 ? parsedStart : undefined;
  const items: ListItemNode[] = [];
  let i = startIndex;

  while (i < lines.length) {
    const match = LIST_ITEM_RE.exec(lines[i]);
    if (!match) break;

    const indent = match[1].replace(/\t/g, "  ").length;
    const itemOrdered = /^\d+\.$/.test(match[2]);

    if (indent < baseIndent) break;
    if (indent > baseIndent) {
      if (items.length === 0) break;
      const nested = parseListBlock(lines, i, indent);
      const last = items[items.length - 1];
      items[items.length - 1] = {
        text: last.text,
        children: [...last.children, nested.block],
      };
      i = nested.nextIndex;
      continue;
    }

    if (itemOrdered !== ordered) break;

    items.push({ text: match[3], children: [] });
    i += 1;
  }

  return {
    block: { kind: "list", ordered, start, items },
    nextIndex: i,
  };
}

function isBlockStart(lines: readonly string[], index: number): boolean {
  const line = lines[index];
  if (line.trim() === "") return true;
  if (parseFenceOpen(line) !== null) return true;
  if (HEADING_RE.test(line)) return true;
  if (HR_RE.test(line)) return true;
  if (line.startsWith(">")) return true;
  if (LIST_ITEM_RE.test(line)) return true;
  if (
    line.includes("|") &&
    index + 1 < lines.length &&
    isTableSeparator(lines[index + 1])
  ) {
    return true;
  }
  return false;
}

function parseMarkdownBlocks(source: string): readonly MarkdownBlock[] {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: MarkdownBlock[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i += 1;
      continue;
    }

    const fenceOpen = parseFenceOpen(line);
    if (fenceOpen !== null) {
      const codeLines: string[] = [];
      i += 1;
      while (i < lines.length && !isFenceClose(lines[i], fenceOpen.char, fenceOpen.length)) {
        codeLines.push(lines[i]);
        i += 1;
      }
      if (i < lines.length && isFenceClose(lines[i], fenceOpen.char, fenceOpen.length)) {
        i += 1;
      }
      blocks.push({ kind: "code", lang: fenceOpen.lang, code: codeLines.join("\n") });
      continue;
    }

    if (HR_RE.test(line)) {
      blocks.push({ kind: "hr" });
      i += 1;
      continue;
    }

    const headingMatch = HEADING_RE.exec(line);
    if (headingMatch) {
      const level = toHeadingLevel(headingMatch[1].length);
      blocks.push({ kind: "heading", level, text: headingMatch[2] });
      i += 1;
      continue;
    }

    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      isTableSeparator(lines[i + 1])
    ) {
      const headers = splitTableRow(line);
      const alignments = parseTableAlignments(lines[i + 1]);
      const rows: (readonly string[])[] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
        rows.push(splitTableRow(lines[i]));
        i += 1;
      }
      blocks.push({ kind: "table", headers, alignments, rows });
      continue;
    }

    if (line.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        quoteLines.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push({ kind: "blockquote", lines: quoteLines });
      continue;
    }

    const listMatch = LIST_ITEM_RE.exec(line);
    if (listMatch) {
      const indent = listMatch[1].replace(/\t/g, "  ").length;
      const parsed = parseListBlock(lines, i, indent);
      blocks.push(parsed.block);
      i = parsed.nextIndex;
      continue;
    }

    const paraLines: string[] = [];
    while (i < lines.length && !isBlockStart(lines, i)) {
      paraLines.push(lines[i]);
      i += 1;
    }
    if (paraLines.length > 0) {
      blocks.push({ kind: "paragraph", text: paraLines.join("\n") });
    } else {
      i += 1;
    }
  }

  return blocks;
}

interface ParsedInlineLink {
  readonly label: string;
  readonly url: string;
  readonly endIndex: number;
}

function tryParseInlineLink(text: string, start: number): ParsedInlineLink | null {
  if (text[start] !== "[") return null;
  let closeBracket = -1;
  for (let i = start + 1; i < text.length; i += 1) {
    if (text[i] === "\\" && i + 1 < text.length) {
      i += 1;
      continue;
    }
    if (text[i] === "\n") return null;
    if (text[i] === "]") {
      closeBracket = i;
      break;
    }
  }
  if (closeBracket === -1 || text[closeBracket + 1] !== "(") return null;

  const label = text.slice(start + 1, closeBracket);
  if (label.length === 0) return null;

  let depth = 1;
  let endParen = -1;
  for (let i = closeBracket + 2; i < text.length; i += 1) {
    const ch = text[i];
    if (/\s/.test(ch)) return null;
    if (ch === "(") {
      depth += 1;
    } else if (ch === ")") {
      depth -= 1;
      if (depth === 0) {
        endParen = i;
        break;
      }
    }
  }
  if (endParen === -1) return null;

  const url = text.slice(closeBracket + 2, endParen);
  if (!isAllowedExternalUrl(url)) return null;

  return { label, url, endIndex: endParen + 1 };
}

function findClosingDelimiter(
  text: string,
  from: number,
  delimiter: "**" | "~~" | "*"
): number {
  const len = delimiter.length;
  for (let i = from; i <= text.length - len; i += 1) {
    if (text.startsWith(delimiter, i)) {
      if (delimiter === "*" && (text[i + 1] === "*" || text[i - 1] === "*")) {
        continue;
      }
      if (i > from && !/\s/.test(text[i - 1])) {
        return i;
      }
    }
  }
  return -1;
}

function renderInlineMarkdown(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  let plainBuffer = "";
  let i = 0;

  const flushPlain = () => {
    if (plainBuffer.length > 0) {
      parts.push(plainBuffer);
      plainBuffer = "";
    }
  };

  while (i < text.length) {
    const ch = text[i];

    if (ch === "`") {
      let tickCount = 1;
      while (i + tickCount < text.length && text[i + tickCount] === "`") {
        tickCount += 1;
      }
      const closer = "`".repeat(tickCount);
      let closeIdx = text.indexOf(closer, i + tickCount);
      while (
        closeIdx !== -1 &&
        (text[closeIdx - 1] === "`" || text[closeIdx + tickCount] === "`")
      ) {
        closeIdx = text.indexOf(closer, closeIdx + 1);
      }
      if (closeIdx !== -1 && closeIdx > i + tickCount) {
        flushPlain();
        parts.push(<code key={`md-${i}`}>{text.slice(i + tickCount, closeIdx)}</code>);
        i = closeIdx + tickCount;
        continue;
      }
    }

    if (ch === "[") {
      const link = tryParseInlineLink(text, i);
      if (link !== null) {
        flushPlain();
        parts.push(
          <a
            key={`md-${i}`}
            href={link.url}
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => handleExternalLinkClick(event, link.url)}
          >
            {link.label}
          </a>
        );
        i = link.endIndex;
        continue;
      }
    }

    if (ch === "*" && text[i + 1] === "*" && i + 2 < text.length && !/\s/.test(text[i + 2])) {
      const closeIdx = findClosingDelimiter(text, i + 2, "**");
      if (closeIdx !== -1) {
        flushPlain();
        parts.push(<strong key={`md-${i}`}>{text.slice(i + 2, closeIdx)}</strong>);
        i = closeIdx + 2;
        continue;
      }
    }

    if (ch === "~" && text[i + 1] === "~" && i + 2 < text.length && !/\s/.test(text[i + 2])) {
      const closeIdx = findClosingDelimiter(text, i + 2, "~~");
      if (closeIdx !== -1) {
        flushPlain();
        parts.push(<del key={`md-${i}`}>{text.slice(i + 2, closeIdx)}</del>);
        i = closeIdx + 2;
        continue;
      }
    }

    if (
      ch === "*" &&
      text[i + 1] !== "*" &&
      i + 1 < text.length &&
      !/\s/.test(text[i + 1])
    ) {
      const closeIdx = findClosingDelimiter(text, i + 1, "*");
      if (closeIdx !== -1) {
        flushPlain();
        parts.push(<em key={`md-${i}`}>{text.slice(i + 1, closeIdx)}</em>);
        i = closeIdx + 1;
        continue;
      }
    }

    plainBuffer += ch;
    i += 1;
  }

  flushPlain();
  return parts;
}

function renderListNode(block: ListBlock, keyPrefix: string): ReactNode {
  const listItems = block.items.map((item, idx) => (
    <li key={`${keyPrefix}-${idx}`}>
      {renderInlineMarkdown(item.text)}
      {item.children.map((child, childIdx) =>
        renderListNode(child, `${keyPrefix}-${idx}-c${childIdx}`)
      )}
    </li>
  ));

  if (block.ordered) {
    return (
      <ol key={keyPrefix} start={block.start}>
        {listItems}
      </ol>
    );
  }
  return <ul key={keyPrefix}>{listItems}</ul>;
}

function renderHeadingNode(
  level: HeadingLevel,
  text: string,
  key: string
): ReactNode {
  const children = renderInlineMarkdown(text);
  switch (level) {
    case 1:
      return <h1 key={key}>{children}</h1>;
    case 2:
      return <h2 key={key}>{children}</h2>;
    case 3:
      return <h3 key={key}>{children}</h3>;
    case 4:
      return <h4 key={key}>{children}</h4>;
    case 5:
      return <h5 key={key}>{children}</h5>;
    case 6:
      return <h6 key={key}>{children}</h6>;
    default: {
      const _exhaustive: never = level;
      return _exhaustive;
    }
  }
}

type CopyState = "idle" | "copied" | "failed";

function copyWithExecCommand(text: string): boolean {
  if (typeof document === "undefined") return false;
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  textarea.style.pointerEvents = "none";
  document.body.appendChild(textarea);
  textarea.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    document.body.removeChild(textarea);
  }
  return ok;
}

const CodeBlock = memo(function CodeBlock({
  lang,
  code,
}: {
  readonly lang: string;
  readonly code: string;
}) {
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null && typeof window !== "undefined") {
        window.clearTimeout(timerRef.current);
      }
    };
  }, []);

  async function handleCopy(): Promise<void> {
    let succeeded = false;
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(code);
        succeeded = true;
      } catch {
        succeeded = copyWithExecCommand(code);
      }
    } else {
      succeeded = copyWithExecCommand(code);
    }

    setCopyState(succeeded ? "copied" : "failed");
    if (typeof window !== "undefined") {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      timerRef.current = window.setTimeout(() => {
        setCopyState("idle");
        timerRef.current = null;
      }, 1800);
    }
  }

  const tokens = useMemo(() => tokenizeCode(code, lang), [code, lang]);

  return (
    <div className="pi-code-wrap">
      <div className="pi-code-header">
        <span className="pi-code-lang">{lang || "text"}</span>
        <button
          type="button"
          className={`pi-code-copy-btn${
            copyState === "copied" ? " copied" : copyState === "failed" ? " failed" : ""
          }`}
          onClick={() => void handleCopy()}
          aria-label={
            copyState === "copied"
              ? "Copied code"
              : copyState === "failed"
                ? "Copy failed"
                : "Copy code"
          }
        >
          {copyState === "copied" ? (
            <CheckIcon size={13} />
          ) : copyState === "failed" ? (
            <AlertIcon size={13} />
          ) : (
            <CopyIcon size={13} />
          )}
          <span>
            {copyState === "copied"
              ? "Copied"
              : copyState === "failed"
                ? "Copy failed"
                : "Copy"}
          </span>
        </button>
      </div>
      <pre className="pi-code-block">
        <code>
          {tokens.map((tok, idx) =>
            tok.kind === "plain" ? (
              tok.text
            ) : (
              <span key={idx} className={`pi-tok-${tok.kind}`}>
                {tok.text}
              </span>
            )
          )}
        </code>
      </pre>
    </div>
  );
});

const MarkdownView = memo(function MarkdownView({
  content,
  streaming,
}: {
  readonly content: string;
  readonly streaming?: boolean;
}) {
  const blocks = useMemo(() => parseMarkdownBlocks(content), [content]);
  return (
    <div className="pi-md">
      {blocks.map((block, index) => {
        const key = `${block.kind}-${index}`;
        switch (block.kind) {
          case "code":
            return <CodeBlock key={key} lang={block.lang} code={block.code} />;
          case "heading":
            return renderHeadingNode(block.level, block.text, key);
          case "hr":
            return <hr key={key} className="pi-md-hr" />;
          case "table":
            return (
              <div key={key} className="pi-md-table-wrap">
                <table className="pi-md-table">
                  <thead>
                    <tr>
                      {block.headers.map((header, colIdx) => (
                        <th
                          key={colIdx}
                          style={{ textAlign: block.alignments[colIdx] ?? "left" }}
                        >
                          {renderInlineMarkdown(header)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {block.rows.map((row, rowIdx) => (
                      <tr key={rowIdx}>
                        {block.headers.map((_, colIdx) => (
                          <td
                            key={colIdx}
                            style={{ textAlign: block.alignments[colIdx] ?? "left" }}
                          >
                            {renderInlineMarkdown(row[colIdx] ?? "")}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          case "blockquote":
            return (
              <blockquote key={key}>
                {block.lines.map((line, lineIdx) => (
                  <p key={lineIdx}>{renderInlineMarkdown(line)}</p>
                ))}
              </blockquote>
            );
          case "list":
            return renderListNode(block, key);
          case "paragraph":
            return <p key={key}>{renderInlineMarkdown(block.text)}</p>;
          default: {
            const _exhaustive: never = block;
            return _exhaustive;
          }
        }
      })}
      {streaming && <span className="pi-streaming-caret" aria-label="Streaming" />}
    </div>
  );
});

// ============================================================
// Expandable Tool-Call Pills
// ============================================================

function formatToolDetail(detail: string): string {
  const trimmed = detail.trim();
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      return JSON.stringify(parsed, null, 2);
    } catch {
      return detail;
    }
  }
  return detail;
}

function toolStatusLabel(status: PiToolCallState["status"]): string {
  switch (status) {
    case "running":
      return "Running";
    case "done":
      return "Done";
    case "error":
      return "Failed";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

function ToolCallCard({ tool }: { readonly tool: PiToolCallState }) {
  const [expanded, setExpanded] = useState(false);
  const singleLinePreview = tool.detail ? tool.detail.replace(/\s+/g, " ").trim() : "";

  return (
    <div className="pi-tool-card">
      <button
        type="button"
        className="pi-tool-card-header"
        aria-expanded={expanded}
        onClick={() => setExpanded((prev) => !prev)}
      >
        <span className={`pi-tool-status-dot ${tool.status}`} aria-hidden="true" />
        <span className="pi-tool-name">{tool.name}</span>
        <span className="pi-tool-preview">{singleLinePreview}</span>
        <span className={`pi-tool-status-label ${tool.status}`}>
          {toolStatusLabel(tool.status)}
        </span>
        <span className={`pi-tool-chevron${expanded ? " expanded" : ""}`}>
          <ChevronRightIcon size={14} />
        </span>
      </button>

      {expanded && (
        <div className="pi-tool-card-body">
          <div className="pi-tool-card-meta">
            <span>id: {tool.id}</span>
            <span>status: {tool.status}</span>
          </div>
          {tool.detail ? (
            <pre className="pi-tool-card-pre">{formatToolDetail(tool.detail)}</pre>
          ) : (
            <div className="pi-tool-card-meta">No additional output recorded.</div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolCallList({ tools }: { readonly tools: readonly PiToolCallState[] }) {
  if (tools.length === 0) return null;
  return (
    <div className="pi-tool-list" aria-label="Tool executions">
      {tools.map((tool) => (
        <ToolCallCard key={tool.id} tool={tool} />
      ))}
    </div>
  );
}

// ============================================================
// Quiet Starter Prompt Lines (shared fallback)
// ============================================================

export interface StarterSuggestion {
  readonly id: string;
  readonly label: string;
  readonly prompt: string;
  readonly icon: ReactNode;
}

export const STARTER_SUGGESTIONS: readonly StarterSuggestion[] = [
  {
    id: "kani-verify",
    label: "Verify a Rust module with Kani",
    prompt:
      "Verify a Rust module with Kani: inspect check_scope bounds in crates/verity-core/src/scope.rs and prove write-scope invariants.",
    icon: <ShieldCheckIcon size={15} />,
  },
  {
    id: "scope-diff",
    label: "Review this diff for scope violations",
    prompt:
      "Review this diff for scope violations and audit bubblewrap read-only .git bind mounts in sandbox.rs.",
    icon: <GitBranchIcon size={15} />,
  },
  {
    id: "state-invariant",
    label: "Trace a state-machine invariant",
    prompt:
      "Trace a state-machine invariant across Pi session exclusivity transitions and deferred provider reloads in crates/verity-core.",
    icon: <TerminalIcon size={15} />,
  },
];

// ============================================================
// Thinking Indicator (3-Dot Constellation) & Thinking Panel
// ============================================================

function ThinkingConstellation({ animated }: { readonly animated: boolean }) {
  return (
    <span
      className={`pi-constellation${animated ? " pi-constellation--animated" : ""}`}
      aria-hidden="true"
    >
      <span className="pi-constellation-dot pi-constellation-dot--1" />
      <span className="pi-constellation-dot pi-constellation-dot--2" />
      <span className="pi-constellation-dot pi-constellation-dot--3" />
    </span>
  );
}

function ThinkingIndicator() {
  return (
    <div className="pi-thinking-indicator" role="status" aria-live="polite">
      <ThinkingConstellation animated />
      <span className="pi-thinking-label">Thinking...</span>
    </div>
  );
}

function ThinkingPanel({
  thinking,
  streaming,
  hasContent,
}: {
  readonly thinking: string;
  readonly streaming: boolean;
  readonly hasContent: boolean;
}) {
  const [expanded, setExpanded] = useState(() => streaming);
  const isThinkingActive = streaming && !hasContent;

  return (
    <div
      className={`pi-thinking-panel${isThinkingActive ? " pi-thinking-panel--streaming" : ""}`}
    >
      <button
        type="button"
        className="pi-thinking-header"
        aria-expanded={expanded}
        onClick={() => setExpanded((prev) => !prev)}
      >
        <span className="pi-thinking-header-left">
          <ThinkingConstellation animated={isThinkingActive} />
          <span className="pi-thinking-label">
            {isThinkingActive ? "Thinking..." : "Thinking"}
          </span>
        </span>
        <span className={`pi-thinking-chevron${expanded ? " expanded" : ""}`}>
          <ChevronRightIcon size={14} />
        </span>
      </button>

      {expanded && (
        <div className="pi-thinking-body">
          <pre className="pi-thinking-content">
            {thinking}
            {isThinkingActive && (
              <span className="pi-streaming-caret" aria-hidden="true" />
            )}
          </pre>
        </div>
      )}
    </div>
  );
}

// ============================================================
// ChatTranscript
// ============================================================

export function ChatTranscript({
  messages,
  isStreaming,
  liveTools,
  retryBanner,
  onAbortRetry,
  onPickStarterPrompt,
}: ChatTranscriptProps) {
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const isPinnedToBottomRef = useRef(true);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const lastMessage = messages.length > 0 ? messages[messages.length - 1] : null;
  const lastContentLength = lastMessage?.content.length ?? 0;
  const lastThinkingLength = lastMessage?.thinking?.length ?? 0;
  const hasStreamingAssistantMessage = messages.some(
    (m) => m.role === "assistant" && Boolean(m.streaming)
  );

  const visibleLiveTools = useMemo(() => {
    if (liveTools.length === 0) return liveTools;
    const renderedToolIds = new Set<string>();
    for (const msg of messages) {
      if (msg.toolCalls) {
        for (const tool of msg.toolCalls) {
          renderedToolIds.add(tool.id);
        }
      }
    }
    return liveTools.filter((tool) => !renderedToolIds.has(tool.id));
  }, [messages, liveTools]);

  useEffect(() => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const handleScroll = () => {
      isPinnedToBottomRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    };
    el.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      el.removeEventListener("scroll", handleScroll);
    };
  }, []);

  useEffect(() => {
    if (!isPinnedToBottomRef.current) return;
    bottomRef.current?.scrollIntoView({
      behavior: isStreaming ? "auto" : "smooth",
      block: "end",
    });
  }, [
    messages.length,
    isStreaming,
    visibleLiveTools.length,
    lastContentLength,
    lastThinkingLength,
  ]);

  return (
    <div className="pi-transcript-shell">
      <div
        ref={scrollContainerRef}
        className="pi-transcript-scroll"
        role="log"
        aria-live="polite"
      >
        <div className="pi-transcript-inner">
          {messages.length === 0 && !isStreaming ? (
            <div className="pi-empty-hero">
              <h2 className="pi-empty-title">Where should we start?</h2>
              {onPickStarterPrompt && (
                <div className="verity-hero-suggestions">
                  {STARTER_SUGGESTIONS.map((item) => (
                    <button
                      key={item.id}
                      type="button"
                      className="verity-suggestion-line"
                      onClick={() => onPickStarterPrompt(item.prompt)}
                    >
                      <span className="verity-suggestion-icon" aria-hidden="true">
                        {item.icon}
                      </span>
                      <span>{item.label}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            <>
              {messages.map((message) => {
                const hasThinking = Boolean(
                  message.thinking && message.thinking.trim().length > 0
                );
                const hasContent = message.content.trim().length > 0;
                const errorText = message.errorMessage?.trim() ?? "";
                const isErrorTurn =
                  message.stopReason === "error" || errorText.length > 0;
                const isAssistantStreaming =
                  message.role === "assistant" &&
                  Boolean(message.streaming) &&
                  !isErrorTurn;

                return (
                  <article key={message.id} className={`pi-message-row ${message.role}`}>
                    <div className="pi-message-header">
                      {message.role === "assistant" ? (
                        <span className="pi-role-avatar assistant" aria-hidden="true">
                          <VerityLogoMark size={13} strokeWidth={2.1} />
                        </span>
                      ) : message.role === "user" ? (
                        <span className="pi-role-avatar user" aria-hidden="true">
                          U
                        </span>
                      ) : (
                        <span className="pi-role-avatar system" aria-hidden="true">
                          <InfoIcon size={12} />
                        </span>
                      )}
                      <span className="pi-message-role">
                        {message.role === "user"
                          ? "You"
                          : message.role === "assistant"
                            ? "Verity"
                            : "System"}
                      </span>
                      <span className="pi-message-time">{message.timestamp}</span>
                    </div>

                    <div className={`pi-message-bubble ${message.role}`}>
                      {message.attachments && message.attachments.length > 0 && (
                        <div className="pi-message-attachments">
                          {message.attachments.map((att, idx) => (
                            <span
                              key={`${att.filename}-${idx}`}
                              className="pi-attachment-thumb"
                            >
                              {isAllowedRasterMime(att.mime_type) && (
                                <img
                                  src={`data:${att.mime_type.trim().toLowerCase()};base64,${att.base64_data}`}
                                  alt={att.filename}
                                />
                              )}
                              <span>{att.filename}</span>
                            </span>
                          ))}
                        </div>
                      )}

                      {message.role === "assistant" ? (
                        <>
                          {hasThinking ? (
                            <ThinkingPanel
                              thinking={message.thinking ?? ""}
                              streaming={isAssistantStreaming}
                              hasContent={hasContent}
                            />
                          ) : (
                            isAssistantStreaming && !hasContent && <ThinkingIndicator />
                          )}

                          {hasContent && (
                            <MarkdownView
                              content={message.content}
                              streaming={isAssistantStreaming}
                            />
                          )}

                          {isErrorTurn && (
                            <div className="pi-turn-error-callout" role="alert">
                              <span className="pi-turn-error-icon" aria-hidden="true">
                                <AlertIcon size={15} />
                              </span>
                              <span className="pi-turn-error-text">
                                {errorText || "The response stopped due to an error."}
                              </span>
                            </div>
                          )}
                        </>
                      ) : (
                        <MarkdownView
                          content={message.content}
                          streaming={message.streaming}
                        />
                      )}
                    </div>

                    {message.toolCalls && message.toolCalls.length > 0 && (
                      <ToolCallList tools={message.toolCalls} />
                    )}
                  </article>
                );
              })}

              {isStreaming && !hasStreamingAssistantMessage && (
                <article
                  className="pi-message-row assistant pi-message-row--pending"
                  aria-busy="true"
                >
                  <div className="pi-message-header">
                    <span className="pi-role-avatar assistant" aria-hidden="true">
                      <VerityLogoMark size={13} strokeWidth={2.1} />
                    </span>
                    <span className="pi-message-role">Verity</span>
                  </div>
                  <div className="pi-message-bubble assistant">
                    <ThinkingIndicator />
                  </div>
                </article>
              )}
            </>
          )}

          {visibleLiveTools.length > 0 && (
            <div className="pi-message-row assistant">
              <ToolCallList tools={visibleLiveTools} />
            </div>
          )}

          <div ref={bottomRef} />
        </div>
      </div>

      {retryBanner && (
        <div className="pi-retry-banner" role="status">
          <span className="pi-retry-banner-text">
            <AlertIcon size={15} />
            <span>
              Auto-retrying request (attempt {retryBanner.attempt} of {retryBanner.maxAttempts},{" "}
              {Math.ceil(retryBanner.delayMs / 1000)}s backoff)...
            </span>
          </span>
          <button type="button" className="md3-btn-outlined" onClick={onAbortRetry}>
            Abort retry
          </button>
        </div>
      )}
    </div>
  );
}
