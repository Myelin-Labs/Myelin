# Myelin Website Audit

Audit of `/Users/arthur/RustroverProjects/Myelin/website/` (the Astro static
landing site). Scope: source, build output, assets, claims. Compared against
`README.md`, `MYELIN_USE_CASE_POSITIONING.md`, and the runtime spine.

Status facts are based on the source as of this audit. The website has no
release tag, so the audit pins to "current source on disk" rather than a
version number.

## 1. Build hygiene — clean

| Check | Result |
| --- | --- |
| `npm run build` (`astro check && astro build`) | passes, 0 errors / 0 warnings / 0 hints |
| Astro version | 5.18.2 installed (declared `^5.9.2`) |
| Output mode | `static` |
| Shipped JS runtime | none (`dist/` has no `.js` chunk) |
| `dist/index.html` | 18.6 KB |
| `dist/_astro/index.CTmUh_-V.css` | 11.8 KB |
| `tsconfig.json` | extends `astro/tsconfigs/strict` |
| `src/env.d.ts` | minimal `astro/client` reference |
| Repo-level `.gitignore` | covers `/website/dist/`, `/website/node_modules/`, `/website/.astro/` |
| Astro config | `site: https://myelin.network`, dev toolbar disabled |
| lucide icons | 14 inline SVGs, no extra HTTP |

No fixes needed for build hygiene.

## 2. Claim discipline — mostly disciplined, three fixes

The site's positioning is broadly consistent with
`MYELIN_USE_CASE_POSITIONING.md`. The hero strip explicitly disclaims
the easy marketing lines ("not a CKB full node · not a new L1 · early
closed-validator session fast paths"), which matches §6.3's safe-to-claim
list.

Three places where the line drifts:

### 2.1 Bare "high-frequency" (3 occurrences)

- `src/pages/index.astro` hero lead: `"A CKB-aligned off-chain Cell session runtime for high-frequency, finite state transitions..."`
- `<meta name="description">`: same phrase.
- `<meta property="og:description">`: `"Run high-frequency Cell transitions off-chain..."`

`USE_CASE_POSITIONING.md` §6.3 explicitly calls out `"high-frequency finance"`
as not safe to claim. Bare `"high-frequency"` without a domain qualifier is
weaker, but it still reads as a throughput claim when paired with "Cell
transitions". The site is allowed architecture-fit language; make it
unambiguous. Suggest:

> "designed for high-throughput off-chain Cell transitions"
> or
> "built for many finite off-chain Cell transitions per session"

Do not pin a TPS number; no acceptance run supports one (§6.2 is explicit on
this).

### 2.2 "fixtures" mislabels the consensus engines

Evidence list, item 4:

> "Static committee and Tendermint-style precommit fixtures"

The runtime spine ships `myelin-consensus` with two engines
(`StaticClosedCommittee`, `Tendermint` BFT). The CLI command
`session open-fixture --consensus static-closed-committee` produces a
fixture *of* a session opened under that engine — the fixture is one
exercise path, not the engine itself. As written, this reads as if only
fixtures exist. Suggest:

> "Static closed-committee and Tendermint-style precommit finality, exercised by the production gate"

### 2.3 "high-frequency" / "many" / "fast" cross-check

- "Fast sessions" card body: `"Run many finite state transitions off-chain before the L1 path matters."` — design language, no number, no domain. Safe.
- "Contestable evidence" card body: `"Emit bundles that explain what changed and where a dispute should focus."` — present tense. Supported by the production gate (§6.1 lists the court-bundle → verify-court-bundle path). Safe.
- "Future L1 court" flow step: `"The disputed path is designed around CKB-style transaction context and CKB-VM verification."` — future tense, design language. Safe.
- "CKB aligned" card body: `"Optimise for CKB-style context and CKB-VM semantics, not an independent L1."` — accurate per README. Safe.
- CTA: `"Start with a report, not a slogan"` — on-brand. Safe.

Net: §2.1 and §2.2 are the only material callouts. The rest of the prose
holds the architecture-fit / production-evidence line.

## 3. Asset and image issues — highest impact

### 3.1 Placeholder image is 1.49 MB

`website/public/media/protocol-slot.png`:

- File size: 1,490,026 bytes (1.49 MB)
- Actual dimensions: 1672 × 941
- Labeled in figcaption as **1280 × 720** — wrong

This single asset dominates the page weight. The page otherwise ships
~32 KB of HTML+CSS. The image is also referenced twice — once in the hero
(`loading="eager"`) and once in the Evidence section
(`loading="lazy"`).

Two separate problems:

1. **The file is much larger than it needs to be.** A 1672 × 941 PNG
   for a decorative "slot" frame is unreasonable even before
   optimisation. Convert to WebP/AVIF and target ~80–120 KB.
2. **The figcaption dimension label is wrong.** Either update the
   label to match the asset, or resize the asset to match the label.
3. **Both figures use the same image.** The Evidence section's figcaption
   says "Session evidence screenshot slot" — a slot for *evidence*,
   not for the protocol diagram. When real screenshots exist
   (CLI report outputs, court-bundle JSON preview), they should drop in
   here.

### 3.2 `<img>` has no width/height — CLS risk

Both `<img>` tags have no intrinsic size attributes, only CSS
`aspect-ratio: 16/9` and `width: 100%`. The browser still doesn't know
the natural pixel size until decode, so layout shifts during image
load. For a 1.49 MB PNG on a dark hero, the shift is visible. Add
`width="1672" height="941"` (or use Astro's `<Image>` from
`astro:assets`, which generates intrinsic dimensions automatically
and produces optimised output).

### 3.3 No `<Image>` / no asset pipeline

`astro:assets` is available (Astro 5) but unused. The current `src/`
has no `assets/` directory; the `public/` copy path means the file
ships untouched. Adopting `<Image>` for the two figures gives:

- build-time resize / WebP / AVIF generation
- intrinsic width / height injection
- automatic `loading="lazy"` / `decoding="async"`
- LCP `fetchpriority="high"` for the hero image

## 4. Performance

### 4.1 Google Fonts via CSS `@import` — render-blocking + FOUT

`src/styles/global.css` line 1:

```css
@import url("https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600&family=Outfit:wght@400;500;600;700;800&display=swap");
```

Two problems:

1. `@import` inside CSS forces the browser to download the CSS, parse
   it, discover the import, then start the font request — chained
   blocking. A `<link rel="stylesheet">` in `<head>` parallelises.
2. The CSS file is shipped as a render-blocking stylesheet regardless,
   so first paint waits on both the CSS and the font CSS. Even with
   `&display=swap`, the swap itself is visible on a marketing page where
   the hero `<h1>` is set in `Outfit`.

Suggested move (still external, much better):

```html
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet"
  href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;500;600&family=Outfit:wght@400;500;600;700;800&display=swap">
```

Or self-host both WOFF2 files (only ~40 KB total for the weights used)
and reference via `@font-face` with `font-display: swap`.

### 4.2 Decorative paint cost — acceptable

`body::before` uses `position: fixed` with three stacked gradients,
and `.media-frame::before` uses `mix-blend-mode: screen`. These add
paint cost on scroll but are acceptable for a single-page marketing
site. No fix needed unless the page grows.

## 5. SEO / meta — many missing

Present:

- `<meta name="description">`
- `<meta property="og:title">`, `og:description`
- `<title>`
- `<meta name="color-scheme" content="dark">`

Missing (in rough priority order):

- `<link rel="canonical" href="https://myelin.network/">`
- `<meta property="og:type" content="website">`
- `<meta property="og:url" content="https://myelin.network/">`
- `<meta property="og:site_name" content="Myelin">`
- `<meta property="og:image" content="https://myelin.network/media/protocol-slot.png">` (after the image is optimised)
- `<meta name="twitter:card" content="summary_large_image">`
- `<meta name="twitter:title" content="...">`
- `<meta name="twitter:description" content="...">`
- `<meta name="twitter:image" content="...">`
- `<meta name="theme-color" content="#050816">` (matches `var(--bg)`)
- `<link rel="icon" href="/favicon.svg" type="image/svg+xml">`
- `<link rel="sitemap" href="/sitemap-index.xml">`
- JSON-LD: `Organization` or `SoftwareApplication` schema

For a single-page static site, adding `@astrojs/sitemap` gives a
sitemap automatically.

## 6. Accessibility — solid baseline, two cosmetic notes

- ✓ Skip link (`Skip to content`) with focus-visible transform.
- ✓ `aria-labelledby` on every `<section>`, `aria-label` on `<nav>` and
  `aria-hidden="true"` on all decorative SVGs.
- ✓ `focus-visible` outline (cyan, 2 px, 3 px offset) on links and
  buttons.
- ✓ All text/luma combinations look within WCAG AA on the dark
  background (`#050816`); blue `#49a6ff` against `#050816` clears AAA
  for large text.
- ✓ Semantic structure: single `<h1>`, multiple `<h2>`, no skipped
  levels.
- ⚠ `.hero-lead::first-line { color: var(--blue-strong); }` depends on
  the browser's first-line calculation, which is uneven across
  reflow. Cosmetic only, not a screen-reader concern.
- ⚠ Brand-mark SVG is decorative (`aria-hidden`) but has no `<title>`
  element. Optional polish — the wrapper already carries
  `aria-label="Myelin home"`.

No accessibility blockers.

## 7. Responsive — clean, with one navigation gap

- Breakpoints at `1040px` and `720px`.
- Hero / what / split / module-board all collapse to one column at
  `≤1040px`.
- Flow 5-col → 2-col at `1040px` → 1-col at `720px`.
- Header `GitHub` link collapses to icon-only at `≤720px`.
- Header `.site-nav` is hidden at `≤1040px` with no replacement
  (no hamburger, no drawer). On mobile, the only exits from the page
  are the GitHub icon and the in-page CTAs.

Acceptable for a marketing landing page — the CTA buttons and GitHub
icon both lead off the page, and the in-page anchor links are not the
primary navigation. Worth noting if a longer site grows out of this
shell.

## 8. Astro / TypeScript setup

- `astro.config.mjs`: minimal and correct. `devToolbar.enabled = false`
  is a sensible default for a static marketing site.
- `tsconfig.json`: extends `astro/tsconfigs/strict`.
- `src/env.d.ts`: only the `astro/client` reference.
- `package.json` scripts: `dev`, `build` (with `astro check`),
  `preview`. All bind to `127.0.0.1`, which is the right default for
  a site that does not need LAN access.

No setup issues. `astro check` runs as part of `npm run build`, so
template errors surface in CI.

## 9. Drift / inconsistencies

- **Figcaption dimension label**: 1280 × 720 vs actual 1672 × 941. Fix
  one or the other (§3.1).
- **Same image twice** in hero and Evidence: the Evidence figcaption
  promises a session-evidence screenshot but delivers the protocol
  diagram again. Fix by swapping in a real evidence asset when one
  exists (§3.1).
- **`package.json` version `0.1.0`** vs no version narrative in
  source: consistent. No `v1`/`v2` framing in copy or README either.

## 10. Recommended fix order

P0 — do first:

1. Optimise `protocol-slot.png` (WebP/AVIF, ~80 KB target) or replace
   with a smaller slot. Update the figcaption dimension.
2. Add `width`/`height` on both `<img>` tags (or migrate to
   `<Image>` from `astro:assets`).
3. Swap the second `<figure>` to use a real evidence screenshot when
   one is available; otherwise remove the slot figure or change the
   caption to make the placeholder nature explicit.
4. Move Google Fonts from CSS `@import` to `<link>` in `<head>` with
   `preconnect` hints.
5. Reword "high-frequency" → "designed for high-throughput off-chain
   Cell transitions" (or similar). Apply in hero lead, meta
   description, OG description.
6. Reword evidence item 4: "Static closed-committee and Tendermint-style
   precommit finality, exercised by the production gate."

P1 — quality:

7. Fill in OG / Twitter / canonical / favicon / theme-color meta in
   `<head>`.
8. Add an `Organization` JSON-LD block (name, url, logo, sameAs links).
9. Add `@astrojs/sitemap` integration.
10. Consider a small mobile nav (drawer or hamburger) if the site
    grows past one page.

P2 — polish:

11. Drop the `::first-line` styling on `.hero-lead` or replace with a
    stable token.
12. Add `<title>` to the brand-mark SVG (cosmetic; not a11y-critical).