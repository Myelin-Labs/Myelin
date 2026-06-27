/* tslint:disable */
/* eslint-disable */

/**
 * Compile CellScript source to metadata JSON (path A, no ELF).
 *
 * Returns a JSON string. On success this is the serialized
 * `CompileMetadata` (module, types, actions with effect_class /
 * consume_set / create_set / estimated_cycles, etc.). On error it
 * is `{"error": "<message>"}`.
 *
 * The `target` argument is optional; pass `None` for the default
 * (ckb) target profile.
 */
export function compile_metadata_json(source: string, target?: string | null): string;

/**
 * Compile CellScript source and return a stable result envelope for tools.
 *
 * On success the response is:
 * `{ "metadata": <CompileMetadata>, "diagnostic_count": 0, "error_count": 0, "warning_count": 0, "diagnostics": [] }`
 *
 * On failure the response is:
 * `{ "metadata": null, "diagnostic_count": N, "error_count": E, "warning_count": W, "diagnostics": [{ message, severity, code, range }, ...] }`
 *
 * `range` is omitted when the compiler error is not tied to a source
 * span. Offsets are UTF-8 byte offsets from the original source; line and
 * column are 1-based.
 */
export function compile_metadata_json_diagnostics(source: string, target?: string | null): string;

/**
 * Compile a virtual multi-file source set and return metadata diagnostics.
 *
 * `sources_json` must be a JSON array of `{ path, source, role? }` objects.
 * `entry_path` selects the source that should produce metadata. This is an
 * additive API; the single-source functions remain stable.
 */
export function compile_metadata_json_sources(sources_json: string, entry_path: string, target?: string | null): string;

/**
 * Query the in-process CellScript language service for browser tooling.
 *
 * `line` and `character` are zero-based UTF-16 positions, matching LSP.
 * The result contains completion, hover, definition and current document
 * diagnostics in one JSON payload so the playground can avoid multiple
 * WASM calls per cursor move.
 */
export function language_service_json(source: string, line: number, character: number): string;

/**
 * Return the compiler version string (e.g. "0.17.0").
 */
export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly compile_metadata_json: (a: number, b: number, c: number, d: number) => [number, number];
    readonly compile_metadata_json_diagnostics: (a: number, b: number, c: number, d: number) => [number, number];
    readonly compile_metadata_json_sources: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number];
    readonly language_service_json: (a: number, b: number, c: number, d: number) => [number, number];
    readonly version: () => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
