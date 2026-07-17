import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked, Renderer } from "marked";
import { renderSource } from "./highlight";

const here = dirname(fileURLToPath(import.meta.url));
const repoRootCandidates = [
  process.env.CELLSCRIPT_REPO_ROOT,
  resolve(here, "..", "..", ".."),
].filter((candidate): candidate is string => Boolean(candidate));
const repoRoot =
  repoRootCandidates.find((candidate) => existsSync(resolve(candidate, "docs", "wiki"))) ??
  repoRootCandidates[0];
const wikiRoot = resolve(repoRoot, "docs", "wiki");

const docsOrder = [
  "Home.md",
  "Tutorial-01-Getting-Started.md",
  "Tutorial-02-Language-Basics.md",
  "Tutorial-03-Resources-and-Cell-Effects.md",
  "Tutorial-09-Action-Model-and-Canonical-Syntax.md",
  "Tutorial-10-Standard-Library.md",
  "Tutorial-11-Scoped-Invariants-and-ProofPlan.md",
  "Cookbook-Recipes.md",
  "Tutorial-04-Packages-and-CLI-Workflow.md",
  "Tutorial-05-CKB-Target-Profiles.md",
  "Tutorial-06-Metadata-Verification-and-Production-Gates.md",
  "Tutorial-07-LSP-and-Tooling.md",
  "Tutorial-08-Bundled-Example-Contracts.md",
  "Tutorial-12-Phase1-Registry-End-to-End.md",
  "Tutorial-13-Agentic-Loops-and-cellscript-mcp.md",
  "CKB-Glossary.md",
] as const;

export interface DocsHeading {
  depth: number;
  id: string;
  text: string;
}

export interface DocsPage {
  file: string;
  slug: string;
  href: string;
  title: string;
  description: string;
  html: string;
  headings: DocsHeading[];
}

export interface DocsNeighbor {
  title: string;
  href: string;
}

const stripMarkdown = (value: string): string =>
  value
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
    .replace(/[*_#>]/g, "")
    .trim();

const slugify = (value: string): string =>
  stripMarkdown(value)
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fff]+/g, "-")
    .replace(/^-+|-+$/g, "");

const fileToSlug = (file: string): string => {
  if (file === "Home.md") return "";
  return basename(file, extname(file)).toLowerCase();
};

const slugToHref = (slug: string): string => slug ? `/docs/${slug}/` : "/docs/";

const firstParagraph = (markdown: string): string => {
  const paragraph = markdown
    .split(/\n{2,}/)
    .map((block) => block.trim())
    .find((block) => block && !block.startsWith("#") && !block.startsWith("```"));
  return paragraph ? stripMarkdown(paragraph).slice(0, 180) : "";
};

const orderedWikiFiles = (): string[] => {
  const files = readdirSync(wikiRoot).filter((file) => file.endsWith(".md") && file !== "_Sidebar.md");
  const known = docsOrder.filter((file) => files.includes(file));
  const remaining = files.filter((file) => !known.includes(file as (typeof docsOrder)[number])).sort();
  return [...known, ...remaining];
};

const buildFileHrefMap = (files: readonly string[]): Map<string, string> => {
  const map = new Map<string, string>();
  for (const file of files) {
    const href = slugToHref(fileToSlug(file));
    map.set(file, href);
    map.set(`./${file}`, href);
    map.set(basename(file, ".md"), href);
  }
  return map;
};

const cellScriptCodeLanguages = new Set(["cellscript", "cell", ".cell", "cells"]);

const codeLanguageName = (lang?: string): string => (lang ?? "").trim().split(/\s+/)[0]?.toLowerCase() ?? "";

const isCellScriptCodeLanguage = (lang?: string): boolean =>
  cellScriptCodeLanguages.has(codeLanguageName(lang));

/*
 * Markdown sources remain GitHub-Wiki friendly. The website renderer rewrites:
 * - relative wiki links: Tutorial-02-Language-Basics.md -> /docs/tutorial-02-language-basics/
 * - absolute GitHub Wiki self-links -> local /docs/
 * - other relative repo docs -> GitHub blob URLs
 */
const normalizeDocsHref = (href: string, fileHrefMap: Map<string, string>): string => {
  if (/^https?:/.test(href)) {
    try {
      const url = new URL(href);
      const wikiPrefix = /^\/(?:CellScript-Labs|a19q3)\/CellScript\/wiki\/(.+)$/;
      const wikiMatch = url.pathname.match(wikiPrefix);
      if (url.hostname === "github.com" && wikiMatch) {
        const wikiPage = decodeURIComponent(wikiMatch[1]);
        const baseHref = fileHrefMap.get(wikiPage) ?? fileHrefMap.get(`${wikiPage}.md`);
        if (baseHref) return url.hash ? `${baseHref}${url.hash}` : baseHref;
      }
    } catch {
      return href;
    }
    return href;
  }
  if (/^(mailto:|#|\/)/.test(href)) return href;
  const [pathPart, hashPart] = href.split("#");
  const decodedPath = decodeURIComponent(pathPart ?? "");
  if (!decodedPath.endsWith(".md")) return href;
  const target = basename(decodedPath);
  const baseHref = fileHrefMap.get(target) ?? fileHrefMap.get(decodedPath);
  if (!baseHref) {
    const repoPath = decodedPath.replace(/^\.\.\//, "docs/").replace(/^\.\//, "");
    return `https://github.com/CellScript-Labs/CellScript/blob/main/${repoPath}`;
  }
  return hashPart ? `${baseHref}#${slugify(hashPart)}` : baseHref;
};

const makeMarked = (fileHrefMap: Map<string, string>, headings: DocsHeading[]): Marked => {
  const renderer = new Renderer();
  const headingCounts = new Map<string, number>();

  renderer.heading = ({ tokens, depth }) => {
    const text = stripMarkdown(tokens.map((token) => token.raw).join(""));
    const baseId = slugify(text) || "section";
    const count = headingCounts.get(baseId) ?? 0;
    headingCounts.set(baseId, count + 1);
    const id = count ? `${baseId}-${count}` : baseId;
    if (depth <= 3) headings.push({ depth, id, text });
    return `<h${depth} id="${id}"><a class="docs-anchor" href="#${id}" aria-label="Link to ${escapeHtml(text)}"></a>${text}</h${depth}>`;
  };

  renderer.link = ({ href, title, tokens }) => {
    const label = tokens.map((token) => token.raw).join("");
    const normalized = normalizeDocsHref(href, fileHrefMap);
    const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
    const external = /^https?:/.test(normalized);
    const targetAttrs = external ? ' target="_blank" rel="noopener noreferrer"' : "";
    return `<a href="${escapeHtml(normalized)}"${titleAttr}${targetAttrs}>${label}</a>`;
  };

  renderer.code = ({ text, lang }) => {
    const languageName = codeLanguageName(lang);
    if (languageName === "mermaid") {
      return `<pre class="mermaid">${escapeHtml(text)}</pre>`;
    }
    const language = lang ? ` data-language="${escapeHtml(lang)}"` : "";
    if (isCellScriptCodeLanguage(lang)) {
      return `<pre class="docs-code docs-code-cellscript"${language}><code>${renderSource(text)}</code></pre>`;
    }
    return `<pre class="docs-code"${language}><code>${escapeHtml(text)}</code></pre>`;
  };

  // GitHub-style admonitions: > [!NOTE], > [!TIP], > [!WARNING], > [!IMPORTANT]
  (renderer as any).blockquote = function ({ tokens }: any) {
    const first = tokens[0];
    if (first?.type === "paragraph") {
      const raw = (first.tokens ?? []).map((t: any) => t.raw).join("");
      const m = raw.match(/^\s*\[!(NOTE|TIP|WARNING|IMPORTANT|CAUTION)\]\s*/i);
      if (m) {
        const type = m[1].toLowerCase();
        const rest = raw.slice(m[0].length);
        if (rest) {
          first.tokens = [{ type: "text", raw: rest }];
        } else {
          tokens.shift();
        }
        const inner = this.parser.parse(tokens);
        return `<div class="docs-callout docs-callout-${type}">${inner}</div>`;
      }
    }
    return `<blockquote>${this.parser.parse(tokens)}</blockquote>`;
  };

  return new Marked({ gfm: true, breaks: false, renderer });
};

const escapeHtml = (value: string): string =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");

export const getDocsPages = (): DocsPage[] => {
  if (!existsSync(wikiRoot)) return [];
  const files = orderedWikiFiles();
  const fileHrefMap = buildFileHrefMap(files);

  return files.map((file) => {
    const markdown = readFileSync(resolve(wikiRoot, file), "utf-8");
    const titleMatch = markdown.match(/^#\s+(.+)$/m);
    const fallbackTitle = basename(file, ".md").replace(/-/g, " ");
    const title = stripMarkdown(titleMatch?.[1] ?? fallbackTitle);
    const headings: DocsHeading[] = [];
    const html = makeMarked(fileHrefMap, headings).parse(markdown, { async: false }) as string;
    const slug = fileToSlug(file);
    return {
      file,
      slug,
      href: slugToHref(slug),
      title,
      description: firstParagraph(markdown),
      html,
      headings,
    };
  });
};

export const getDocsPageBySlug = (slug: string): DocsPage | undefined =>
  getDocsPages().find((page) => page.slug === slug);

export const getDocsNeighbors = (page: DocsPage, pages = getDocsPages()): { previous?: DocsNeighbor; next?: DocsNeighbor } => {
  const index = pages.findIndex((candidate) => candidate.slug === page.slug);
  return {
    previous: index > 0 ? { title: pages[index - 1].title, href: pages[index - 1].href } : undefined,
    next: index >= 0 && index < pages.length - 1 ? { title: pages[index + 1].title, href: pages[index + 1].href } : undefined,
  };
};
