import { describe, expect, it } from 'vitest';
import { coordinateWeaksideMotion } from '../../src/strategy/dual-track-coordinator.js';
import { evaluateDefensiveRead, routeTacticalBranch } from '../../src/strategy/play-graph.js';
import type { LiveCourtSense } from '../../src/perception/live-court.js';

describe('Tactics & Dual-Track First-Principles Architecture', () => {
  const baseSense: LiveCourtSense = {
    team: 'BOS',
    defense: 'MIA',
    handler: 'J.Tatum',
    handlerTarget: 'J.Butler',
    handlerPose: { x: 0.75, y: 0.5 },
    paintDefenders: 2,
    spacePressure: 0.3,
    ball: { x: 0.75, y: 0.5, z: 1.0, carrier: 'J.Tatum', loose: false },
    rim: { x: 0.946, y: 0.5 },
    offense: 'BOS',
    offensePlayers: [
      { jersey: 'J.Tatum', pose: { x: 0.75, y: 0.5 } },
      { jersey: 'J.Brown', pose: { x: 0.78, y: 0.15 } },
      { jersey: 'K.Porzingis', pose: { x: 0.78, y: 0.85 } },
      { jersey: 'D.White', pose: { x: 0.72, y: 0.3 } },
      { jersey: 'J.Holiday', pose: { x: 0.85, y: 0.85 } },
    ] as any,
    defensePlayers: [
      { jersey: 'J.Butler', pose: { x: 0.77, y: 0.5 } },
      { jersey: 'B.Adebayo', pose: { x: 0.85, y: 0.5 } },
      { jersey: 'T.Herro', pose: { x: 0.79, y: 0.18 } },
      { jersey: 'T.Rozier', pose: { x: 0.74, y: 0.32 } },
      { jersey: 'H.Highsmith', pose: { x: 0.86, y: 0.83 } },
    ] as any,
    tacticalContext: {
      weaksideJerseys: ['K.Porzingis', 'J.Holiday'],
      hasStrongCorner: true,
      hasWeakCorner: true,
    } as any,
  };

  it('coordinates dual-track off-ball motion according to defensive and offensive spacing', () => {
    const weaksideJerseys = ['K.Porzingis', 'J.Holiday'];
    const weakside = coordinateWeaksideMotion(baseSense, weaksideJerseys, 'HORNS_CHEST', 'HORNS');
    expect(weakside).toBeDefined();
    expect(weakside.type).toBe('PIN_DOWN');
    expect(weakside.screenerId).toBe('K.Porzingis');
    expect(weakside.cutterId).toBe('J.Holiday');
  });

  it('evaluates defensive read and routes tactical branch dynamically without hardcoding', () => {
    const dropSense = { ...baseSense, paintDefenders: 2 };
    const read = evaluateDefensiveRead(dropSense, 'PNR_ROLL');
    expect(read).toBe('DROP');
    const nextPlay = routeTacticalBranch('PNR_ROLL', read);
    expect(nextPlay).toBe('PNR_POP'); // Against Drop, branch to Pop/Stepback 3
  });

  it('branches to Short Roll / Reset on Blitz/Trap coverage', () => {
    // 2 defenders swarming within 6ft of handler
    const blitzSense: LiveCourtSense = {
      ...baseSense,
      defensePlayers: [
        { jersey: 'J.Butler', pose: { x: 0.755, y: 0.51 } },
        { jersey: 'B.Adebayo', pose: { x: 0.76, y: 0.49 } },
        { jersey: 'T.Herro', pose: { x: 0.79, y: 0.18 } },
        { jersey: 'T.Rozier', pose: { x: 0.74, y: 0.32 } },
        { jersey: 'H.Highsmith', pose: { x: 0.86, y: 0.83 } },
      ] as any,
    };
    const read = evaluateDefensiveRead(blitzSense, 'PNR_ROLL');
    expect(read).toBe('BLITZ');
    const nextPlay = routeTacticalBranch('PNR_ROLL', read);
    expect(nextPlay).toBe('DRIVE_KICK'); // Blitz forces ball out of trap to open spacer
  });
});
