/**
 * Regenerate spectator/court-draw-spec.json from src/court/geometry.ts.
 *
 * The web shell fetches this standalone file (browser has no TS import),
 * so any change to the court geometry (config/court-geometry.json or the
 * draw-spec builder) must be followed by:
 *   npx tsx scripts/regen-court-spec.ts
 */
import { writeFileSync } from 'node:fs';
import { courtDrawSpecJson } from '../src/court/geometry.js';

const spec = courtDrawSpecJson();
writeFileSync('spectator/court-draw-spec.json', `${JSON.stringify(spec)}\n`);
const c = spec.left;
console.log('corner segs (left):', JSON.stringify(c.threeCorners));
console.log('arc[0] (left):', JSON.stringify(c.threeArc[0]));
console.log('arc[last] (left):', JSON.stringify(c.threeArc[c.threeArc.length - 1]));
console.log('arc pts:', c.threeArc.length, 'hash marks:', c.laneHashMarks.length);
console.log('corner segs (right):', JSON.stringify(spec.right.threeCorners));
console.log('arc[0] (right):', JSON.stringify(spec.right.threeArc[0]));
