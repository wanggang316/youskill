import { Marked } from "marked";
import { markedHighlight } from "marked-highlight";
import markedCjkFriendly from "marked-cjk-friendly";
import type { Token, Tokens } from "marked";
import hljs from "highlight.js/lib/common";
import matter from "gray-matter";

const CODE_BLOCK_LANG_PATTERN = /\s+/;

const LANGUAGE_LABEL_MAP: Record<string, string> = {
  bash: "Bash",
  shell: "Shell",
  sh: "Shell",
  c: "C",
  cpp: "C++",
  cs: "C#",
  css: "CSS",
  docker: "Docker",
  go: "Go",
  golang: "Go",
  html: "HTML",
  java: "Java",
  javascript: "JavaScript",
  js: "JavaScript",
  json: "JSON",
  jsx: "JSX",
  kotlin: "Kotlin",
  lua: "Lua",
  markdown: "Markdown",
  md: "Markdown",
  php: "PHP",
  plaintext: "Text",
  python: "Python",
  py: "Python",
  ruby: "Ruby",
  rust: "Rust",
  sql: "SQL",
  swift: "Swift",
  text: "Text",
  toml: "TOML",
  ts: "TypeScript",
  tsx: "TSX",
  typescript: "TypeScript",
  vue: "Vue",
  xml: "XML",
  yaml: "YAML",
  yml: "YAML",
};

const COPY_BUTTON_ICON = `<svg class="code-icon" xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path></svg>`;
const COPY_BUTTON_LABEL = "Copied";

function escapeHtml(value: string): string {
  return value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

function resolveLanguage(language?: string | null): string | undefined {
  if (!language) return undefined;
  const normalized = language.trim().toLowerCase();
  if (!normalized) return undefined;
  const [candidate] = normalized.split(CODE_BLOCK_LANG_PATTERN);
  return candidate || undefined;
}

function formatLanguageLabel(language?: string): string {
  if (!language) return "Text";
  const mapped = LANGUAGE_LABEL_MAP[language];
  if (mapped) return mapped;
  return (
    language
      .split(/[-_]/)
      .filter(Boolean)
      .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
      .join(" ") || language.toUpperCase()
  );
}

const markedRenderer = new Marked(
  markedHighlight({
    langPrefix: "hljs language-",
    highlight(code, language) {
      const resolvedLanguage = resolveLanguage(language);
      // A fence without a language is labeled "Text"; auto detection would color it anyway.
      if (!resolvedLanguage) return escapeHtml(code);

      if (hljs.getLanguage(resolvedLanguage)) {
        try {
          return hljs.highlight(code, { language: resolvedLanguage }).value;
        } catch (error) {
          console.warn("highlight.js failed", {
            language: resolvedLanguage,
            error,
          });
        }
      }

      try {
        return hljs.highlightAuto(code).value;
      } catch (error) {
        console.warn("highlight.js auto detection failed", { error });
        return escapeHtml(code);
      }
    },
  })
);

// CommonMark does not close `**` next to CJK punctuation (e.g. `**“foo”**bar`); translations hit this often.
markedRenderer.use(markedCjkFriendly());

markedRenderer.use({
  renderer: {
    code(token: Tokens.Code) {
      const language = resolveLanguage(token.lang ?? undefined);
      const languageLabel = formatLanguageLabel(language);
      const escapedLabel = escapeHtml(languageLabel);
      const languageAttr = language ? ` data-language="${escapeHtml(language)}"` : "";
      const classNames = ["hljs"];
      if (language) {
        classNames.push(`language-${language}`);
      }

      const codeHtml = token.escaped ? token.text : escapeHtml(token.text);

      return `<figure class="markdown-code-block"${languageAttr}>
  <header class="markdown-code-block__header">
    <span class="markdown-code-block__language">${escapedLabel}</span>
    <button type="button" class="markdown-code-block__copy" aria-label="Copy code">
      <span class="markdown-code-block__copy-icon">${COPY_BUTTON_ICON}</span>
      <span class="markdown-code-block__copy-label">${COPY_BUTTON_LABEL}</span>
    </button>
  </header>
  <pre><code class="${classNames.join(" ")}">${codeHtml}\n</code></pre>
</figure>`;
    },
    image({ href, title, text }: Tokens.Image) {
      const src = href ?? "";
      const alt = escapeHtml(text ?? "");
      const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
      return `<img src="${escapeHtml(src)}" alt="${alt}"${titleAttr} />`;
    },
  },
});

markedRenderer.options({ async: false });

export function renderMarkdown(content: string | undefined | null): string {
  if (!content) return "";

  const result = markedRenderer.parse(content);
  return typeof result === "string" ? result : "";
}

function withoutEmptySections(tokens: Token[]): Token[] {
  const result: Token[] = [];
  let index = 0;
  while (index < tokens.length) {
    const token = tokens[index];
    if (token.type !== "heading") {
      result.push(token);
      index += 1;
      continue;
    }
    const depth = (token as Tokens.Heading).depth;
    let end = index + 1;
    while (
      end < tokens.length &&
      !(tokens[end].type === "heading" && (tokens[end] as Tokens.Heading).depth <= depth)
    ) {
      end += 1;
    }
    const body = withoutEmptySections(tokens.slice(index + 1, end));
    if (body.some((child) => child.type !== "space")) result.push(token, ...body);
    index = end;
  }
  return result;
}

/** Removes headings that have no content below them, e.g. empty changelog categories. */
export function removeEmptySections(content: string | undefined | null): string {
  if (!content) return "";
  return withoutEmptySections(markedRenderer.lexer(content))
    .map((token) => token.raw)
    .join("")
    .trim();
}

// Blocks that the translation keeps as-is, so bilingual view shows them once.
const UNTRANSLATED_BLOCK_TYPES = new Set(["code", "hr", "html", "def"]);

function lexBlocks(content: string): Token[] {
  const tokens = markedRenderer.lexer(content);
  const walkTokens = markedRenderer.defaults.walkTokens;
  // `parse()` runs walkTokens (syntax highlight) itself; `lexer()` + `parser()` do not.
  if (walkTokens) markedRenderer.walkTokens(tokens, walkTokens);
  return tokens.filter((token) => token.type !== "space");
}

function renderBlocks(tokens: Token[]): string {
  return markedRenderer.parser(tokens);
}

function translationHtml(innerHtml: string): string {
  return `<div class="markdown-translation">${innerHtml}</div>`;
}

function htmlToken(html: string): Tokens.HTML {
  return { type: "html", block: true, pre: false, raw: html, text: html };
}

function sameText(a: Token, b: Token): boolean {
  return a.raw.trim() === b.raw.trim();
}

/**
 * Pairs original and translated blocks by the longest common subsequence of block types.
 * The translation normally keeps the block structure; LCS tolerates a few merged or split blocks.
 */
function alignBlocks(
  original: Token[],
  translated: Token[]
): Array<{ original?: Token; translated?: Token }> {
  const n = original.length;
  const m = translated.length;
  const lcs = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      lcs[i][j] =
        original[i].type === translated[j].type
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const pairs: Array<{ original?: Token; translated?: Token }> = [];
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && original[i].type === translated[j].type) {
      pairs.push({ original: original[i], translated: translated[j] });
      i += 1;
      j += 1;
    } else if (j >= m || (i < n && lcs[i + 1][j] >= lcs[i][j + 1])) {
      pairs.push({ original: original[i] });
      i += 1;
    } else {
      pairs.push({ translated: translated[j] });
      j += 1;
    }
  }
  return pairs;
}

/** Renders a list with the translation of each item placed inside the item. */
function renderBilingualList(original: Tokens.List, translated: Tokens.List): string {
  const items = original.items.map((item, index) => {
    const translatedItem = translated.items[index];
    if (sameText(item, translatedItem)) return item;
    // The original item already renders the task checkbox.
    const translatedTokens = translatedItem.tokens.filter((token) => token.type !== "checkbox");
    const itemTranslation = translationHtml(renderBlocks(translatedTokens));
    return { ...item, tokens: [...item.tokens, htmlToken(itemTranslation)] };
  });
  return renderBlocks([{ ...original, items }]);
}

/** Renders a heading with its translation on a second line, so the pair keeps one heading style. */
function renderBilingualHeading(original: Tokens.Heading, translated: Tokens.Heading): string {
  const inlineHtml = markedRenderer.Parser.parseInline(translated.tokens, markedRenderer.defaults);
  const html = `<br><span class="markdown-translation">${inlineHtml}</span>`;
  const translationToken: Tokens.Tag = {
    type: "html",
    raw: html,
    text: html,
    inLink: false,
    inRawBlock: false,
    block: false,
  };
  return renderBlocks([{ ...original, tokens: [...original.tokens, translationToken] }]);
}

/**
 * Render original and translated markdown bodies (without frontmatter) block by block:
 * each original block is followed by its translation in a `.markdown-translation` element.
 */
export function renderBilingualMarkdown(original: string, translated: string): string {
  const pairs = alignBlocks(lexBlocks(original), lexBlocks(translated));
  return pairs
    .map(({ original: source, translated: target }) => {
      if (!source) return target ? translationHtml(renderBlocks([target])) : "";
      if (!target || UNTRANSLATED_BLOCK_TYPES.has(source.type) || sameText(source, target)) {
        return renderBlocks([source]);
      }
      if (
        source.type === "list" &&
        target.type === "list" &&
        source.items.length === target.items.length
      ) {
        return renderBilingualList(source as Tokens.List, target as Tokens.List);
      }
      if (source.type === "heading" && target.type === "heading") {
        return renderBilingualHeading(source as Tokens.Heading, target as Tokens.Heading);
      }
      return renderBlocks([source]) + translationHtml(renderBlocks([target]));
    })
    .join("");
}

// Parsed frontmatter data structure
export interface ParsedFrontmatter {
  name?: string;
  description?: string;
  [key: string]: unknown;
}

// Result of parsing markdown with frontmatter
export interface ParsedMarkdown {
  frontmatter: ParsedFrontmatter;
  content: string;
  hasFrontmatter: boolean;
}

/**
 * Parse markdown content and extract frontmatter
 * Returns structured data with frontmatter and body content
 */
export function parseMarkdown(content: string | undefined | null): ParsedMarkdown {
  if (!content) {
    return {
      frontmatter: {},
      content: "",
      hasFrontmatter: false,
    };
  }

  const parsed = matter(content);

  return {
    frontmatter: parsed.data as ParsedFrontmatter,
    content: parsed.content.trim(),
    hasFrontmatter: Object.keys(parsed.data).length > 0,
  };
}

/**
 * Render markdown body only (without frontmatter)
 */
export function renderMarkdownBody(content: string | undefined | null): string {
  const { content: body } = parseMarkdown(content);
  return renderMarkdown(body);
}
