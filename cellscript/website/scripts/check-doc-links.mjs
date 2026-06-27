import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";

const distDocs = resolve("dist", "docs");

const walkHtml = (dir) => {
  const entries = readdirSync(dir, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walkHtml(path);
    return entry.name === "index.html" ? [path] : [];
  });
};

const htmlFiles = existsSync(distDocs) ? walkHtml(distDocs) : [];
const failures = [];

for (const file of htmlFiles) {
  const html = readFileSync(file, "utf-8");
  const hrefs = [...html.matchAll(/\shref="([^"]+)"/g)].map((match) => match[1]);

  for (const href of hrefs) {
    if (!href.startsWith("/docs/")) continue;
    const [pathPart, hashPart] = href.split("#");
    const relativePath = pathPart.replace(/^\//, "");
    const target = pathPart.endsWith("/")
      ? resolve("dist", relativePath, "index.html")
      : resolve("dist", relativePath);
    if (!existsSync(target)) {
      failures.push(`${file}: missing docs target ${href}`);
      continue;
    }
    if (hashPart) {
      const targetHtml = readFileSync(target, "utf-8");
      const targetIds = new Set([...targetHtml.matchAll(/\sid="([^"]+)"/g)].map((match) => match[1]));
      if (!targetIds.has(hashPart)) failures.push(`${file}: missing anchor ${href}`);
    }
  }
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(`docs links ok (${htmlFiles.length} pages)`);
