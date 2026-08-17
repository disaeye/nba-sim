/**
 * Render layer types — pure presentation, no kernel authority.
 */
export interface RenderPlayer {
  readonly jersey: string;
  readonly team: 'home' | 'away';
  /** Court coords [0,1] — frontend maps to pixels. */
  readonly x: number;
  readonly y: number;
  readonly zone: string;
  readonly hasBall: boolean;
  readonly action: string;
  /** §8 current stamina; −1 when unknown. */
  readonly stm: number;
  readonly stmMax: number;
}

export interface RenderBall {
  readonly x: number;
  readonly y: number;
  readonly status: string;
  readonly holderId: string | null;
}
export interface RenderTacticalRoutePoint {
  readonly x: number;
  readonly y: number;
}

export interface RenderTacticalRoute {
  readonly kind: string;
  readonly points: readonly RenderTacticalRoutePoint[];
}

export interface RenderTacticalAssignment {
  readonly jersey: string;
  readonly role: string;
  readonly action: string;
  readonly targetJersey: string | null;
  readonly lane?: string;
  readonly route?: RenderTacticalRoute;
}

export interface RenderScreenDefense {
  readonly mode: 'DROP' | 'SWITCH' | 'BLITZ' | 'HEDGE' | 'ICE';
  readonly onBallDefender: string | null;
  readonly screenerDefender: string | null;
  readonly switchesAtUse: boolean;
}
export interface RenderTactical {
  readonly kind: string;
  readonly stage: string;
  readonly offense: 'home' | 'away';
  readonly handler: string;
  readonly assignments: readonly RenderTacticalAssignment[];
  readonly screenDefense?: RenderScreenDefense;
  readonly activeAction?: {
    readonly jersey: string;
    readonly kind: string;
    readonly stage: string;
    readonly windupSeconds: number;
    readonly recoverySeconds: number;
    readonly elapsedSeconds: number;
    readonly targetZone?: string;
  };
}

export interface RenderFrame {
  readonly t: number;
  readonly t_game: number;
  readonly shotClock: number;
  readonly period: number;
  readonly phase: string;
  readonly score: { readonly home: number; readonly away: number };
  readonly players: readonly RenderPlayer[];
  readonly ball: RenderBall;
  readonly tactical?: RenderTactical;
  readonly eventType: string | null;
  readonly eventSeq?: number | null;
  readonly eventPayload?: Readonly<Record<string, unknown>>;
  readonly callout: string | null;
  readonly intensity: 'low' | 'mid' | 'high' | null;
}

/**
 * Spectator stream tick: RenderFrame plus the fields the web shell needs
 * (`gameClock`, `keyframeIndex`). Produced directly by renderFromSnapshots
 * with `asTicks: true` so the exporter holds one array, not two.
 */
export interface StreamTick extends RenderFrame {
  readonly gameClock: number;
  readonly keyframeIndex: number | null;
}

export interface RenderOptions {
  /** Subsample snapshots (1 = every tick). */
  readonly stride?: number;
  readonly language?: 'cn' | 'en';
  /** Produce spectator stream tick shape (adds gameClock + keyframeIndex). */
  readonly asTicks?: boolean;
}
