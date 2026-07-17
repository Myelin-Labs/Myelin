import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename, resolve } from "node:path";

const failures = [];

const read = (path) => readFileSync(path, "utf-8");

const fail = (message) => {
  failures.push(message);
};

const expectFile = (path) => {
  if (!existsSync(path)) fail(`missing file: ${path}`);
};

const expectDir = (path) => {
  if (!existsSync(path)) fail(`missing directory: ${path}`);
};

const expectContains = (name, value, needle) => {
  if (!value.includes(needle)) fail(`${name}: missing ${needle}`);
};

const expectNotContains = (name, value, needle) => {
  if (value.includes(needle)) fail(`${name}: unexpected ${needle}`);
};

const countContains = (value, needle) => value.split(needle).length - 1;

const root = resolve(".");
const dist = resolve(root, "dist");
const distIndex = resolve(dist, "index.html");
const distDocsIndex = resolve(dist, "docs", "index.html");
const docsSource = resolve(root, "src", "lib", "docs.ts");
const wikiRoot = resolve(root, "..", "docs", "wiki");

expectFile(distIndex);
expectFile(distDocsIndex);
expectFile(docsSource);

const indexHtml = existsSync(distIndex) ? read(distIndex) : "";
const docsHtml = existsSync(distDocsIndex) ? read(distDocsIndex) : "";
const docsSourceText = existsSync(docsSource) ? read(docsSource) : "";

const cssDir = resolve(dist, "_astro");
const cssText = existsSync(cssDir)
  ? readdirSync(cssDir)
      .filter((file) => file.endsWith(".css"))
      .map((file) => read(resolve(cssDir, file)))
      .join("\n")
  : "";

if (!cssText) fail("dist/_astro: no generated CSS found");

expectContains("home", indexHtml, '<a class="hero-release-tag"');
expectContains("home", indexHtml, 'href="https://github.com/CellScript-Labs/CellScript/releases/tag/');
expectNotContains("home", indexHtml, '<div class="hero-release-tag"');

const valueCopyCount = countContains(indexHtml, "value-card-copy");
if (valueCopyCount < 3) fail(`home: expected at least 3 value-card-copy layers, found ${valueCopyCount}`);

const exampleCopyCount = countContains(indexHtml, "landing-example-copy");
if (exampleCopyCount < 4) fail(`home: expected at least 4 landing-example-copy layers, found ${exampleCopyCount}`);

for (const token of [
  "--image-caption-bg",
  ".hero-release-tag:hover",
  ".value-card-copy",
  ".landing-example-copy",
  "text-shadow:none",
  "text-wrap:normal",
]) {
  expectContains("generated CSS", cssText, token);
}

const newDocSlugs = [
  "tutorial-09-action-model-and-canonical-syntax",
  "tutorial-13-agentic-loops-and-cellscript-mcp",
];

const oldDocSlugs = [
  "tutorial-09-action-model-and-0-13-syntax",
  "tutorial-13-agentic-loops-and-cellc-mcp",
];

for (const slug of newDocSlugs) {
  expectDir(resolve(dist, "docs", slug));
  expectContains("docs index", docsHtml, `/docs/${slug}/`);
}

for (const slug of oldDocSlugs) {
  if (existsSync(resolve(dist, "docs", slug))) fail(`dist/docs: stale generated slug ${slug}`);
  expectNotContains("docs index", docsHtml, slug);
  expectNotContains("home", indexHtml, slug);
}

const orderChecks = [
  [
    "/docs/tutorial-03-resources-and-cell-effects/",
    "/docs/tutorial-09-action-model-and-canonical-syntax/",
    "/docs/tutorial-10-standard-library/",
  ],
  [
    "/docs/tutorial-13-agentic-loops-and-cellscript-mcp/",
    "/docs/ckb-glossary/",
  ],
];

for (const chain of orderChecks) {
  let previous = -1;
  for (const href of chain) {
    const next = docsHtml.indexOf(href);
    if (next === -1) {
      fail(`docs index: missing ordered href ${href}`);
      continue;
    }
    if (previous !== -1 && next <= previous) {
      fail(`docs index: ${href} appears out of order in ${chain.join(" -> ")}`);
      break;
    }
    previous = next;
  }
}

const docsOrderMatch = docsSourceText.match(/const docsOrder = \[([\s\S]*?)\] as const;/);
if (!docsOrderMatch) {
  fail("src/lib/docs.ts: could not find docsOrder");
} else {
  const docsOrder = [...docsOrderMatch[1].matchAll(/"([^"]+\.md)"/g)].map((match) => match[1]);
  for (const file of docsOrder) {
    if (existsSync(wikiRoot) && !existsSync(resolve(wikiRoot, file))) {
      fail(`src/lib/docs.ts: docsOrder entry does not exist in docs/wiki: ${file}`);
    }
  }
  for (const stale of [
    "Tutorial-09-Action-Model-and-0-13-Syntax.md",
    "Tutorial-13-Agentic-Loops-and-cellc-mcp.md",
  ]) {
    if (docsOrder.includes(stale)) fail(`src/lib/docs.ts: stale docsOrder entry ${stale}`);
  }
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(
  `dist regressions ok (${basename(distIndex)}, ${newDocSlugs.length} renamed docs, ${valueCopyCount} value layers, ${exampleCopyCount} example layers)`,
);
