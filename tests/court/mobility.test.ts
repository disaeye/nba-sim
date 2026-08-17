import { describe, it, expect } from 'vitest';
import { loadMobilityConfig, speedFor } from '../../src/court/mobility.js';

describe('mobility table', () => {
  it('loads tick_seconds 0.1 and default action speeds', () => {
    const cfg = loadMobilityConfig();
    expect(cfg.tick_seconds).toBe(0.1);
    expect(cfg.default_action_speeds.advance).toBeGreaterThan(0);
    expect(cfg.ball.pass_speed).toBeGreaterThan(0);
  });

  it('speedFor uses default action when no player override', () => {
    const cfg = loadMobilityConfig();
    expect(speedFor('99', 'advance', cfg)).toBe(cfg.default_action_speeds.advance);
    expect(speedFor('99', 'idle', cfg)).toBe(cfg.default_action_speeds.idle);
  });

  it('unknown action falls back to idle', () => {
    const cfg = loadMobilityConfig();
    expect(speedFor('1', 'not_a_real_action', cfg)).toBe(cfg.default_action_speeds.idle);
  });
});

describe('role-based speed profiles', () => {
  it('speedFor uses role profile when role is provided', () => {
    const cfg = loadMobilityConfig();
    expect(cfg.role_action_speeds).toBeDefined();
    const roleSpeed = speedFor('99', 'drive', cfg, 'ball_handler');
    const defaultSpeed = speedFor('99', 'drive', cfg);
    expect(roleSpeed).not.toBe(defaultSpeed);
  });

  it('ball_handler drives faster than screener', () => {
    const cfg = loadMobilityConfig();
    const handlerDrive = speedFor('99', 'drive', cfg, 'ball_handler');
    const screenerDrive = speedFor('99', 'drive', cfg, 'screener');
    expect(handlerDrive).toBeGreaterThan(screenerDrive);
  });

  it('screener cuts faster than it advances', () => {
    const cfg = loadMobilityConfig();
    const screenerCut = speedFor('99', 'cut', cfg, 'screener');
    const screenerAdvance = speedFor('99', 'advance', cfg, 'screener');
    expect(screenerCut).toBeGreaterThan(screenerAdvance);
  });

  it('on_ball_defender defends faster than weak_side_defender', () => {
    const cfg = loadMobilityConfig();
    const onBall = speedFor('99', 'on_ball_defend', cfg, 'on_ball_defender');
    const weakSide = speedFor('99', 'weak_side', cfg, 'weak_side_defender');
    expect(onBall).toBeGreaterThan(weakSide);
  });

  it('role precedence beats jersey override', () => {
    const cfg = loadMobilityConfig();
    // jersey 1 has player_overrides drive=0.22, but role ball_handler drive=0.22
    // jersey 4 has player_overrides drive=0.15, role screener drive=0.15
    // Different roles for different jerseys → role wins, producing differentiated speeds
    const roleSpeed = speedFor('4', 'drive', cfg, 'ball_handler');
    const jerseySpeed = speedFor('4', 'drive', cfg);
    expect(roleSpeed).not.toBe(jerseySpeed);
  });
});
