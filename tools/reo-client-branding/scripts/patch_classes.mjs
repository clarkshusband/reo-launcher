// Patch polyplus jar classes: OneClient -> ReoClient, ONECLIENT -> REOCLIENT
// Usage: node patch_classes.mjs [--apply]
import fs from 'node:fs';
import path from 'node:path';

const APPLY = process.argv.includes('--apply');
const work = 'C:/Users/drako/AppData/Local/Temp/opencode/reo_client_patch/work';

// ---------- class file parsing ----------
function readU2(b, o) { return b.readUInt16BE(o); }
function readU4(b, o) { return b.readUInt32BE(o); }

function parsePool(b) {
  const cpCount = readU2(b, 8);
  let o = 10;
  const entries = []; // index i corresponds to pool index i (entries[1] is first)
  entries.push(null);
  for (let i = 1; i < cpCount; i++) {
    const tag = b[o];
    const start = o;
    o += 1;
    let len = 0;
    let text = null;
    switch (tag) {
      case 1: { const l = readU2(b, o); text = b.toString('latin1', o + 2, o + 2 + l); len = 2 + l; break; }
      case 3: case 4: len = 4; break;
      case 5: case 6: len = 8; break;
      case 7: case 8: case 16: case 19: case 20: len = 2; break;
      case 9: case 10: case 11: case 12: case 17: case 18: len = 4; break;
      case 15: len = 3; break;
      default: throw new Error(`unknown tag ${tag} at ${start} in ${b.length} bytes`);
    }
    o += len;
    entries.push({ tag, start, end: o, text });
    if (tag === 5 || tag === 6) { entries.push(null); i++; }
  }
  return { cpCount, entries, poolEnd: o };
}

function fullValidate(b, parsed) {
  // walk the whole class structure to EOF
  let o = parsed.poolEnd;
  const cp = parsed.entries;
  const u2 = () => { const v = readU2(b, o); o += 2; return v; };
  const u4 = () => { const v = readU4(b, o); o += 4; return v; };
  const checkIdx = (i, what) => { if (i < 1 || i >= parsed.cpCount || !cp[i]) throw new Error(`bad ${what} index ${i}`); };
  const attrs = () => {
    const n = u2();
    for (let i = 0; i < n; i++) {
      const nameIdx = u2(); checkIdx(nameIdx, 'attribute name'); if (cp[nameIdx].tag !== 1) throw new Error('attr name not utf8');
      const len = u4(); o += len;
      if (o > b.length) throw new Error('attribute overruns file');
    }
  };
  u2(); // access flags
  const thisIdx = u2(); checkIdx(thisIdx, 'this_class'); if (cp[thisIdx].tag !== 7) throw new Error('this_class not Class');
  const superIdx = u2(); checkIdx(superIdx, 'super_class'); if (cp[superIdx].tag !== 7) throw new Error('super_class not Class');
  const ifc = u2();
  for (let i = 0; i < ifc; i++) { const x = u2(); checkIdx(x, 'interface'); }
  const fields = u2();
  for (let i = 0; i < fields; i++) { u2(); u2(); u2(); attrs(); }
  const methods = u2();
  for (let i = 0; i < methods; i++) { u2(); u2(); u2(); attrs(); }
  attrs();
  if (o !== b.length) throw new Error(`trailing bytes: parsed to ${o} of ${b.length}`);
  return { bytes: b.length, fields, methods };
}

function rebuild(b, parsed, changes) {
  const parts = [];
  parts.push(b.subarray(0, 10));
  for (let i = 1; i < parsed.cpCount; i++) {
    const e = parsed.entries[i];
    if (!e) continue;
    const change = changes.get(e);
    if (change !== undefined) {
      const payload = Buffer.from(change, 'latin1');
      const head = Buffer.alloc(3);
      head[0] = 1;
      head.writeUInt16BE(payload.length, 1);
      parts.push(head, payload);
    } else {
      parts.push(b.subarray(e.start, e.end));
    }
  }
  parts.push(b.subarray(parsed.poolEnd));
  return Buffer.concat(parts);
}

// ---------- replacement policy ----------
const identifierLike = /^[A-Za-z0-9_$]+$/;

function plan(text, fileName) {
  // wordmark, exact match
  if (text === 'ONECLIENT') return 'REOCLIENT';
  if (!text.includes('OneClient')) return null;
  if (text.includes('://') || text.startsWith('http')) return null;
  const prose = /[\s<>!?,;:'"]/.test(text) || text === 'OneClient';
  if (!prose) return null;
  if (identifierLike.test(text)) return null;
  return text.split('OneClient').join('ReoClient');
}

// ---------- walk ----------
const classFiles = [];
(function walk(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p);
    else if (e.name.endsWith('.class')) classFiles.push(p);
  }
})(work);

let changedFiles = 0, changedEntries = 0;
const skipped = [];
for (const file of classFiles) {
  const original = fs.readFileSync(file);
  if (original[0] !== 0xca || original[1] !== 0xfe || original[2] !== 0xba || original[3] !== 0xbe) continue;
  const parsed = parsePool(original);
  const changes = new Map();
  const rel = path.relative(work, file);
  for (const e of parsed.entries) {
    if (!e || e.tag !== 1) continue;
    const out = plan(e.text, rel);
    if (out !== null && out !== e.text) {
      changes.set(e, out);
      if (!APPLY) console.log(`${rel}\n    "${e.text}"\n -> "${out}"`);
    } else if (e.text.includes('OneClient') || e.text.includes('ONECLIENT')) {
      skipped.push({ rel, text: e.text });
    }
  }
  if (changes.size > 0) {
    changedFiles++;
    changedEntries += changes.size;
    if (APPLY) {
      const rebuilt = rebuild(original, parsed, changes);
      // validate the rebuilt file end-to-end
      const reparsed = parsePool(rebuilt);
      fullValidate(rebuilt, reparsed);
      fs.writeFileSync(file, rebuilt);
    } else {
      const rebuilt = rebuild(original, parsed, changes);
      const reparsed = parsePool(rebuilt);
      fullValidate(rebuilt, reparsed);
    }
  } else {
    // still ensure the unmodified file parses structurally (catch any oddity early)
    try { fullValidate(original, parsed); } catch (err) { console.log(`!! unmodified file fails full validation: ${rel}: ${err.message}`); }
  }
}

console.log('');
console.log(`classes scanned: ${classFiles.length}`);
console.log(`${APPLY ? 'applied' : 'planned'}: ${changedEntries} entries in ${changedFiles} files`);
if (skipped.length) {
  console.log('');
  console.log('--- entries containing OneClient/ONECLIENT left untouched ---');
  const seen = new Set();
  for (const s of skipped) {
    const k = s.rel + '|' + s.text;
    if (seen.has(k)) continue;
    seen.add(k);
    console.log(`${s.rel}\n    "${s.text}"`);
  }
}
