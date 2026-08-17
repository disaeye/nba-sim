/**
 * Completion facts — pure world predicates' outputs.
 * No RNG, no events, no score. See docs/foundation/architecture.md §1.3.
 */
import type { TeamId } from '../state/types.js';
import type { ShotType } from '../court/ball-motion.js';

export type CompletionFact =
  | {
      readonly kind: 'BallArrivedAtReceiver';
      readonly receiverId: string;
      readonly passerId: string;
      readonly team: TeamId;
      readonly via: 'pass' | 'handoff';
    }
  | {
      readonly kind: 'PassOutOfBounds';
      readonly passerId: string;
      readonly receiverId: string;
      readonly team: TeamId;
      readonly x: number;
      readonly y: number;
    }
  | {
      readonly kind: 'PassMissed';
      readonly passerId: string;
      readonly receiverId: string;
      readonly team: TeamId;
    }
  | {
      readonly kind: 'PassIntercepted';
      readonly stealerId: string;
      readonly victimId: string;
    }
  | {
      readonly kind: 'ShotArrivedAtRim';
      readonly shooterId: string;
      readonly shotValue: 2 | 3;
      readonly zone: string;
      readonly x: number;
      readonly y: number;
      readonly assisterId: string | null;
      readonly shotType?: ShotType | null;
    }
  | {
      readonly kind: 'HolderStripped';
      readonly stealerId: string;
      readonly victimId: string;
    }
  | {
      readonly kind: 'PlayerArrivedInPaint';
      readonly ballHandlerId: string;
      readonly offense: TeamId;
    }
  | {
      readonly kind: 'PlayerCrossedHalf';
      readonly ballHandlerId: string;
    }
  | {
      readonly kind: 'LooseRecovered';
      readonly recovererId: string;
      readonly team: TeamId;
    }
  | {
      readonly kind: 'ShotClockExpired';
      readonly team: TeamId;
    }
  | {
      readonly kind: 'GameClockExpired';
    }
  | {
      readonly kind: 'HandoffExchange';
      readonly giverId: string;
      readonly receiverId: string;
      readonly team: TeamId;
    }
  | {
      readonly kind: 'IntentPassStarted';
      readonly passerId: string;
      readonly receiverId: string;
      readonly team: TeamId;
    }
  | {
      readonly kind: 'IntentShotReleased';
      readonly shooterId: string;
      readonly shotValue: 2 | 3;
      readonly zone: string;
      readonly x: number;
      readonly y: number;
      readonly assisterId: string | null;
      readonly shotType?: 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';
    }
  | {
      readonly kind: 'IntentDriveBegun';
      readonly ballHandlerId: string;
      readonly screenerId: string | null;
    }
  | {
      readonly kind: 'IntentAdvance';
      readonly ballHandlerId: string;
    }
  | {
      readonly kind: 'CosmeticScreen';
      readonly screenerId: string;
      readonly ballHandlerId: string;
    }
  | {
      readonly kind: 'ScreenSet';
      readonly screenerId: string;
      readonly ballHandlerId: string;
      readonly defenderId: string | null;
      readonly separationBonus: number;
      readonly anchorX: number;
      readonly anchorY: number;
      readonly handlerX: number;
      readonly handlerY: number;
    }
  | {
      readonly kind: 'ScreenUsed';
      readonly screenerId: string;
      readonly ballHandlerId: string;
    };
