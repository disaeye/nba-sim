/**
 * Generate a two-team demo config from the §7 player generator and write it
 * as a GameInput-compatible JSON (same shape as config/demo-game.json).
 *
 *   npx tsx scripts/generate-roster.ts --seed 7 --out config/generated-roster.json
 *
 * Each roster player carries layered `playerData` (no hand-set `abilities`);
 * the kernel derives its capability overlay through the playerdata bridge.
 * Offense roles are assigned to each five-man unit by greedy Fit (distinct
 * roles per slot, §2), then translated into the engine's usageProfile
 * vocabulary (creator/screener/spacer) for the lineup packages.
 */
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { mulberry32 } from '../src/rng/mulberry32.js';
import { FOUNDATION_VERSION } from '../src/sim-utils.js';
import { generateRoster } from '../src/playerdata/generate.js';
import { trueFit, OFFENSE_ROLE_IDS, DEFENSE_ROLE_IDS } from '../src/playerdata/fit.js';
import { allAttributes, gradeOf } from '../src/playerdata/aggregate.js';
import { bridgeCapabilities } from '../src/playerdata/bridge.js';
import type { PlayerData, OffenseRoleId, RoleId } from '../src/playerdata/types.js';

interface RosterEntry {
  id: string;
  jersey: string;
  teamId: string;
  playerData: PlayerData;
}

interface AssignedUnit {
  players: readonly RosterEntry[];
  roles: Readonly<Record<string, OffenseRoleId>>;
}

function parseArgs(): { seed: number; out: string } {
  const argv = process.argv.slice(2);
  const seedIdx = argv.indexOf('--seed');
  const outIdx = argv.indexOf('--out');
  const seedValue = seedIdx !== -1 ? argv[seedIdx + 1] : undefined;
  const outValue = outIdx !== -1 ? argv[outIdx + 1] : undefined;
  const seed = seedValue !== undefined ? Number(seedValue) : 7;
  const out = outValue !== undefined ? outValue : 'config/generated-roster.json';
  return { seed, out };
}

/** Greedy distinct-role assignment by Fit: best (player, role) pairs first. */
function assignOffenseRoles(players: readonly RosterEntry[]): Readonly<Record<string, OffenseRoleId>> {
  const pairs: Array<{ jersey: string; role: OffenseRoleId; fit: number }> = [];
  for (const p of players) {
    for (const role of OFFENSE_ROLE_IDS) {
      pairs.push({ jersey: p.jersey, role, fit: trueFit(p.playerData, role).value });
    }
  }
  pairs.sort((a, b) => b.fit - a.fit || a.jersey.localeCompare(b.jersey) || a.role.localeCompare(b.role));
  const assigned = new Map<string, OffenseRoleId>();
  const usedRoles = new Set<OffenseRoleId>();
  for (const pair of pairs) {
    if (assigned.has(pair.jersey) || usedRoles.has(pair.role)) continue;
    assigned.set(pair.jersey, pair.role);
    usedRoles.add(pair.role);
    if (assigned.size === 5) break;
  }
  const out: Record<string, OffenseRoleId> = {};
  for (const [jersey, role] of assigned) out[jersey] = role;
  return out;
}

/** Build a lineup package (engine vocabulary) for a five-man unit. */
function buildPackage(unit: AssignedUnit, id: string): Record<string, unknown> {
  const roles = unit.roles;
  const capByJersey: Record<string, number> = {};
  for (const p of unit.players) capByJersey[p.jersey] = bridgeCapabilities(p.playerData).creation;
  // creator = best creation capability; screener = best screening; rest spacer.
  const sorted = [...unit.players].sort((a, b) => (capByJersey[b.jersey] ?? 0) - (capByJersey[a.jersey] ?? 0));
  // The engine's binder reads creator[0] as primary and creator[1] as
  // secondary (falling back to creator[0] when only one exists — which
  // would duplicate the jersey), so the profile needs two creators.
  const creators = [sorted[0]!, sorted[1]!];
  const screener = [...unit.players]
    .filter((p) => p.jersey !== creators[0]!.jersey && p.jersey !== creators[1]!.jersey)
    .sort((a, b) => (bridgeCapabilities(b.playerData).screening) - (bridgeCapabilities(a.playerData).screening))[0]!;
  const spacer = unit.players.filter((p) => p.jersey !== creators[0]!.jersey && p.jersey !== creators[1]!.jersey && p.jersey !== screener.jersey);
  return {
    id,
    players: unit.players.map((p) => p.jersey),
    usageProfile: {
      creator: creators.map((p) => p.jersey),
      screener: [screener.jersey],
      spacer: spacer.map((p) => p.jersey),
    },
    lineupIdentity: {
      primary: 'initiator',
      assignments: unit.players.map((p) => ({
        jersey: p.jersey,
        roles: [roles[p.jersey] ?? 'connector'],
        capabilities: bridgeCapabilities(p.playerData),
      })),
    },
  };
}

function generateTeam(teamId: string, rng: import('../src/rng/types.js').Rng): { roster: RosterEntry[]; packages: Record<string, unknown>[] } {
  const data = generateRoster({ count: 10, rng, ageMin: 19, ageMax: 35 });
  // Jerseys must be unique ACROSS teams — the kernel keys poses/abilities by
  // jersey, so home owns 1-10 and away owns 11-20 (same contract as the
  // hand-tuned demo config). A collision silently corrupts pose teams and
  // freezes every possession whose holder shares a jersey with the other
  // team (sense-null stalls, shot-clock violations).
  const offset = teamId === 'away' ? 10 : 0;
  const roster: RosterEntry[] = data.map((pd, i) => ({
    id: `${teamId}${i + 1}`,
    jersey: String(i + 1 + offset),
    teamId,
    playerData: pd,
  }));
  // Unit 1 = best five by average fit; unit 2 = the rest (bench).
  const fitSum = roster.map((p) => {
    let sum = 0;
    for (const role of [...OFFENSE_ROLE_IDS, ...DEFENSE_ROLE_IDS]) sum += trueFit(p.playerData, role).value;
    return { p, sum };
  });
  fitSum.sort((a, b) => b.sum - a.sum);
  const starters = fitSum.slice(0, 5).map((x) => x.p);
  const bench = fitSum.slice(5).map((x) => x.p);
  const unitA: AssignedUnit = { players: starters, roles: assignOffenseRoles(starters) };
  const unitB: AssignedUnit = { players: bench, roles: assignOffenseRoles(bench) };
  return {
    roster,
    packages: [buildPackage(unitA, `${teamId}_starters`), buildPackage(unitB, `${teamId}_bench`)],
  };
}

function main(): void {
  const { seed, out } = parseArgs();
  const rng = mulberry32(seed);
  const home = generateTeam('home', rng);
  const away = generateTeam('away', rng);
  const config = {
    foundation_version: FOUNDATION_VERSION,
    home_team: { id: 'home', name: 'Generated Hawks', roster: home.roster, lineup_packages: home.packages },
    away_team: { id: 'away', name: 'Generated Owls', roster: away.roster, lineup_packages: away.packages },
  };
  const path = resolve(process.cwd(), out);
  writeFileSync(path, `${JSON.stringify(config, null, 2)}\n`);
  console.log(`wrote ${path}`);
  for (const side of [home, away]) {
    console.log(`\n${side.roster[0]?.teamId}`);
    for (const p of side.roster) {
      const attrs = allAttributes(p.playerData);
      const grades = (Object.keys(attrs) as Array<keyof typeof attrs>)
        .map((k) => `${k}:${gradeOf(attrs[k])}`)
        .join(' ');
      const bestO = bestFit(p.playerData, OFFENSE_ROLE_IDS);
      const bestD = bestFit(p.playerData, DEFENSE_ROLE_IDS);
      console.log(`  #${p.jersey} ${physicalProfile(p.playerData)} | ${grades}`);
      console.log(`      best offense: ${bestO} | best defense: ${bestD}`);
    }
  }
}

function physicalProfile(pd: PlayerData): string {
  const { H, WT, VJ, SPD, LAT, AGE, DUR } = pd.physical;
  return `H${H}/WT${WT}/VJ${VJ}/SPD${SPD}/LAT${LAT}/DUR${DUR}/AGE${AGE}`;
}

function bestFit(pd: PlayerData, roles: readonly RoleId[]): string {
  let best: { role: RoleId; fit: number } | null = null;
  for (const role of roles) {
    const fit = trueFit(pd, role);
    if (best === null || fit.value > best.fit) best = { role, fit: fit.value };
  }
  return best === null ? '?' : `${best.role} ${best.fit.toFixed(1)}`;
}

main();
