#!/usr/bin/env node
// scripts/check-foundation.mjs
//
// Wave 0 quality gate for the nba-sim foundation. Validates that every
// foundation config is internally consistent: each JSON file conforms to its
// schema (via ajv-cli) AND the closed-set / cross-file invariants that JSON
// Schema draft-07 cannot express (uniqueness, cross-file references, set
// disjointness) hold.
//
// Pure Node.js ESM. Only Node built-ins are used (fs, path, child_process);
// schema checks shell out to `npx ajv-cli@latest`.
//
// Run from the repo root:  node scripts/check-foundation.mjs
// Exit 0 = all 21 checks pass; exit 1 = at least one check failed.

import { readFileSync, readdirSync, statSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');
const CONFIG = resolve(ROOT, 'config');
const SCHEMAS = resolve(CONFIG, 'schemas');

const TOTAL_CHECKS = 24;
let passed = 0;

function record(label, ok, detail = '') {
  const status = ok ? '[PASS]' : '[FAIL]';
  const line = detail ? `${status} ${label}: ${detail}` : `${status} ${label}`;
  if (ok) passed += 1;
  console.log(line);
}

function readJSON(file) {
  return JSON.parse(readFileSync(file, 'utf8'));
}

// Run an ajv-cli subcommand with stdio piped so ajv output does not clutter the
// summary. Returns { ok, message } where message is the first line of whatever
// ajv printed (stderr first, then stdout) on failure.
function runAjv(args) {
  try {
    execSync(`npx --yes ajv-cli@latest ${args}`, {
      stdio: 'pipe',
      cwd: ROOT,
    });
    return { ok: true, message: '' };
  } catch (e) {
    const stderr = (e.stderr ?? '').toString().trim();
    const stdout = (e.stdout ?? '').toString().trim();
    const message = (stderr || stdout || String(e.message)).split('\n')[0];
    return { ok: false, message };
  }
}

// ---------------------------------------------------------------------------
// Checks 1-7: ajv validate (schema <- data)
// ---------------------------------------------------------------------------

const validatePairs = [
  ['event-catalog.schema.json', 'event-catalog.json'],
  ['rng.schema.json', 'rng.json'],
  ['duration.schema.json', 'duration.json'],
  ['court-zones.schema.json', 'court-zones.json'],
  ['fsm.schema.json', 'fsm.json'],
  ['plays.schema.json', 'plays.json'],
  ['resolve.schema.json', 'resolve.json'],
];

for (const [schemaFile, dataFile] of validatePairs) {
  const schemaPath = resolve(SCHEMAS, schemaFile);
  const dataPath = resolve(CONFIG, dataFile);
  const res = runAjv(`validate -s "${schemaPath}" -d "${dataPath}"`);
  record(`ajv: ${dataFile}`, res.ok, res.message);
}

// ---------------------------------------------------------------------------
// Checks 8-19: cross-reference (pure Node logic)
// ---------------------------------------------------------------------------

const foundation = readJSON(resolve(CONFIG, 'foundation.json'));
const catalog = readJSON(resolve(CONFIG, 'event-catalog.json'));
const fsm = readJSON(resolve(CONFIG, 'fsm.json'));
const fsmSchema = readJSON(resolve(SCHEMAS, 'fsm.schema.json'));
const plays = readJSON(resolve(CONFIG, 'plays.json'));
const durationCfg = readJSON(resolve(CONFIG, 'duration.json'));
const zones = readJSON(resolve(CONFIG, 'court-zones.json'));

// 8. Event type uniqueness (closed catalog pinned to 40 in foundation 0.2.0).
{
  const types = catalog.events.map((e) => e.type);
  const unique = new Set(types);
  const dupes = [...new Set(types.filter((t, i) => types.indexOf(t) !== i))];
  const ok = types.length === unique.size && types.length === 40;
  const detail = ok
    ? ''
    : `count=${types.length} unique=${unique.size}${
        dupes.length ? ` duplicates=${JSON.stringify(dupes)}` : ''
      }`;
  record('xref: event type uniqueness (40 unique)', ok, detail);
}

// 9. Every FSM emit_events name exists in the event catalog.
{
  const catalogTypes = new Set(catalog.events.map((e) => e.type));
  const missing = new Set();
  for (const t of fsm.transitions) {
    for (const ev of t.emit_events ?? []) {
      if (!catalogTypes.has(ev)) missing.add(ev);
    }
  }
  const ok = missing.size === 0;
  record(
    'xref: FSM emit_events ⊆ catalog',
    ok,
    ok ? '' : `missing=${JSON.stringify([...missing].sort())}`,
  );
}

// 10. FSM phases array matches the schema's phase enum (if present).
{
  const enumNode = fsmSchema?.$defs?.phaseName?.enum;
  let ok;
  let detail;
  if (!Array.isArray(enumNode)) {
    ok = true;
    detail = 'no phase enum in schema';
  } else {
    const phasesSet = new Set(fsm.phases);
    const enumSet = new Set(enumNode);
    const dataOnly = fsm.phases.filter((p) => !enumSet.has(p));
    const enumOnly = enumNode.filter((p) => !phasesSet.has(p));
    ok = dataOnly.length === 0 && enumOnly.length === 0;
    detail = ok
      ? ''
      : `data_only=${JSON.stringify(dataOnly)} enum_only=${JSON.stringify(enumOnly)}`;
  }
  record('xref: FSM phases == schema phase enum', ok, detail);
}

// 11. No two transitions share the same (from, trigger) pair.
{
  const seen = new Set();
  const dupes = [];
  for (const t of fsm.transitions) {
    const key = `${t.from}\0${t.trigger}`;
    if (seen.has(key)) dupes.push(`${t.from} + ${t.trigger}`);
    else seen.add(key);
  }
  const ok = dupes.length === 0;
  record(
    'xref: FSM (from, trigger) pairs unique',
    ok,
    ok ? '' : `duplicates=${JSON.stringify(dupes)}`,
  );
}

// 12. Substitution legal_phases and illegal_phases share no elements.
{
  const legal = new Set(fsm.substitution_legality.legal_phases);
  const illegal = new Set(fsm.substitution_legality.illegal_phases);
  const intersection = [...legal].filter((p) => illegal.has(p));
  const ok = intersection.length === 0;
  record(
    'xref: substitution legal ∩ illegal = ∅',
    ok,
    ok ? '' : `intersection=${JSON.stringify(intersection)}`,
  );
}

// 13. legal ∪ illegal == phases (every phase is classified exactly once).
{
  const legal = new Set(fsm.substitution_legality.legal_phases);
  const illegal = new Set(fsm.substitution_legality.illegal_phases);
  const phases = new Set(fsm.phases);
  const union = new Set([...legal, ...illegal]);
  const missing = [...phases].filter((p) => !union.has(p));
  const extra = [...union].filter((p) => !phases.has(p));
  const ok = missing.length === 0 && extra.length === 0;
  record(
    'xref: substitution legal ∪ illegal = phases',
    ok,
    ok ? '' : `missing=${JSON.stringify(missing)} extra=${JSON.stringify(extra)}`,
  );
}

// 14. Every play step duration_id exists in the duration catalog.
{
  const durationIds = new Set(durationCfg.durations.map((d) => d.id));
  const missing = new Set();
  for (const play of plays.plays) {
    for (const step of play.steps ?? []) {
      if (step.duration_id && !durationIds.has(step.duration_id)) {
        missing.add(step.duration_id);
      }
    }
  }
  const ok = missing.size === 0;
  record(
    'xref: plays duration_ids ⊆ duration catalog',
    ok,
    ok ? '' : `missing=${JSON.stringify([...missing].sort())}`,
  );
}

// 15. Every play slot name is one of the five known offense role slots.
{
  const allowed = new Set([
    'primary_creator',
    'secondary_creator',
    'screener',
    'spacer_strong',
    'spacer_weak',
  ]);
  const invalid = new Set();
  for (const play of plays.plays) {
    for (const slot of play.slots ?? []) {
      if (!allowed.has(slot)) invalid.add(slot);
    }
  }
  const ok = invalid.size === 0;
  record(
    'xref: plays slot names valid',
    ok,
    ok ? '' : `invalid=${JSON.stringify([...invalid].sort())}`,
  );
}

// 16. Every play mode is declared in the top-level modes array.
{
  const allowed = new Set(plays.modes);
  const invalid = new Set();
  for (const play of plays.plays) {
    if (!allowed.has(play.mode)) invalid.add(play.mode);
  }
  const ok = invalid.size === 0;
  record(
    'xref: plays modes ⊆ allowed',
    ok,
    ok ? '' : `invalid=${JSON.stringify([...invalid].sort())}`,
  );
}

// 17. At least one play exists per declared mode.
{
  const counts = Object.fromEntries(plays.modes.map((m) => [m, 0]));
  for (const play of plays.plays) {
    if (Object.prototype.hasOwnProperty.call(counts, play.mode)) {
      counts[play.mode] += 1;
    }
  }
  const empty = plays.modes.filter((m) => counts[m] === 0);
  const ok = empty.length === 0;
  record(
    'xref: ≥1 play per mode',
    ok,
    ok ? '' : `empty_modes=${JSON.stringify(empty)}`,
  );
}

// 18. Court zones are a closed set: all match ^[a-zA-Z_]+$ and count == 14.
{
  const re = /^[a-zA-Z_]+$/;
  const list = zones.zones;
  const invalid = list.filter((z) => !re.test(z));
  const ok = list.length === 14 && invalid.length === 0;
  let detail = '';
  if (!ok) {
    const parts = [];
    if (list.length !== 14) parts.push(`count=${list.length}`);
    if (invalid.length) parts.push(`invalid=${JSON.stringify(invalid)}`);
    detail = parts.join(' ');
  }
  record('xref: court zones closed set (14, ^[a-zA-Z_]+$)', ok, detail);
}

// 19. Every config data file carries the same foundation_version as foundation.json.
{
  const expected = foundation.foundation_version;
  const dataFiles = ['foundation.json', ...validatePairs.map(([, d]) => d)];
  const mismatches = [];
  for (const f of dataFiles) {
    const data = readJSON(resolve(CONFIG, f));
    if (data.foundation_version !== expected) {
      mismatches.push(`${f}=${JSON.stringify(data.foundation_version)}`);
    }
  }
  const ok = mismatches.length === 0;
  record(
    'xref: foundation_version consistency',
    ok,
    ok
      ? ''
      : `expected=${JSON.stringify(expected)} mismatches=[${mismatches.join(', ')}]`,
  );
}

// ---------------------------------------------------------------------------
// Checks 20-21: ajv compile (schema self-consistency under strict mode)
// ---------------------------------------------------------------------------

for (const schemaFile of [
  'lineup-package.schema.json',
  'game-result.schema.json',
]) {
  const schemaPath = resolve(SCHEMAS, schemaFile);
  const res = runAjv(`compile -s "${schemaPath}"`);
  record(`ajv compile: ${schemaFile}`, res.ok, res.message);
}

// ---------------------------------------------------------------------------
// Checks 22-24: single-axis import bans (architecture.md §3)
// ---------------------------------------------------------------------------

function listTsFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = resolve(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) out.push(...listTsFiles(full));
    else if (name.endsWith('.ts')) out.push(full);
  }
  return out;
}

function fileImports(src) {
  const text = readFileSync(src, 'utf8');
  const re = /from\s+['"]([^'"]+)['"]/g;
  const mods = [];
  let m;
  while ((m = re.exec(text)) !== null) mods.push(m[1]);
  return mods;
}

{
  const decisionDir = resolve(ROOT, 'src/decision');
  const banned = ['../adjudicate/', '../state/apply', '../state/handlers', '../duration/sampler', 'sampleDuration'];
  const offenders = [];
  for (const f of listTsFiles(decisionDir)) {
    const text = readFileSync(f, 'utf8');
    // resolve/ is allowed since foundation 0.9.0: the EV decision core
    // PRICES shots/drives/passes with the resolve rate model (one-way
    // dependency, resolve never imports decision). geometry resolveSlotTarget
    // is court/relations — also allowed.
    if (/from\s+['"][^'"]*adjudicate\//.test(text)) offenders.push(f.replace(ROOT + '/', '') + ':adjudicate');
    if (/from\s+['"][^'"]*duration\/sampler/.test(text) || /sampleDuration/.test(text)) {
      offenders.push(f.replace(ROOT + '/', '') + ':sampleDuration');
    }
    if (/from\s+['"][^'"]*state\/(apply|handlers)/.test(text)) {
      offenders.push(f.replace(ROOT + '/', '') + ':state-mutate');
    }
  }
  const ok = offenders.length === 0;
  record('arch: decision/ import bans', ok, ok ? '' : offenders.join('; '));
}

{
  const completionDir = resolve(ROOT, 'src/completion');
  const offenders = [];
  if (statSync(completionDir, { throwIfNoEntry: false })?.isDirectory()) {
    for (const f of listTsFiles(completionDir)) {
      const text = readFileSync(f, 'utf8');
      if (/from\s+['"][^'"]*resolve\//.test(text)) offenders.push(f.replace(ROOT + '/', '') + ':resolve');
      if (/from\s+['"][^'"]*adjudicate\//.test(text)) offenders.push(f.replace(ROOT + '/', '') + ':adjudicate');
      if (/rng\.next|sampleDuration/.test(text)) offenders.push(f.replace(ROOT + '/', '') + ':rng-or-duration');
    }
  }
  const ok = offenders.length === 0;
  record('arch: completion/ purity bans', ok, ok ? '' : offenders.join('; '));
}

{
  // adjudicate is the only module under src/ that may import resolve checks
  // (plus sim-tick FT path temporarily and resolve itself). The ball-motion
  // invariant documentation may mention the word "resolve", but it performs
  // no outcome resolution and must not be classified as a resolve site.
  const srcDir = resolve(ROOT, 'src');
  const offenders = [];
  for (const f of listTsFiles(srcDir)) {
    const rel = f.replace(ROOT + '/', '');
    if (rel.startsWith('src/resolve/')) continue;
    if (rel.startsWith('src/adjudicate/')) continue;
    if (rel === 'src/sim-tick.ts') continue;
    const text = readFileSync(f, 'utf8');
    const code = text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
    if (/from\s+['"][^'"]*resolve\/(index|checks)/.test(code) || /resolveShot|resolvePass|resolveSteal|resolveRebound|resolveDrive/.test(code)) {
      if (/import\s+type/.test(code) && !/resolveShot|resolvePass|resolveSteal|resolveRebound|resolveDrive/.test(code)) continue;
      if (/resolveShot|resolvePass|resolveSteal|resolveRebound|resolveDrive|resolveHandoff|resolveFt/.test(code)) {
        offenders.push(rel);
      } else if (/from\s+['"].*resolve\//.test(code)) {
        offenders.push(rel);
      }
    }
  }
  const ok = offenders.length === 0;
  record('arch: sole resolve site is adjudicate/', ok, ok ? '' : offenders.join('; '));
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

if (passed === TOTAL_CHECKS) {
  console.log(`Foundation check: ALL PASS (${passed}/${TOTAL_CHECKS})`);
  process.exit(0);
} else {
  console.log(`Foundation check: FAILED (${passed}/${TOTAL_CHECKS})`);
  process.exit(1);
}
