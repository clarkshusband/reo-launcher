// Trace the alpha silhouette of reo-icon.png into polygon loops (SVG-ready).
// Input: assets/mask.bin (150x118, 1 byte per pixel, 1 = foreground)
// Output: assets/loops.json { w, h, loops: [ [ [x,y], ... ], ... ] }
import fs from 'node:fs';

const dir = 'C:/Users/drako/AppData/Local/Temp/opencode/reo_client_patch/assets';
const W = 150, H = 118;
const mask = fs.readFileSync(`${dir}/mask.bin`);
const fg = (x, y) => x >= 0 && y >= 0 && x < W && y < H && mask[y * W + x] === 1;

// directed boundary edges: walk so foreground is on the right of the travel direction
const edges = [];
for (let y = 0; y < H; y++) {
  for (let x = 0; x < W; x++) {
    if (!fg(x, y)) continue;
    if (!fg(x, y - 1)) edges.push({ f: [x, y], t: [x + 1, y], d: [1, 0] });
    if (!fg(x + 1, y)) edges.push({ f: [x + 1, y], t: [x + 1, y + 1], d: [0, 1] });
    if (!fg(x, y + 1)) edges.push({ f: [x + 1, y + 1], t: [x, y + 1], d: [-1, 0] });
    if (!fg(x - 1, y)) edges.push({ f: [x, y + 1], t: [x, y], d: [0, -1] });
  }
}

const key = (x, y) => y * 1024 + x;
const out = new Map();
edges.forEach((e, i) => {
  const k = key(e.f[0], e.f[1]);
  if (!out.has(k)) out.set(k, []);
  out.get(k).push(i);
});

// turn preference per incoming direction (screen coords, y down): right, straight, left, back
const order = {
  '1,0': [[0, 1], [1, 0], [0, -1], [-1, 0]],
  '0,1': [[-1, 0], [0, 1], [1, 0], [0, -1]],
  '-1,0': [[0, -1], [-1, 0], [0, 1], [1, 0]],
  '0,-1': [[1, 0], [0, -1], [-1, 0], [0, 1]],
};
const rankMap = new Map();
for (const [k, arr] of Object.entries(order)) rankMap.set(k, new Map(arr.map((d, i) => [d.join(','), i])));

const used = new Array(edges.length).fill(false);
function pickNext(vx, vy, din) {
  const cands = out.get(key(vx, vy)) || [];
  const rank = rankMap.get(din.join(','));
  let best = -1, bestRank = 99;
  for (const i of cands) {
    if (used[i]) continue;
    const r = rank.get(edges[i].d.join(','));
    if (r < bestRank) { bestRank = r; best = i; }
  }
  return best;
}

const raw = [];
for (let s = 0; s < edges.length; s++) {
  if (used[s]) continue;
  const pts = [];
  let cur = s;
  while (cur !== -1 && !used[cur]) {
    used[cur] = true;
    pts.push(edges[cur].f);
    const e = edges[cur];
    cur = pickNext(e.t[0], e.t[1], e.d);
  }
  if (pts.length >= 4) raw.push(pts);
}

// closed-loop simplification: rotate so the loop starts at the farthest pair point, then RDP
function rdp(pts, eps) {
  if (pts.length < 3) return pts.slice();
  const n = pts.length;
  let best = -1, ia = 0, ib = 0;
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const d = (pts[i][0] - pts[j][0]) ** 2 + (pts[i][1] - pts[j][1]) ** 2;
      if (d > best) { best = d; ia = i; ib = j; }
    }
  }
  const rot = pts.slice(ia).concat(pts.slice(0, ia));
  // split into two open chains: ia->ib and ib->end(ia)
  const idxB = (ib - ia + n) % n;
  const chain1 = rot.slice(0, idxB + 1);
  const chain2 = rot.slice(idxB).concat([rot[0]]);

  function simplify(chain) {
    if (chain.length < 3) return chain.slice();
    const a = chain[0], b = chain[chain.length - 1];
    const dx = b[0] - a[0], dy = b[1] - a[1];
    const len = Math.hypot(dx, dy) || 1e-9;
    let maxD = -1, idx = -1;
    for (let i = 1; i < chain.length - 1; i++) {
      const p = chain[i];
      const dist = Math.abs(dy * p[0] - dx * p[1] + b[0] * a[1] - b[1] * a[0]) / len;
      if (dist > maxD) { maxD = dist; idx = i; }
    }
    if (maxD <= eps) return [a, b];
    const left = simplify(chain.slice(0, idx + 1));
    const right = simplify(chain.slice(idx));
    return left.slice(0, -1).concat(right);
  }

  const s1 = simplify(chain1);
  const s2 = simplify(chain2);
  const merged = s1.concat(s2.slice(1, -1));
  return merged;
}

function area(pts) {
  let a = 0;
  for (let i = 0; i < pts.length; i++) {
    const p = pts[i], q = pts[(i + 1) % pts.length];
    a += p[0] * q[1] - q[0] * p[1];
  }
  return a / 2;
}

const loops = [];
for (const pts of raw) {
  const a = Math.abs(area(pts));
  if (a < 2.5) continue;
  const simp = rdp(pts, 0.42);
  // drop consecutive duplicates
  const clean = [];
  for (const p of simp) {
    const last = clean[clean.length - 1];
    if (!last || last[0] !== p[0] || last[1] !== p[1]) clean.push(p);
  }
  if (clean.length >= 3) loops.push(clean);
}

fs.writeFileSync(`${dir}/loops.json`, JSON.stringify({ w: W, h: H, loops }));
const pts = loops.reduce((n, l) => n + l.length, 0);
console.log(`loops: ${loops.length}  total points: ${pts}`);
console.log('loop sizes:', loops.map(l => l.length).join(', '));
console.log('areas:', loops.map(l => area(l).toFixed(1)).join(', '));
