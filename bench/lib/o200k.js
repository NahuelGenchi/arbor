// Minimal o200k_base token counter (tiktoken-compatible pre-split regex + byte-level BPE).
// Ranks: $O200K_FILE, by default the copy the VS Code Copilot extension ships, stored as [len][bytes] records in
// rank order (record i has rank i).
// CLI: node o200k.js FILE            -> token count of the file's text
//      node o200k.js --json FILE     -> {"name": count} for a JSON object of {"name": text}
const fs = require('fs');
const RANKS = process.env.O200K_FILE || '/usr/share/code/resources/app/extensions/copilot/dist/o200k_base.tiktoken';
const ranks = new Map();
{
  const buf = fs.readFileSync(RANKS);
  let i = 0, r = 0;
  while (i < buf.length) { const L = buf[i]; ranks.set(buf.subarray(i + 1, i + 1 + L).toString('latin1'), r++); i += 1 + L; }
}
const C = "(?:'[sS]|'[tT]|'[rR][eE]|'[vV][eE]|'[mM]|'[lL][lL]|'[dD])?";
const PAT = new RegExp([
  "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]*[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]+" + C,
  "[^\\r\\n\\p{L}\\p{N}]?[\\p{Lu}\\p{Lt}\\p{Lm}\\p{Lo}\\p{M}]+[\\p{Ll}\\p{Lm}\\p{Lo}\\p{M}]*" + C,
  "\\p{N}{1,3}", " ?[^\\s\\p{L}\\p{N}]+[\\r\\n/]*", "\\s*[\\r\\n]+", "\\s+(?!\\S)", "\\s+"].join('|'), 'gu');
const cache = new Map();
function bpeCount(piece) {
  if (ranks.has(piece)) return 1;
  if (cache.has(piece)) return cache.get(piece);
  const toks = [];
  for (let i = 0; i < piece.length; i++) toks.push(piece[i]);
  while (toks.length > 1) {
    let best = Infinity, bi = -1;
    for (let i = 0; i < toks.length - 1; i++) { const r = ranks.get(toks[i] + toks[i + 1]); if (r !== undefined && r < best) { best = r; bi = i; } }
    if (bi < 0) break;
    toks.splice(bi, 2, toks[bi] + toks[bi + 1]);
  }
  if (cache.size < 200000) cache.set(piece, toks.length);
  return toks.length;
}
function count(text) {
  let n = 0;
  for (const m of text.matchAll(PAT)) n += bpeCount(Buffer.from(m[0], 'utf8').toString('latin1'));
  return n;
}
module.exports = { count, vocab: ranks.size };
if (require.main === module) {
  if (process.argv[2] === '--json') {
    const d = JSON.parse(fs.readFileSync(process.argv[3], 'utf8'));
    const out = {};
    for (const [k, v] of Object.entries(d)) out[k] = count(v);
    console.log(JSON.stringify(out));
  } else {
    console.log(count(process.argv[2] ? fs.readFileSync(process.argv[2], 'utf8') : fs.readFileSync(0, 'utf8')));
  }
}
