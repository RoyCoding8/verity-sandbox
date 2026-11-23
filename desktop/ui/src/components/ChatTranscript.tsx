import { useEffect, useRef, useState, type ReactNode } from "react";
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

export interface ChatTranscriptProps {
  readonly messages: readonly PiChatMessage[];
  readonly isStreaming: boolean;
  readonly liveTools: readonly PiToolCallState[];
  readonly retryBanner: RetryBannerState | null;
  readonly onAbortRetry: () => void;
  readonly onPickStarterPrompt?: (prompt: string) => void;
}

// ============================================================
// Lightweight Syntax Highlighter (Pure TS, Zero Dependencies)
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
    { kind: "string", pattern: 'r#*"[\\s\\S]*?"#*|"(?:\\\\.|[^"\\\\])*"|\'(?:\\\\.|[^\'\\\\])\'' },
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
    { kind: "number", pattern: "\\b(?:0x[0-9a-fA-F_]+|0b[01_]+|0o[0-7_]+|\\d[\\d_]*(?:\\.\\d[\\d_]*)?(?:[uif](?:8|16|32|64|128|size))?)\\b" },
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

function tokenizeCode(code: string, rawLang: string): readonly CodeToken[] {
  const lang = normalizeLang(rawLang);
  if (lang === "plain" || code.length === 0) {
    return [{ kind: "plain", text: code }];
  }

  const rules = LANGUAGE_RULES[lang];
  const combined = new RegExp(
    rules.map((rule) => `(${rule.pattern})`).join("|"),
    "g"
  );

  const tokens: CodeToken[] = [];
  let lastIndex = 0;
  let match = combined.exec(code);

  while (match !== null) {
    if (match.index > lastIndex) {
      tokens.push({ kind: "plain", text: code.slice(lastIndex, match.index) });
    }
    let matchedKind: TokenKind = "plain";
    for (let i = 0; i < rules.length; i += 1) {
      if (match[i + 1] !== undefined) {
        matchedKind = rules[i].kind;
        break;
      }
    }
    tokens.push({ kind: matchedKind, text: match[0] });
    lastIndex = match.index + match[0].length;
    if (match[0].length === 0) {
      combined.lastIndex += 1;
    }
    match = combined.exec(code);
  }

  if (lastIndex < code.length) {
    tokens.push({ kind: "plain", text: code.slice(lastIndex) });
  }
  return tokens;
}

// ============================================================
// Pure Markdown Block & Inline Renderer (Zero innerHTML)
// ============================================================

type TableAlign = "left" | "center" | "right";

interface ListItemNode {
  readonly text: string;
  readonly children: readonly ListBlock[];
}

interface ListBlock {
  readonly kind: "list";
  readonly ordered: boolean;
  readonly items: readonly ListItemNode[];
}

type MarkdownBlock =
  | { readonly kind: "code"; readonly lang: string; readonly code: string }
  | { readonly kind: "heading"; readonly level: 1 | 2 | 3; readonly text: string }
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

const HR_RE = /^\s*([-*_])(?:\s*\1){2,}\s*$/;
const HEADING_RE = /^(#{1,3})\s+(.*)$/;
const LIST_ITEM_RE = /^(\s*)([-*+]|\d+\.)\s+(.*)$/;
const TABLE_SEP_RE = /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)+\|?\s*$/;
const SINGLE_COL_SEP_RE = /^\s*\|\s*:?-{3,}:?\s*\|\s*$/;

function splitTableRow(line: string): readonly string[] {
  const trimmed = line.trim();
  const stripped = trimmed.replace(/^\|/, "").replace(/\|$/, "");
  return stripped.split("|").map((cell) => cell.trim());
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
    block: { kind: "list", ordered, items },
    nextIndex: i,
  };
}

function isBlockStart(lines: readonly string[], index: number): boolean {
  const line = lines[index];
  if (line.trim() === "") return true;
  if (line.startsWith("```")) return true;
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

    if (line.startsWith("```")) {
      const lang = line.slice(3).trim();
      const codeLines: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i].startsWith("```")) {
        codeLines.push(lines[i]);
        i += 1;
      }
      if (i < lines.length && lines[i].startsWith("```")) {
        i += 1;
      }
      blocks.push({ kind: "code", lang, code: codeLines.join("\n") });
      continue;
    }

    if (HR_RE.test(line)) {
      blocks.push({ kind: "hr" });
      i += 1;
      continue;
    }

    const headingMatch = HEADING_RE.exec(line);
    if (headingMatch) {
      const level =
        headingMatch[1].length === 1 ? 1 : headingMatch[1].length === 2 ? 2 : 3;
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

function renderInlineMarkdown(text: string): ReactNode[] {
  const tokenRegex =
    /(`[^`]+`|\*\*[^*]+\*\*|~~[^~]+~~|\*[^*]+\*|\[[^\]]+\]\(https?:\/\/[^)]+\))/g;
  const parts: ReactNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null = tokenRegex.exec(text);

  while (match !== null) {
    if (match.index > lastIndex) {
      parts.push(text.slice(lastIndex, match.index));
    }
    const token = match[0];
    const key = `md-${match.index}`;
    if (token.startsWith("`")) {
      parts.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith("**")) {
      parts.push(<strong key={key}>{token.slice(2, -2)}</strong>);
    } else if (token.startsWith("~~")) {
      parts.push(<del key={key}>{token.slice(2, -2)}</del>);
    } else if (token.startsWith("*")) {
      parts.push(<em key={key}>{token.slice(1, -1)}</em>);
    } else {
      const linkMatch = /^\[([^\]]+)\]\((https?:\/\/[^)]+)\)$/.exec(token);
      if (linkMatch) {
        parts.push(
          <a key={key} href={linkMatch[2]} target="_blank" rel="noreferrer">
            {linkMatch[1]}
          </a>
        );
      } else {
        parts.push(token);
      }
    }
    lastIndex = match.index + token.length;
    match = tokenRegex.exec(text);
  }

  if (lastIndex < text.length) {
    parts.push(text.slice(lastIndex));
  }
  return parts;
}

function renderListNode(block: ListBlock, keyPrefix: string): ReactNode {
  const Tag = block.ordered ? "ol" : "ul";
  return (
    <Tag key={keyPrefix}>
      {block.items.map((item, idx) => (
        <li key={`${keyPrefix}-${idx}`}>
          {renderInlineMarkdown(item.text)}
          {item.children.map((child, childIdx) =>
            renderListNode(child, `${keyPrefix}-${idx}-c${childIdx}`)
          )}
        </li>
      ))}
    </Tag>
  );
}

function CodeBlock({ lang, code }: { readonly lang: string; readonly code: string }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
    };
  }, []);

  function handleCopy() {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(code);
      setCopied(true);
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      timerRef.current = window.setTimeout(() => {
        setCopied(false);
        timerRef.current = null;
      }, 1500);
    }
  }

  const tokens = tokenizeCode(code, lang);

  return (
    <div className="pi-code-wrap">
      <div className="pi-code-header">
        <span className="pi-code-lang">{lang || "text"}</span>
        <button
          type="button"
          className={`pi-code-copy-btn${copied ? " copied" : ""}`}
          onClick={handleCopy}
          aria-label={copied ? "Copied code" : "Copy code"}
        >
          {copied ? <CheckIcon size={13} /> : <CopyIcon size={13} />}
          <span>{copied ? "Copied" : "Copy"}</span>
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
}

function MarkdownView({
  content,
  streaming,
}: {
  readonly content: string;
  readonly streaming?: boolean;
}) {
  const blocks = parseMarkdownBlocks(content);
  return (
    <div className="pi-md">
      {blocks.map((block, index) => {
        const key = `${block.kind}-${index}`;
        switch (block.kind) {
          case "code":
            return <CodeBlock key={key} lang={block.lang} code={block.code} />;
          case "heading":
            if (block.level === 1) {
              return <h1 key={key}>{renderInlineMarkdown(block.text)}</h1>;
            }
            if (block.level === 2) {
              return <h2 key={key}>{renderInlineMarkdown(block.text)}</h2>;
            }
            return <h3 key={key}>{renderInlineMarkdown(block.text)}</h3>;
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
}

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
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages.length, isStreaming, liveTools.length]);

  return (
    <div className="pi-transcript-shell">
      <div className="pi-transcript-scroll" role="log" aria-live="polite">
        <div className="pi-transcript-inner">
          {messages.length === 0 ? (
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
            messages.map((message) => (
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
                      {message.attachments.map((att) => (
                        <span key={att.filename} className="pi-attachment-thumb">
                          <img
                            src={`data:${att.mime_type};base64,${att.base64_data}`}
                            alt={att.filename}
                          />
                          <span>{att.filename}</span>
                        </span>
                      ))}
                    </div>
                  )}
                  <MarkdownView content={message.content} streaming={message.streaming} />
                </div>

                {message.toolCalls && message.toolCalls.length > 0 && (
                  <ToolCallList tools={message.toolCalls} />
                )}
              </article>
            ))
          )}

          {liveTools.length > 0 && (
            <div className="pi-message-row assistant">
              <ToolCallList tools={liveTools} />
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
