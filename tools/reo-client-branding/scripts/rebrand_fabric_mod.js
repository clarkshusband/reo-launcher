// Rewrites a fabric.mod.json for the ReoMods distribution builds:
//   * optional display-name override (--name)
//   * authors -> drako (--authors)
//   * description: OneClient -> ReoClient, Polyfrost -> Reo
//   * contact policy: remove (default) | discord (Reo Discord invite) | keep
//   * always removes custom.modmenu.links
//
// usage:
//   node rebrand_fabric_mod.js <in.json> <out.json> [--name=ReoBlur] [--authors=drako] [--contact=remove|discord|keep]
const fs = require('fs');

const [, , inPath, outPath, ...flags] = process.argv;
if (!inPath || !outPath) {
  console.error('usage: node rebrand_fabric_mod.js <in.json> <out.json> [--name=X] [--authors=a,b] [--contact=remove|discord|keep]');
  process.exit(2);
}

const opt = {};
for (const f of flags) {
  const m = /^--([^=]+)=([\s\S]*)$/.exec(f);
  if (m) opt[m[1]] = m[2];
}

const j = JSON.parse(fs.readFileSync(inPath, 'utf8'));

if (opt.name) j.name = opt.name;
if (opt.authors) j.authors = opt.authors.split(',').map((s) => s.trim()).filter(Boolean);

if (typeof j.description === 'string') {
  j.description = j.description
    .split('OneClient').join('ReoClient')
    .split('Polyfrost').join('Reo');
}

const contact = opt.contact || 'remove';
if (contact === 'remove') {
  delete j.contact;
} else if (contact === 'discord') {
  j.contact = { homepage: 'https://discord.gg/reomc' };
} else if (contact !== 'keep') {
  console.error(`unknown --contact=${contact}`);
  process.exit(2);
}

if (j.custom && typeof j.custom === 'object' && j.custom.modmenu && typeof j.custom.modmenu === 'object') {
  delete j.custom.modmenu.links;
}

fs.writeFileSync(outPath, JSON.stringify(j, null, 2) + '\n', 'utf8');
