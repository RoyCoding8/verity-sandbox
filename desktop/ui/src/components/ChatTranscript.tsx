import { useEffect, useRef, useState, type ReactNode } from "react";
import type { PiChatMessage, PiToolCallState } from "../api/tauri";
import type { RetryBannerState } from "../state/piStore";

export interface ChatTranscriptProps {
  readonly messages: readonly PiChatMessage[];
  readonly isStreaming: boolean;
  readonly liveTools: readonly PiToolCallState[];
  readonly retryBanner: RetryBannerState | null;
  readonly onAbortRetry: () => void;
  readonly onPickStarterPrompt?: (prompt: string) => void;
}

// ============================================================
// Pure Markdown Block & Inline Renderer (Zero innerHTML)
// ============================================================

type MarkdownBlock =
  | { readonly kind: "code"; readonly lang: string; readonly code: string }
  | { readonly kind: "heading"; readonly level: 1 | 2 | 3; readonly text: string }
  | { readonly kind: "blockquote"; readonly text: string }
  | { readonly kind: "ul"; readonly items: readonly string[] }
  | { readonly kind: "ol"; readonly items: readonly string[] }
  | { readonly kind: "paragraph"; readonly text: string };

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

    const headingMatch = /^(#{1,3})\s+(.*)$/.exec(line);
    if (headingMatch) {
      const level = headingMatch[1].length === 1 ? 1 : headingMatch[1].length === 2 ? 2 : 3;
      blocks.push({ kind: "heading", level, text: headingMatch[2] });
      i += 1;
      continue;
    }

    if (line.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        quoteLines.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      blocks.push({ kind: "blockquote", text: quoteLines.join("\n") });
      continue;
    }

    if (/^\s*[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*[-*]\s+/, ""));
        i += 1;
      }
      blocks.push({ kind: "ul", items });
      continue;
    }

    if (/^\s*\d+\.\s+/.test(line)) {
      const items: string[] = [];
      while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
        items.push(lines[i].replace(/^\s*\d+\.\s+/, ""));
        i += 1;
      }
      blocks.push({ kind: "ol", items });
      continue;
    }

    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !/^(#{1,3})\s+/.test(lines[i]) &&
      !lines[i].startsWith(">") &&
      !/^\s*[-*]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i])
    ) {
      paraLines.push(lines[i]);
      i += 1;
    }
    blocks.push({ kind: "paragraph", text: paraLines.join("\n") });
  }

  return blocks;
}

function renderInlineMarkdown(text: string): ReactNode[] {
  const tokenRegex = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|\[[^\]]+\]\(https?:\/\/[^)]+\))/g;
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

function CodeBlock({ lang, code }: { readonly lang: string; readonly code: string }) {
  const [copied, setCopied] = useState(false);

  function handleCopy() {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      void navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    }
  }

  return (
    <div>
      <div className="pi-code-header">
        <span>{lang || "code"}</span>
        <button
          type="button"
          className="md3-btn-text"
          style={{ padding: "2px 8px", fontSize: "11px" }}
          onClick={handleCopy}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="pi-code-block">{code}</pre>
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
            if (block.level === 1) return <h1 key={key}>{renderInlineMarkdown(block.text)}</h1>;
            if (block.level === 2) return <h2 key={key}>{renderInlineMarkdown(block.text)}</h2>;
            return <h3 key={key}>{renderInlineMarkdown(block.text)}</h3>;
          case "blockquote":
            return <blockquote key={key}>{renderInlineMarkdown(block.text)}</blockquote>;
          case "ul":
            return (
              <ul key={key}>
                {block.items.map((item, i) => (
                  <li key={i}>{renderInlineMarkdown(item)}</li>
                ))}
              </ul>
            );
          case "ol":
            return (
              <ol key={key}>
                {block.items.map((item, i) => (
                  <li key={i}>{renderInlineMarkdown(item)}</li>
                ))}
              </ol>
            );
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

function ToolPills({ tools }: { readonly tools: readonly PiToolCallState[] }) {
  if (tools.length === 0) return null;
  return (
    <div className="pi-tool-pills" aria-label="Tool executions">
      {tools.map((tool) => (
        <span key={tool.id} className={`pi-tool-pill ${tool.status}`}>
          <span className="pi-tool-dot" aria-hidden="true" />
          <span>{tool.name}</span>
          {tool.detail && <span style={{ opacity: 0.78 }}>{tool.detail}</span>}
        </span>
      ))}
    </div>
  );
}

const STARTER_PROMPTS = [
  "Inspect check_scope bounds in verity-core",
  "/ext-demo",
  "/retry-demo",
  "$kani-verify crates/verity-core/src/scope.rs",
] as const;

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
    <>
      <div className="pi-transcript-scroll" role="log" aria-live="polite">
        <div className="pi-transcript-inner">
          {messages.length === 0 ? (
            <div className="pi-empty-state">
              <h2>Start a Pi session</h2>
              <p>
                Ask a coding question, run slash commands with <code>/</code>, or invoke
                workspace skills with <code>$</code>.
              </p>
              {onPickStarterPrompt && (
                <div className="pi-empty-prompts">
                  {STARTER_PROMPTS.map((prompt) => (
                    <button
                      key={prompt}
                      type="button"
                      className="md3-btn-tonal"
                      onClick={() => onPickStarterPrompt(prompt)}
                    >
                      {prompt}
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            messages.map((message) => (
              <article key={message.id} className={`pi-message-row ${message.role}`}>
                <div className="pi-message-header">
                  <span className="pi-message-role">
                    {message.role === "user"
                      ? "You"
                      : message.role === "assistant"
                        ? "Pi"
                        : "System"}
                  </span>
                  <span>{message.timestamp}</span>
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
                  <ToolPills tools={message.toolCalls} />
                )}
              </article>
            ))
          )}

          {liveTools.length > 0 && (
            <div className="pi-message-row assistant">
              <ToolPills tools={liveTools} />
            </div>
          )}

          <div ref={bottomRef} />
        </div>
      </div>

      {retryBanner && (
        <div className="pi-retry-banner" role="status">
          <span>
            Auto-retrying request (attempt {retryBanner.attempt} of {retryBanner.maxAttempts},{" "}
            {Math.ceil(retryBanner.delayMs / 1000)}s backoff)...
          </span>
          <button type="button" className="md3-btn-outlined" onClick={onAbortRetry}>
            Abort retry
          </button>
        </div>
      )}
    </>
  );
}
