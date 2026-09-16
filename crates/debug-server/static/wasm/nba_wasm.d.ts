/* tslint:disable */
/* eslint-disable */

/**
 * JavaScript-facing facade for the authoritative Rust match service.
 *
 * The bridge deliberately exposes JSON strings rather than internal domain
 * structs: JavaScript can observe snapshots and events, but cannot mutate
 * score, possession, player positions, or ball outcomes directly.
 */
export class WasmMatchService {
    free(): void;
    [Symbol.dispose](): void;
    events_since_json(sequence: bigint): string;
    fastForwardMs(duration_ms: number): string;
    constructor();
    next_possession_json(): string;
    pause(): void;
    resume(): void;
    sessionState(): string;
    setup_match_json(json: string): string;
    snapshot_json(): string;
    start(): void;
    tickMs(dt_ms: number): string;
    static withSeed(seed: bigint): WasmMatchService;
}

/**
 * 返回默认 GameRules 的 JSON 字符串（供前端直接初始化规则编辑器）。
 */
export function getDefaultRulesJson(): string;

/**
 * 在浏览器内存中直接运行指定 scope 的模拟，输出 NDJSON 字符串。
 * 完全无需后端服务器参与，0 磁盘消耗。
 */
export function simulateToNdjson(seed: bigint, scope: string, rules_json?: string | null): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmmatchservice_free: (a: number, b: number) => void;
    readonly getDefaultRulesJson: () => [number, number, number, number];
    readonly simulateToNdjson: (a: bigint, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly wasmmatchservice_events_since_json: (a: number, b: bigint) => [number, number, number, number];
    readonly wasmmatchservice_fastForwardMs: (a: number, b: number) => [number, number, number, number];
    readonly wasmmatchservice_new: () => number;
    readonly wasmmatchservice_next_possession_json: (a: number) => [number, number, number, number];
    readonly wasmmatchservice_pause: (a: number) => void;
    readonly wasmmatchservice_resume: (a: number) => [number, number];
    readonly wasmmatchservice_sessionState: (a: number) => [number, number];
    readonly wasmmatchservice_setup_match_json: (a: number, b: number, c: number) => [number, number, number, number];
    readonly wasmmatchservice_snapshot_json: (a: number) => [number, number, number, number];
    readonly wasmmatchservice_start: (a: number) => [number, number];
    readonly wasmmatchservice_tickMs: (a: number, b: number) => [number, number, number, number];
    readonly wasmmatchservice_withSeed: (a: bigint) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
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
