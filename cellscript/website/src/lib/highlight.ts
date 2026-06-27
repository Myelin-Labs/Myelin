/**
 * CellScript syntax highlighter for the playground editor overlay.
 *
 * This is the same tokenizer used by the landing page hero code
 * panels, extracted into a shared module so the playground textarea
 * overlay can reuse it. The token classes map 1:1 to the CSS
 * `.token-*` rules in global.css, so the two surfaces stay visually
 * consistent.
 */

const escapeHtml = (value: string) =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

const tokenGroups = {
  keyword: new Set([
    "module", "use", "has", "action", "lock", "fn", "verification", "const",
    "struct", "enum", "invariant",
  ]),
  cellKind: new Set(["resource", "shared", "receipt", "flow"]),
  cellEffect: new Set([
    "consume", "create", "destroy", "preserve", "create_unique",
    "replace_unique", "destroy_unique", "destroy_instance",
    "destroy_singleton_type", "claim", "settle",
    "launch", "read_ref", "burn_amount",
  ]),
  assertion: new Set([
    "assert_invariant", "assert_sum", "assert_conserved", "assert_delta",
    "assert_distinct", "assert_singleton", "require",
  ]),
  control: new Set([
    "if", "else", "for", "in", "while", "match", "return", "let",
    "mut", "ref", "transition", "read", "protected", "witness",
    "lock_args", "as", "from", "by",
  ]),
  builtin: new Set([
    "u8", "u16", "u32", "i32", "u64", "u128", "usize", "isize",
    "bool", "Address", "Hash",
    "String", "Vec", "env", "std", "self", "true", "false", "source",
    "witness", "ckb", "identity", "field", "script_args", "ckb_type_id",
    "singleton_type", "type_id", "identity_field", "with_lock",
    "with_capacity", "with_capacity_floor", "with_default_hash_type",
    "none", "data", "Data", "type", "Type", "hash_blake2b",
    "min", "max", "math_min", "isqrt", "spawn", "wait", "process_id",
    "pipe", "pipe_read", "pipe_write", "inherited_fd", "close",
    "require_time", "require_maturity", "require_epoch_after",
    "require_epoch_relative", "occupied_capacity", "hash_chain",
    "hash_pair", "explicit_entry", "lock_group", "type_group",
    "selected_cells", "group", "transaction", "group_input",
    "group_inputs", "group_output", "group_outputs", "inputs",
    "outputs", "cell_dep", "cell_deps",
    "header_dep", "header_deps", "trigger", "scope", "reads",
  ]),
  capability: new Set([
    "store", "create", "consume", "replace", "burn", "relock",
    "read_ref", "destroy", "retarget_type",
  ]),
  standardModuleRoot: new Set([
    "std", "env", "witness", "source", "ckb", "Vec", "verifier",
  ]),
};

type HighlightContext = {
  inCapabilityList: boolean;
};

const isWhitespaceOnly = (token: string): boolean => /^\s+$/.test(token);

const isCommentToken = (token: string): boolean =>
  token.startsWith("//") || token.startsWith("/*");

const isStringToken = (token: string): boolean =>
  token.startsWith("\"") || token.startsWith("'") || token.startsWith("b\"");

export const classifyCellToken = (token: string, context?: HighlightContext): string => {
  if (isCommentToken(token)) return "comment";
  if (isStringToken(token)) return "string";
  if (/^(?:0x[0-9a-fA-F]+|\d+)$/.test(token)) return "number";
  if (token === "->" || token === "=>") return "arrow";
  if (/^[{}()[\]#;:,.]$/.test(token)) return "punctuation";
  if (/^[<>+\-*/%=&|!]$/.test(token) || ["==", "!=", "<=", ">=", "&&", "||"].includes(token))
    return "operator";
  if (token.includes("::")) {
    const root = token.split("::")[0] ?? "";
    return tokenGroups.standardModuleRoot.has(root) ? "builtin-type" : "";
  }
  const base = token;
  if (context?.inCapabilityList && tokenGroups.capability.has(base)) return "capability";
  if (tokenGroups.cellKind.has(base)) return "cell-kind";
  if (tokenGroups.cellEffect.has(base)) return "cell-effect";
  if (tokenGroups.assertion.has(base)) return "assert";
  if (tokenGroups.control.has(base)) return "control";
  if (tokenGroups.builtin.has(base)) return "builtin-type";
  if (tokenGroups.capability.has(base)) return "capability";
  if (tokenGroups.keyword.has(base)) return "keyword";
  return "";
};

const isIdentifierStart = (char: string | undefined): boolean =>
  !!char && /[A-Za-z_]/.test(char);

const isIdentifierPart = (char: string | undefined): boolean =>
  !!char && /[A-Za-z0-9_]/.test(char);

const isHexDigit = (char: string | undefined): boolean =>
  !!char && /[0-9a-fA-F]/.test(char);

const readQuotedToken = (source: string, start: number, quoteIndex: number): string => {
  const quote = source[quoteIndex];
  let cursor = quoteIndex + 1;
  while (cursor < source.length) {
    const char = source[cursor];
    if (char === "\\") {
      cursor += 2;
      continue;
    }
    cursor += 1;
    if (char === quote) break;
  }
  return source.slice(start, cursor);
};

const readBlockComment = (source: string, start: number): string => {
  let depth = 1;
  let cursor = start + 2;
  while (cursor < source.length && depth > 0) {
    if (source.startsWith("/*", cursor)) {
      depth += 1;
      cursor += 2;
    } else if (source.startsWith("*/", cursor)) {
      depth -= 1;
      cursor += 2;
    } else {
      cursor += 1;
    }
  }
  return source.slice(start, cursor);
};

const readIdentifierToken = (source: string, start: number): string => {
  let cursor = start;
  while (isIdentifierPart(source[cursor])) cursor += 1;
  while (source.startsWith("::", cursor) && isIdentifierStart(source[cursor + 2])) {
    cursor += 2;
    while (isIdentifierPart(source[cursor])) cursor += 1;
  }
  return source.slice(start, cursor);
};

const readToken = (source: string, start: number): string | undefined => {
  const char = source[start];
  if (source.startsWith("//", start)) {
    const lineEnd = source.indexOf("\n", start);
    return source.slice(start, lineEnd === -1 ? source.length : lineEnd);
  }
  if (source.startsWith("/*", start)) return readBlockComment(source, start);
  if (source.startsWith("b\"", start)) return readQuotedToken(source, start, start + 1);
  if (char === "\"" || char === "'") return readQuotedToken(source, start, start);

  for (const operator of ["->", "=>", "==", "!=", "<=", ">=", "&&", "||"]) {
    if (source.startsWith(operator, start)) return operator;
  }

  if (source.startsWith("0x", start) && isHexDigit(source[start + 2])) {
    let cursor = start + 2;
    while (isHexDigit(source[cursor])) cursor += 1;
    return source.slice(start, cursor);
  }
  if (/[0-9]/.test(char ?? "")) {
    let cursor = start;
    while (/[0-9]/.test(source[cursor] ?? "")) cursor += 1;
    return source.slice(start, cursor);
  }
  if (isIdentifierStart(char)) return readIdentifierToken(source, start);
  if ("{}()[]#;:,.<>+-*/%=&|!".includes(char ?? "")) return char;
  return undefined;
};

const updateContext = (context: HighlightContext, token: string): void => {
  if (isWhitespaceOnly(token) || isCommentToken(token) || isStringToken(token)) return;
  if (token === "has") {
    context.inCapabilityList = true;
    return;
  }
  if (context.inCapabilityList && token === "{") {
    context.inCapabilityList = false;
  }
};

/** Highlight CellScript source into HTML with token spans. */
export const renderLine = (line: string, context: HighlightContext = { inCapabilityList: false }): string => {
  let rendered = "";
  let cursor = 0;

  while (cursor < line.length) {
    const token = readToken(line, cursor);
    if (!token) {
      rendered += escapeHtml(line[cursor] ?? "");
      cursor += 1;
      continue;
    }
    const tokenClass = classifyCellToken(token, context);
    rendered += tokenClass
      ? `<span class="token-${tokenClass}">${escapeHtml(token)}</span>`
      : escapeHtml(token);
    updateContext(context, token);
    cursor += token.length;
  }

  return rendered;
};

/** Highlight a full source string (multiple lines) into HTML. */
export const renderSource = (source: string): string => {
  const context: HighlightContext = { inCapabilityList: false };
  return renderLine(source, context);
};
