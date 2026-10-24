// Rewrites a fabric.mod.json: sets the display name, converts OneClient -> ReoClient
// in the description, removes contact links and Mod Menu links (keeps everything else).
// usage: node edit_fabric_mod.js <in.json> <out.json> [newName]
const fs = require('fs');

const [, , inPath, outPath, newName] = process.argv;
if (!inPath || !outPath) {
  console.error('usage: node edit_fabric_mod.js <in.json> <out.json> [newName]');
  process.exit(2);
}

const j = JSON.parse(fs.readFileSync(inPath, 'utf8'));

if (newName) j.name = newName;

if (typeof j.description === 'string') {
  j.description = j.description
    .split('OneClient').join('ReoClient')
    .split('Polyfrost').join('Reo');
}

delete j.contact;

if (j.custom && typeof j.custom === 'object' && j.custom.modmenu && typeof j.custom.modmenu === 'object') {
  delete j.custom.modmenu.links;
}

fs.writeFileSync(outPath, JSON.stringify(j, null, 2) + '\n', 'utf8');
