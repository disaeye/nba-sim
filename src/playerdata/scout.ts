/**
 * 球探报告呈现 (§1.7) — five blocks: 体测区 (exact) / 能力档位区 (grade +
 * 5-cell confidence bar) / 风格描述区 (3~5 lines, L3+) / 角色适配区 (observed
 * Fit, L3+) / 样本区 (已观察 N 回合).
 *
 * Unlock cadence: L1 physical + wide-range grades; L3 adds style lines and
 * role fit; L5 presents with σ halved (single-grade presentation).
 *
 * Style lines read Tendency_obs — never the hidden truth (§1.8).
 */
import { allAttributes, attributeValue, gradeOf } from './aggregate.js';
import { observedFitSources, computeFit, reportLabel, OFFENSE_ROLE_IDS, DEFENSE_ROLE_IDS, ABILITY_TO_ATTRIBUTE } from './fit.js';
import { observedPlayer, presentationSigma } from './observation.js';
import { OBSERVATION, SCOUT } from './tables.js';
import { clip } from './normalize.js';
import type { Rng } from '../rng/types.js';
import type {
  AbilityKey,
  AttributeKey,
  FitReportRow,
  ObservationState,
  PlayerData,
  RoleId,
  ScoutReport,
  TendencyKey,
} from './types.js';

/** Observed proxy for one ability inside the report (attribute substitution). */
function abilityObs(report: ReturnType<typeof observedPlayer>, key: AbilityKey): number {
  return report.attribute[ABILITY_TO_ATTRIBUTE[key]];
}

/** Confidence-bar fill 1..5 — shrinks as σ grows (⌛ interpretation). */
export function confidenceCells(sigma: number): number {
  const confidence = clip(1 - sigma / 16, 0, 1);
  return Math.max(1, Math.round(SCOUT.confidenceCells * confidence));
}

/**
 * Full scout report for one player at a given observation state.
 * Consumes rng draws for the observation errors (attribute/tendency/
 * awareness in fixed order, then one shared observed view for Fit).
 */
export function buildScoutReport(data: PlayerData, obs: ObservationState, rng: Rng): ScoutReport {
  const { n, scoutLevel } = obs;
  const observed = observedPlayer(data, n, scoutLevel, rng);
  const sigma = presentationSigma('attribute', n, scoutLevel);

  // ── 能力档位区 ──
  const attributes = (Object.keys(ATTRIBUTE_KEYS) as AttributeKey[]).map((key) => {
    const trueValue = attributeValue(data, key);
    const shownValue = observed.attribute[key];
    const grade = gradeOf(shownValue);
    const range = sigma >= OBSERVATION.rangeSigma
      ? { low: gradeOf(shownValue - sigma), high: gradeOf(shownValue + sigma) }
      : null;
    return { key, trueValue, shownValue, grade, range, confidence: confidenceCells(sigma) };
  });

  // ── 风格描述区 (L3+) — Tendency_obs only ──
  const styles: string[] = [];
  const rareStyles: string[] = [];
  if (scoutLevel >= SCOUT.styleUnlockLevel) {
    for (const row of SCOUT.styleText) {
      const v = observed.tendency[row.tendency as TendencyKey];
      if (v >= SCOUT.highThreshold) styles.push(row.high);
      else if (v <= SCOUT.lowThreshold) styles.push(row.low);
    }
    for (const combo of SCOUT.rareCombos) {
      const trigger = observed.tendency[combo.highTendency as TendencyKey] >= combo.highTendencyMin;
      if (!trigger) continue;
      if (combo.id === 'HARD_SHOT_MAKER') {
        // 高难度倾向高 + 能力高: avg(OD3, PUM) 观测值 ≥ 80.
        const avg = (abilityObs(observed, 'OD3') + abilityObs(observed, 'PUM')) / 2;
        if (avg >= combo.abilityMin) rareStyles.push(combo.text);
      } else if (combo.id === 'CONFIDENT_ROLE') {
        // TAKEOVER 高 + Ability 低: 平均属性 < 60.
        if (avgAttributes(observed.attribute) <= combo.abilityMax) rareStyles.push(combo.text);
      } else if (combo.id === 'OLD_SCHOOL') {
        // TPOST 高 + POST 低: 终结属性 < 55.
        if (abilityObs(observed, 'POST') <= combo.abilityLowMax) rareStyles.push(combo.text);
      }
    }
  }

  // ── 角色适配区 (L3+) — observed Fit; <50 不出现 ──
  const roleFit: FitReportRow[] = [];
  if (scoutLevel >= SCOUT.styleUnlockLevel) {
    const sources = observedFitSources(observed);
    for (const role of [...OFFENSE_ROLE_IDS, ...DEFENSE_ROLE_IDS]) {
      const fit = computeFit(sources, role as RoleId);
      const label = reportLabel(fit.value);
      if (label === '') continue;
      roleFit.push({ role: role as RoleId, observedFit: fit.value, label });
    }
    roleFit.sort((a, b) => b.observedFit - a.observedFit);
  }

  return {
    physical: data.physical,
    attributes,
    styles,
    rareStyles,
    roleFit,
    sampleRounds: n,
    sigma,
  };
}

const ATTRIBUTE_KEYS: Readonly<Record<AttributeKey, true>> = {
  SHOOTING: true, FINISHING: true, PLAYMAKING: true,
  PERIMETER_D: true, INTERIOR_D: true, REBOUNDING: true, ATHLETICISM: true,
};

function avgAttributes(attrs: ReturnType<typeof observedPlayer>['attribute']): number {
  const keys = Object.keys(ATTRIBUTE_KEYS) as AttributeKey[];
  return keys.reduce((sum, k) => sum + attrs[k], 0) / keys.length;
}

export { allAttributes };
