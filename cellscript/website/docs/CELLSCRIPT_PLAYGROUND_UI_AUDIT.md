# CellScript Playground — UI/UX audit (June 2026)

> **Historical UI snapshot.** This audit records the playground state observed
> during the June 2026 pass, including the then-visible `cellc v0.17.0` status
> string and the UI behavior documented below. The website and playground may
> have moved on since this snapshot; rerun the playground audit before using
> these findings as current 0.21 evidence.

A full-pass audit of `website/src/pages/playground.astro` and its supporting
code (highlight, translations, data, global.css). The audit was conducted by
reading the source, running the dev server, and exercising every interactive
flow via Playwright in a real browser at three viewports (1440×900,
800×1100, 375×812) and two themes (light/dark), with both English and
Chinese locales.

Scope: `/playground` (the live compiler IDE). The audit does **not** cover
the landing page, the registry pages, or the NavRail itself except where
the NavRail and the playground overlap (toolbar, nav toggle, theme, locale).

---

## TL;DR

The playground is a credible zero-dependency compiler playground. It
loads a real WASM build, debounces edits to a 300 ms compile, surfaces
the result across four tabs plus a right-side provenance rail, and has
working example switching, share-link, export, theme, and language
toggle. The visual chrome is restrained, the dark/light themes are
legible, and the design constraints in the source (textarea + overlay,
≤ 600 KB gzip WASM, no new keyframes) are honoured.

It is, however, materially behind its own RFC. The RFC
(`docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md` §4.3, §4.4, §4.5,
§10.2, §10.3) names several capabilities that are not in the build
today — most notably error-span underlines, JSON syntax highlighting,
an AST tab, expandable rail actions, auto-indent on Enter, and
mobile tab switching. There are also a handful of real visual / UX
bugs that are visible on first load and that the RFC did not anticipate.

Headline numbers from the audit:

- 18 examples in a flat `<select>` (no grouping, no search, no recent).
- Textarea has no `:focus-visible` ring (every other interactive
  element does).
- WASM bundle: 1.6 MB raw, **461 KB gzipped** — under the 600 KB
  ceiling stated in the source comment.
- Compile p50: ≤ 5 ms for the token example on a desktop browser.
- 4 output tabs, none of them are jumpable from a click on the
  error (the error is a plain text block, not a list of source
  positions).

The rest of the document enumerates findings with file/line references
and screenshots, grouped by severity.

---

## Severity scale

| Tag        | Meaning                                                                          |
|------------|----------------------------------------------------------------------------------|
| **Bug**    | Visible broken behaviour, must fix.                                              |
| **Gap**    | Feature promised in the RFC, missing in the build.                               |
| **UX**     | Works, but the user has to work around it.                                       |
| **a11y**   | Accessibility regression or missed opportunity.                                  |
| **Nit**    | Small polish, judgement call.                                                     |

---

## 1. What works (evidence captured)

These were tested end-to-end and should not regress.

- **WASM load + initial compile**: page renders the three-column
  skeleton with "Loading compiler…", the WASM module loads, the version
  string (`cellc v0.17.0`) appears in the status bar, and the initial
  token example compiles in ≤ 22 ms.
- **Live debounce**: typing in the editor schedules a 300 ms compile;
  the compile button state cycles `idle → compiling → ready/error` and
  the status bar reports "compiled in Nms".
- **Ctrl/Cmd + Enter**: `window` keydown listener triggers an immediate
  compile from anywhere on the page.
- **Tab key**: inserts two spaces (does not steal focus).
- **Example switcher**: selecting an example replaces the source,
  re-runs compile, and updates the lesson banner (`updateExampleLesson`).
- **Deep links**: `#src=<base64>` wins over `?example=…` which wins
  over localStorage. The restored-source banner is suppressed when
  a deep link is present. The base64 round-trips UTF-8 correctly.
- **Share button**: produces
  `…/playground?example=<id>#src=<base64>` and writes it to the
  clipboard; the button flashes "Copied link" for 1.6 s.
- **Export**: produces `<example-id>.cell` (e.g. `token.cell`) using
  the example's filename or a module-derived slug; the button flashes
  "Downloaded .cell" for 1.6 s.
- **localStorage auto-save**: every keystroke writes the source to
  `cellscript-playground-source`; on next visit, the source is
  restored and a "Recovered the draft saved in this browser." banner
  appears, dismissable, auto-hiding after 6 s.
- **Theme toggle**: dark / light; defaults to `prefers-color-scheme`;
  persisted in `cellscript-theme`; in both themes the syntax
  highlighting and JSON copy button remain legible.
- **Language toggle**: English / Chinese; persisted in
  `cellscript-locale`; applied live (no reload) via
  `cellscript:locale-change` event, including the compile-button
  label and the status bar text.
- **Error auto-select**: on a failed compile the output panel
  switches to **Errors**, the right rail shows the error inline, and
  the Errors tab badge displays the error count.
- **Metadata copy button**: delegated click handler reads
  `data-copy-text`, writes to the clipboard, toggles `is-copied` for
  ✓ feedback, then reverts after 1.6 s.
- **NavRail view transition**: the rail keeps `view-transition-name:
  nav-rail` so the sidebar does not flash when navigating between
  `/` and `/playground`.

---

## 2. Real bugs (visible on first load)

### B1 — Toolbar title clipped by floating rail toggle on small screens  ·  **Bug**
- **Where**: `website/src/styles/global.css` `.pg-toolbar-title`
  (line 2940) vs `NavRail.astro` `.rail-toggle` (default visible
  whenever the rail is collapsed).
- **Repro**: collapse the NavRail (or first visit at < 1100 px on
  any device) and load `/playground`. The floating round toggle
  button is positioned at `top:16px, left:16px, 44×44` and overlaps
  the first 60 px of the toolbar's "Playground" wordmark.
- **Evidence**: `/tmp/playground-audit/08-medium-tablet.png`,
  `/tmp/playground-audit/09-mobile.png` — title is rendered as
  `P[…]round` because the toggle is on top of it.
- **Why this happens**: the toolbar is grid `1fr auto` at desktop
  and `1fr` at < 820 px. There is no left padding to reserve room
  for the toggle on the playground page (compare with other pages
  where the toggle is the only thing in the rail at this width).
- **Fix sketch**: in the < 1100 px breakpoint, give
  `.playground-body` left padding equal to the toggle's footprint
  (e.g. 60 px) when `data-rail-collapsed="true"`, or shift the
  toolbar title to start at `left: 60px` while the rail is
  collapsed.

### B2 — Long source lines are truncated mid-character, no scroll cue  ·  **Bug**
- **Where**: `global.css` `.pg-highlight, .pg-textarea` (line 3269);
  both have `white-space: pre` and live inside `.pg-editor` which is
  `overflow: hidden` (line 3232).
- **Repro**: load any example with lines wider than the source
  column (every long `create`/`require` line). The highlighter clips
  the line at the right edge of the column — e.g. line 9 of
  `token.cell` reads `…create, consume` with the rest gone, and
  the long `create next_token = Token { amount, sy…` shows the last
  two characters only.
- **Why this is worse than it looks**: the textarea has horizontal
  overflow, so the user *can* scroll right to see the rest of the
  line, but the editor gives no visual cue that more text exists.
  There is no horizontal scrollbar in the gutter area, no fade
  gradient, no `→` indicator, and the gutter numbers are not
  sticky.
- **Fix sketch**: add a soft right-edge mask (linear-gradient from
  transparent to background) to `.pg-editor`, or switch to
  `white-space: pre-wrap` and let the highlighter wrap (sacrifices
  the "code stays in one line" feel but matches the user's
  mental model when columns are tight).

### B3 — Lesson banner shows wrong example after manual edits  ·  **Bug**
- **Where**: `playground.astro` `updateExampleLesson` is called from
  (a) page load, (b) example select change, (c) restored-source
  banner, (d) language change. It is **not** called from the
  textarea's `input` handler or from compile completion.
- **Repro**: select `AMM pool`, then type any custom source. The
  banner still says "AMM pool · Teaches shared pool state…". The
  example `<select>` still says "AMM pool" but the source is
  unrelated. (See `/tmp/playground-audit/14-custom-source.png`.)
- **Fix sketch**: track `currentSource` and call
  `updateExampleLesson(null)` (which renders the "Custom editor
  source" copy) when the textarea diverges from
  `exampleSources[currentExampleId]`.

### B4 — Errors tab does not auto-switch back to Metadata after a fix  ·  **Bug**
- **Where**: `playground.astro` `doCompile` — on error, the panel
  is forced to `errors` (line 589). On success, the panel is not
  switched back, so the user remains on the Errors tab staring at
  "No errors."
- **Repro**: trigger a compile error (any syntax error), fix it,
  the button cycles to "Compiled · 1ms" but the panel still reads
  "No errors." The user has to manually click **Metadata** to see
  the new output.
- **Fix sketch**: on successful compile, switch to the metadata
  tab if the currently active tab is `errors` (or, more
  conservatively, leave it alone for users who have manually
  switched to a different tab; the bug is only the "stuck on
  errors" state).

### B5 — Share button has a clipboard race for long sources  ·  **Bug**
- **Where**: `playground.astro` `shareBtn` handler, lines 833–858.
- **Repro**: paste a 2 000-line source, click **Share**. The
  handler generates the full URL, writes it to the clipboard
  (`navigator.clipboard.writeText(url.toString())`), then checks
  if the URL is > 8 000 chars; if so, it strips `#src=…` and
  writes the short URL again. The window between the two writes
  is small but non-zero: a paste tool that samples the clipboard
  in that window gets a 30 KB+ URL.
- **Fix sketch**: measure the encoded length first, only
  `writeText` the form that will be kept.
  ```js
  const tooLong = url.toString().length > 8000;
  if (tooLong) url.hash = "";
  await navigator.clipboard.writeText(url.toString());
  flashShareBtn(tooLong ? pg("shareTooLong") : pg("copied"));
  ```

### B6 — Rail module name is truncated without an ellipsis  ·  **UX** (borderline bug)
- **Where**: `global.css` `.pg-rail-fact dd` has `word-break: break-all`
  (line 3580). This breaks mid-word with no marker, so a long module
  like `cellscript::fungible_token` becomes `cellscript::fungible_to`
  with no hint that the rest is there.
- **Fix sketch**: keep `overflow-wrap: anywhere` so the name can
  wrap; or use `text-overflow: ellipsis` with a fixed width and a
  `title="…"` attribute that shows the full value on hover.

---

## 3. RFC gaps (planned, not implemented)

The website RFC (`docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md`,
hereafter "the RFC") names several capabilities that the build
does not have today. They are not bugs in the code; they are
features that didn't make it.

### G1 — No error underlines on the source  ·  **Gap** (RFC §4.3, §10.2)
The RFC says "Errors carry span info; the editor renders them as red
wavy underlines on the offending line. Hover reveals the message in a
non-blocking inline tooltip." The current implementation only
stringifies the error and shows it in the Errors tab; the source has
no markers. To land this, the compiler output needs to include
`{line, column, length, message}` per diagnostic, the overlay needs
to accept per-line marker ranges, and the highlighter needs to keep
the marker layer in sync with the cursor on every keystroke.

### G2 — Metadata JSON has no syntax highlighting  ·  **Gap** (RFC §4.4)
The RFC says "Metadata | CompileMetadata JSON, syntax-highlighted."
The build today shows the JSON as a single wall of monospaced text
(no colour for keys, values, strings, numbers). For the AMM example
the metadata is hundreds of lines; scanning it without colours is
real friction. There is already a `cellscript-highlight` design
language on the landing page — the playground should reuse it for
JSON (key colour, string colour, number colour, comment colour).

### G3 — No AST tab  ·  **Gap** (RFC §4.4)
RFC §4.4 lists four tabs: Metadata, AST, Types, Errors. The build
only has three (the AST tab was dropped). The compiled metadata has
no AST node list in its current shape; landing the AST tab is a
compiler change (`compile_ast_json`) plus a new panel, not a
playground change alone.

### G4 — Rail actions are not expandable  ·  **Gap** (RFC §4.5)
RFC §4.5: "Actions list: each action is expandable; shows
`effect_class`, `consume_set`, `create_set`, `estimated_cycles`,
`ckb_runtime_features`." The build's right rail shows the
action list as a flat `name + effect label`; the full
consume/create/cycles are only in the **Actions** tab. Either the
rail should expand to surface the rich details, or the RFC text
should be retired — right now the build contradicts it.

### G5 — No "Format" / target-profile selector  ·  **Gap** (RFC §4.1)
RFC §4.1 ASCII mock has `[token v] [Format]` in the toolbar. The
build has only the example `<select>` and no format selector. The
playground calls `compileMetadataJson(textarea.value, null)` with
`null` target, so the user has no way to switch the target
profile from the UI.

### G6 — No auto-indent on Enter  ·  **Gap** (RFC §10.2)
RFC §10.2: "Auto-indent on Enter: the editor copies the leading
whitespace of the current line to the next." The current
`keydown` handler only handles Tab.

### G7 — No mobile tab-switching for playground columns  ·  **Gap** (RFC §10.3)
RFC §10.3: "Playground columns become tab-switchable below
820 px: [Source][Output][Provenance], one visible at a time, to
preserve editor width on tablets." The build's < 819 px
breakpoint stacks them vertically (`grid-template-rows: none`)
so on a 375 px viewport the user has to scroll through all three
sections to see the right rail. The RFC shape would give the
editor the full width and make the rail a single tap away.

### G8 — No `ckb_runtime_features` surfacing  ·  **Gap** (RFC §4.5)
Even in the Actions tab, `ckb_runtime_features` is not shown.

---

## 4. UX issues (works, but with friction)

### U1 — 18 examples in a flat `<select>`  ·  **UX**
`playgroundExamples` in `data/site.ts` has 18 entries. The dropdown
orders them roughly by topic but the user has to read the entire
list to find, say, "spawn pipeline" or "TYPE_ID create". A
grouped `<select>` (`<optgroup>`) or, better, a small chooser
sheet (filter input + grouped list) would help. The same data
already has implicit groups in `exampleGroups` (protocols /
primitives / language); reusing them would also keep the playground
and the landing page in sync.

### U2 — Empty actions rail says "—" with the heading still visible  ·  **UX**
When a source has resources but no actions, the right rail still
renders the `ACTION EFFECTS` heading and an item containing `—`.
The heading should hide when the list is empty, or render "No
actions" to match the **Actions** tab's empty state
(`pg("noActions")`).

### U3 — Share link example param is misleading for custom sources  ·  **UX**
When a user shares a heavily edited source, the URL still carries
`?example=token` (or whichever example was last selected). The
recipient opens the link, sees the example dropdown highlight
"token", but the source is unrelated. The dropdown is a label,
not the active example. Two options:
- Drop the `?example=` when the share source diverges from any
  preset.
- Switch the dropdown to "Custom" so the recipient isn't misled.

### U4 — Status bar echo of the error can be 80-char clipped  ·  **UX**
`if (status) status.textContent = data.error.slice(0, 80);` clips
the status bar to 80 characters. Errors longer than that are
visibly truncated in the bottom strip. The full error is in the
Errors tab, so this is not a correctness issue, but the truncation
is silent. Adding `…` (and a tooltip / click to copy) would make
the truncation obvious.

### U5 — No "loading" indicator on first compile after restore  ·  **UX**
The first compile after page load runs inside `requestAnimationFrame`
with a one-frame "compiling" state, so the user sees a flash of
"compiling" even on tiny files. For 0 ms compiles the flash is
distracting. Setting state to "ready" directly when the result is
synchronous would be calmer.

### U6 — No visual cue that the Errors badge is a count  ·  **UX**
The Errors tab badge renders `1` (or N) with no unit. The
`aria-label` says "1 errors" but a screen reader announcing
"1" without context is uninformative, and a sighted user has no
way to know the badge is a count of error *lines* (not error
*messages*) without reading the source. Either render `×1` or
hover with a tooltip.

### U7 — `data-i18n` re-application is triggered by `cellscript:locale-change` but the compile button state defaults to `idle`  ·  **UX**
When the user changes the language while the compile button is in
`ready` state, `applyLocaleStrings` re-renders the label correctly
("已编译 · 1ms"). Good. But if the user changes locale *while* a
compile is in flight, the in-flight label reverts to "编译中" (the
post-state `compiling` template) even if the actual compile is
about to finish in the next frame. This is harmless but can flash.

### U8 — No "copy share link" affordance besides the button  ·  **UX**
The Share button is the only way to get a URL. There is no
"Permalink" / "Copy URL" affordance on the page itself; users who
land on `/playground` after a refresh and want to save the URL
have to manually copy the address bar.

---

## 5. Accessibility (a11y)

The page has working `aria-label`s, `role="tab"` + `aria-selected`
on the output tabs, `aria-live="polite"` on the lesson banner, and
a focus ring on every button. There are, however, three real
regressions against the RFC and a handful of missed opportunities.

### A1 — Textarea has no visible focus indicator  ·  **a11y / RFC §10.4**
`global.css`:
```css
.pg-textarea:focus-visible { outline: none; }
```
This removes the browser default focus ring. Combined with
`color: transparent; -webkit-text-fill-color: transparent;` the
textarea shows no visible focus marker at all. Keyboard-only users
cannot tell where focus is. Compute: `getComputedStyle(textarea).outline`
returns `rgba(0, 0, 0, 0) none 3px`. The RFC §10.4 says "All new
interactive elements get `:focus-visible` outlines matching the site
standard (2 px accent, 3 px offset)." The textarea is the most
prominent interactive element on the page; it should not be exempt.

**Fix sketch**: replace the rule with a visible ring on the editor
container, e.g.
```css
.pg-editor:focus-within { box-shadow: inset 0 0 0 2px var(--accent); }
```

### A2 — All four output tabs in the tab order  ·  **a11y**
Standard WAI-ARIA tabs use a roving tabindex (only the active tab
has `tabindex="0"`; the rest have `tabindex="-1"` and are
navigable via Left/Right arrows). The current implementation has
`tabindex="0"` on every tab, so a keyboard user has to Tab past
all four output tabs to reach the next interactive element. Arrow
keys do not switch tabs at all.

**Fix sketch**:
```js
outputTabs.forEach((tab, i) => {
  tab.addEventListener("keydown", (e) => {
    if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      e.preventDefault();
      const dir = e.key === "ArrowRight" ? 1 : -1;
      const next = outputTabs[(i + dir + outputTabs.length) % outputTabs.length];
      selectTab(next.dataset.outputTab);
      next.focus();
    }
  });
});
```
And track `tabindex` per state.

### A3 — No skip link on `/playground`  ·  **a11y**
Other routes on the site (the landing page) carry a
`a11y.skipToContent` link, but `/playground` has none. Keyboard
users have to Tab through the NavRail, the toolbar, and the four
output tabs before they can interact with the source editor.

### A4 — The "Playground" wordmark is not a heading  ·  **a11y**
The toolbar title is a `<span class="pg-toolbar-title">`, not an
`<h1>`. For sighted users the visual weight is enough; for screen
readers there is no document outline for this page (the only `<h1>`
in the layout would be the `BrandMark` link, which says "CellScript
home" not "Playground"). At minimum the page should have a
visually-hidden `<h1>Playground</h1>` to seed the document outline.

### A5 — Compile button state changes are not announced  ·  **a11y**
The button's data-state changes from `compiling` to `ready` or
`error`; the label changes accordingly. There is no
`aria-live` region wrapping the button, so a screen reader user
has to navigate back to the button to discover the result.
Reasonable fix: add `aria-live="polite"` to the status bar
(`<span class="pg-status">`) so its "compiled in Nms" updates are
announced.

### A6 — Right rail actions are not in the tab order  ·  **a11y**
The rail is mostly informational, but the `…` actions/effects
header has no semantic role and no heading. A screen reader
announces "ACTION EFFECTS" with the same weight as "Module
cellscript::fungible_token" — they're peers in the rail's `<dl>`.
A heading element (visually the same) would make the rail
navigable.

### A7 — The example dropdown is not announced as changing the editor content  ·  **a11y**
Selecting an example mutates the textarea value and the lesson
banner. The lesson banner has `aria-live="polite"` so the title
change is announced, but the dropdown selection itself is not
announced as "Loaded example: Fungible token".

---

## 6. Performance and resilience

- **WASM size**: 1 690 877 B raw, **461 575 B gzipped** — under the
  600 KB ceiling. Note: the `cellscript_wasm.js` glue is an
  additional **2 305 B gzipped**.
- **Initial compile time**: ≤ 22 ms for `token.cell` on a desktop
  browser (observed over several runs). 1 ms for the simplest cases.
- **Highlight rebuild**: `updateHighlight` runs on every keystroke
  and calls `renderSource` which loops `String.matchAll` over the
  full source. At 200 lines it is fine; at 2 000 lines it was still
  snappy in testing, but for safety the highlight should be
  `requestAnimationFrame`-scheduled, not synchronous.
- **localStorage source cap**: there is no upper bound on the
  saved source. A user who pastes 5 MB of source into the editor
  will write 5 MB to localStorage every keystroke, which Chrome
  silently truncates to 5 MB. The keystroke handlers are not
  rate-limited; the `input` listener calls `saveSource()` and
  `scheduleCompile()` synchronously.
- **No offline path**: if the WASM module 404s, the playground
  shows "Failed to load compiler: {error}" in the status bar and
  a similar message in the rail, and sets the error panel — but
  the editor still works, so the user can still edit and see
  their own typing. They just cannot get a compile.
- **Editor state is not preserved across page navigations**: if
  the user clicks "Learn" in the NavRail and then comes back to
  the playground, the source is restored from localStorage
  (good). But the in-flight compile state is lost, so the user
  may see a one-frame "loading…" flash on return.
- **view-transition disable for NavRail**: this is correct, but
  the playground does not define its own `view-transition-name`,
  so on view-transition navigations the playground body
  cross-fades with the previous page. There is no
  `prefers-reduced-motion` carve-out for the playground itself.

---

## 7. Discoverability

### D1 — No "Open in Playground" CTA on landing example snippets
The hero pseudo-console has `Edit this live` for the active
example (good). The examples section on the landing page
(workflow / assurance cards) does not — it links to the GitHub
raw file, not the playground. For a user who wants to *try*
the example, that is a missed conversion.

### D2 — No keyboard shortcut hint
Ctrl/Cmd + Enter is the only shortcut. The toolbar does not
surface it; the button's `aria-label` says "Compile now
(Ctrl/Cmd+Enter)" but only screen reader users see that. A
small `⌘↵` glyph in the button (matching the language toggle's
visual treatment) would be low-cost.

### D3 — No link between the provenance rail and the full output
The rail summarises "module, target, format/size, actions,
types". The Metadata tab has the full JSON. There is no
"View in Metadata" affordance on the rail, so the user has to
know that the rail's `target` is a summary of the metadata's
`target_profile`.

### D4 — The lesson banner has no "load this example again" link
When the editor is in "Custom editor source" state, the banner
says "The editor is showing restored or shared source. Choose
an example to load a guided lesson." It is a paragraph; the
user has to scroll up to the dropdown to act. A small inline
"Pick an example →" link would shorten the loop.

### D5 — No way to start from a blank editor
The dropdown always has an example selected. There is no
"Blank" option that clears the source for a true "from scratch"
start. Given the 18 examples are heavy, a `Blank` entry would
also be the natural way to disambiguate `Custom` from
`No example selected`.

---

## 8. i18n review

- The full string table is embedded in the page at build time
  (`window.__CELLSCRIPT_I18N__`), which means the runtime
  toggle is instant.
- All visible UI strings in the playground are translated,
  including the `examples.<id>.title` and `examples.<id>.summary`
  keys for every example.
- The compiler error itself is not translated (it is the WASM
  output string). This is correct — error codes are part of the
  product surface, not the chrome. But a future i18n pass should
  consider whether errors should at least be wrapped in a
  localised envelope (e.g. "Compilation failed · 查看错误" / "View
  error").
- The copy button `aria-label` is correctly re-applied on
  locale change (it uses `getMessage("a11y.copyAria", readLocale())`
  at the moment of copy, not at build time).
- The Chinese locale uses `中` as the language label glyph,
  matching the existing site chrome.

---

## 9. Screenshots captured

All screenshots are at `/Users/arthur/.mavis/tmp/mcp-images/`. They
correspond to the numbered findings above.

| File                                               | Used in   |
|----------------------------------------------------|-----------|
| `mcp-image-1782238357879-f9768606.png`             | baseline dark, English  |
| `mcp-image-1782238417247-9290f344.png`             | Actions tab             |
| `mcp-image-1782238427434-37a3bdf1.png`             | Types tab               |
| `mcp-image-1782238439938-4264ae20.png`             | Error tab auto-selected |
| `mcp-image-1782238453907-1faa398b.png`             | Light theme + restored-source banner |
| `mcp-image-1782238543627-b81dcbb7.png`             | Tablet 800 px — B1      |
| `mcp-image-1782238572965-ff0fbf4f.png`             | Mobile 375 px — B1, G7  |
| `mcp-image-1782238592771-da7f659e.png`             | Empty source            |
| `mcp-image-1782238649025-4a0587e3.png`             | 200-line source — B2    |
| `mcp-image-1782238693495-4da1335d.png`             | Big source + lesson-mismatch — B3 |
| `mcp-image-1782238706672-ba143976.png`             | Custom source — B3, U2  |
| `mcp-image-1782238723320-7065251c.png`             | Chinese locale          |
| `mcp-image-1782238896606-ecc5aafe.png`             | Ctrl+Enter              |
| `mcp-image-1782238919385-0433ccd3.png`             | Export button clicked   |

---

## 10. Suggested fix order (rough ROI)

1. **B1** toolbar overlap — CSS-only fix, visible everywhere, no
   design impact. **Days: 0.1.**
2. **A1** textarea focus ring — CSS-only, restores a11y. **Days: 0.1.**
3. **B4** errors tab auto-switch on success — JS-only, ~5 lines.
   **Days: 0.1.**
4. **B3** lesson banner updates on edit — JS-only, ~10 lines.
   **Days: 0.2.**
5. **B5** share race condition — JS-only, ~5 lines. **Days: 0.1.**
6. **A2** roving tabindex + arrow keys — JS-only, ~15 lines.
   **Days: 0.3.**
7. **G2** JSON syntax highlighting — needs a small highlight
   function (or reuse) and CSS tokens. **Days: 0.5.**
8. **U1** example picker — UI rebuild. **Days: 1.0.**
9. **G1** error underlines — compiler + overlay changes. **Days: 1.5.**
10. **G3** AST tab — compiler + UI. **Days: 2.0.**
11. **G7** mobile tab switch — CSS + JS. **Days: 0.5.**

Total: ~6–7 working days for the high-priority items.

---

## 11. Cross-references

- `website/src/pages/playground.astro:589` — error → errors tab.
- `website/src/pages/playground.astro:833–858` — share handler (B5).
- `website/src/pages/playground.astro:622–631` — `selectTab` (B4, A2).
- `website/src/styles/global.css:2927` — `.pg-toolbar` grid
  (B1).
- `website/src/styles/global.css:3308` — `.pg-textarea:focus-visible`
  (A1).
- `website/src/styles/global.css:3580` — `.pg-rail-fact dd`
  (B6).
- `website/src/components/NavRail.astro:72` — `.rail-toggle` button
  (B1).
- `website/src/data/site.ts:70` — `playgroundExamples` (U1).
- `docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:267` — error
  underline (G1).
- `docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:276` — JSON
  highlighting (G2).
- `docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:281–290` — rail
  expansion (G4).
- `docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:494` — auto-indent
  (G6).
- `docs/CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:503–506` — mobile
  tab switch (G7).
