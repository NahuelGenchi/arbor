// 7a. Token accounting over the Claude Code transcripts in $TRANSCRIPTS_DIR. Only aggregates leave this process.
// One pass reproduces the four scripts behind the M values, each with its own counting rules:
//  A (cost shares, opening context, tool-result tokens, repeat reads): assistant calls deduplicated by message id
//    across all files; a tool result counts when its tool_use appeared earlier in the same file; o200k tokens of
//    its text parts.
//  B (sessions, context per call, opening-context share of cache reads, tool errors): the same call set as A.
//  C (cost per step, read-only follow-ups): calls deduplicated per file only.
//  D (screenshots): every image block in user content or tool results, sized by the image tiers' token formula.
// Weights: input 1, cache write 1.25, cache read 0.1, output 5.
const fs = require('fs'), path = require('path'), readline = require('readline');
const { count } = require(path.join(__dirname, '..', 'lib', 'o200k.js'));
const ROOT = process.env.TRANSCRIPTS_DIR || path.join(process.env.HOME, '.claude', 'projects');
const OUT = process.env.BENCH_RESULTS;
function emit(key, value, ctx) {
  const line = JSON.stringify(ctx ? { key, value, ctx } : { key, value });
  if (OUT) fs.appendFileSync(OUT, line + '\n');
  console.log(line);
}
function walk(d, out = []) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p, out); else if (e.name.endsWith('.jsonl')) out.push(p);
  }
  return out;
}

const RO_TOOLS = new Set(['Read', 'Grep', 'Glob', 'LS', 'WebFetch', 'WebSearch', 'mcp__Claude_Browser__get_page_text', 'mcp__Claude_Browser__read_page', 'mcp__Claude_Browser__find']);
const RO_BASH = /^(sed -n|cat|grep|rg|ls|head|tail|find|wc|git (log|show|diff|status|grep|ls-files|blame|branch|rev-parse)|jq|file|stat|du|tree|awk|sort|uniq|cut|readlink|realpath|which|type|echo \$|pwd|nl)\b/;
function isRO(t) {
  if (RO_TOOLS.has(t.name)) return true;
  if (t.name === 'Bash') {
    const c = (t.command || '').trim().replace(/^(cd [^;&|]+(&&|;)\s*)+/, '').replace(/^[A-Z_]+=\S+;?\s*/, '');
    return RO_BASH.test(c) && !/(>|\bsed -i\b|\brm\b|\bmv\b|\btee\b)/.test(c);
  }
  return false;
}
function resultText(c) {
  if (typeof c === 'string') return c;
  let t = '';
  if (Array.isArray(c)) for (const x of c) if (x.type === 'text') t += x.text;
  return t;
}
function dims(b64) {
  const b = Buffer.from(b64.slice(0, 400000), 'base64');
  if (b.toString('latin1', 1, 4) === 'PNG') return [b.readUInt32BE(16), b.readUInt32BE(20)];
  if (b[0] === 0xff && b[1] === 0xd8) {
    let i = 2;
    while (i < b.length - 9) {
      if (b[i] !== 0xff) { i++; continue; }
      const m = b[i + 1], L = b.readUInt16BE(i + 2);
      if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) return [b.readUInt16BE(i + 7), b.readUInt16BE(i + 5)];
      i += 2 + L;
    }
  }
  if (b.toString('latin1', 0, 4) === 'RIFF' && b.toString('latin1', 8, 12) === 'WEBP') {
    if (b.toString('latin1', 12, 16) === 'VP8X') return [1 + b.readUIntLE(24, 3), 1 + b.readUIntLE(27, 3)];
  }
  return null;
}
function imgTok(w, h, maxEdge, maxTok) {
  let s = Math.min(1, maxEdge / Math.max(w, h)), W = Math.round(w * s), H = Math.round(h * s);
  while (Math.ceil(W / 28) * Math.ceil(H / 28) > maxTok) { s *= 0.99; W = Math.round(w * s); H = Math.round(h * s); }
  return Math.ceil(W / 28) * Math.ceil(H / 28);
}

(async () => {
  const files = walk(ROOT);
  const seen = new Set();
  const U = { calls: 0, input: 0, cw: 0, cr: 0, out: 0 };
  const firstCall = [], sizes = [], callsPer = [], ctxs = [];
  let reads = 0, rereads = 0, resultCount = 0, allResTok = 0;
  let fixedReads = 0, totalReads = 0, sessions = 0, errN = 0, resN = 0;
  let calls3 = 0, toolCalls = 0, multi = 0, roCalls = 0, collapsible = 0, collapsibleUnits = 0, totalUnits = 0;
  let imgN = 0, imgUnk = 0, imgStd = 0, imgHi = 0;
  for (const f of files) {
    const pending = {}, readState = {}, msgs = new Map(), order = [];
    let first = null, callsHere = 0;
    const rl = readline.createInterface({ input: fs.createReadStream(f), crlfDelay: Infinity });
    for await (const line of rl) {
      let j; try { j = JSON.parse(line); } catch (e) { continue; }
      const m = j.message; if (!m) continue;
      if (j.type === 'assistant') {
        if (m.usage && m.id && !seen.has(m.id)) {  // A + B
          seen.add(m.id);
          const u = m.usage, cr = u.cache_read_input_tokens || 0;
          U.calls++; U.input += u.input_tokens || 0; U.cw += u.cache_creation_input_tokens || 0; U.cr += cr; U.out += u.output_tokens || 0;
          const ctx = (u.input_tokens || 0) + (u.cache_creation_input_tokens || 0) + cr;
          if (first === null) { first = ctx; firstCall.push(ctx); }
          callsHere++; totalReads += cr; fixedReads += Math.min(first, cr); ctxs.push(ctx);
        }
        if (Array.isArray(m.content)) for (const b of m.content) if (b.type === 'tool_use') pending[b.id] = { name: b.name, input: b.input || {} };
        if (m.id) {  // C
          if (!msgs.has(m.id)) { msgs.set(m.id, { u: m.usage, tools: [] }); order.push(m.id); }
          const e = msgs.get(m.id);
          if (Array.isArray(m.content)) for (const b of m.content) if (b.type === 'tool_use') e.tools.push({ name: b.name, command: b.name === 'Bash' ? (b.input || {}).command : undefined });
        }
      }
      if (j.type === 'user' && Array.isArray(m.content)) for (const b of m.content) {
        if (b.type !== 'tool_result') continue;
        resN++; if (b.is_error) errN++;  // B: every tool result
        const tu = pending[b.tool_use_id]; if (!tu) continue;  // A: results whose call is in this file
        const n = count(resultText(b.content));
        allResTok += n; resultCount++; sizes.push(n);
        const inp = tu.input;
        if (tu.name === 'Bash') {
          const cmd = (inp.command || '').trim().replace(/^cd [^;&|]+(&&|;)\s*/, '');
          if (/(>|sed -i|mv |cp |rm |git (checkout|reset|apply|merge|pull|stash|switch|rebase)|npm |cargo |make|patch|tee |python|node )/.test(cmd)) for (const k in readState) readState[k] = 'stale';
        }
        if (['Edit', 'Write', 'MultiEdit', 'NotebookEdit'].includes(tu.name) && inp.file_path) readState[inp.file_path] = 'stale';
        if (tu.name === 'Read' && inp.file_path) {
          reads++;
          const key = inp.file_path + '|' + (inp.offset || '') + '|' + (inp.limit || '');
          if (readState[key] === 'fresh') rereads++;
          readState[key] = 'fresh';
        }
        if (['Edit', 'Write', 'MultiEdit'].includes(tu.name) && inp.file_path) for (const k in readState) if (k.startsWith(inp.file_path + '|')) readState[k] = 'stale';
      }
      if (line.includes('"image"') && Array.isArray(m.content)) for (const b of m.content) {  // D
        const arr = b.type === 'tool_result' && Array.isArray(b.content) ? b.content : (b.type === 'image' ? [b] : []);
        for (const x of arr) {
          if (x.type !== 'image' || !x.source || !x.source.data) continue;
          imgN++;
          const d = dims(x.source.data);
          if (!d) { imgUnk++; continue; }
          imgStd += imgTok(d[0], d[1], 1568, 1568); imgHi += imgTok(d[0], d[1], 2576, 4784);
        }
      }
    }
    if (callsHere) { sessions++; callsPer.push(callsHere); }
    let streak = 0;  // C
    for (const id of order) {
      const e = msgs.get(id), u = e.u || {};
      calls3++;
      const units = (u.input_tokens || 0) + 1.25 * (u.cache_creation_input_tokens || 0) + 0.1 * (u.cache_read_input_tokens || 0) + 5 * (u.output_tokens || 0);
      totalUnits += units; toolCalls += e.tools.length; if (e.tools.length > 1) multi++;
      if (e.tools.length > 0 && e.tools.every(isRO)) { roCalls++; streak++; if (streak > 1) { collapsible++; collapsibleUnits += units; } } else streak = 0;
    }
  }
  const asc = (a) => a.sort((x, y) => x - y);
  asc(sizes); asc(firstCall); asc(callsPer); asc(ctxs);
  const at = (a, p) => a[Math.floor(a.length * p)];
  const corpus = { files: files.length, sessions, calls: U.calls };
  const totalIn = U.input + U.cw + U.cr;
  const units = U.input + 1.25 * U.cw + 0.1 * U.cr + 5 * U.out;
  emit('tok.sessions', sessions, { files: files.length });
  emit('tok.calls', calls3, { dedup_global: U.calls });
  emit('tok.cache_read_share_input', 100 * U.cr / totalIn, corpus);
  emit('tok.cost.cache_read', 100 * 0.1 * U.cr / units, corpus);
  emit('tok.cost.cache_write', 100 * 1.25 * U.cw / units, corpus);
  emit('tok.cost.output', 100 * 5 * U.out / units, corpus);
  emit('tok.cost.input', 100 * U.input / units, corpus);
  emit('tok.saving_vs_uncached', 100 * (1 - units / (totalIn + 5 * U.out)), corpus);
  emit('tok.opening_ctx_median', firstCall[firstCall.length >> 1], { p10: at(firstCall, 0.1), p90: at(firstCall, 0.9), sessions });
  emit('tok.opening_ctx_share_of_reads', 100 * fixedReads / totalReads, corpus);
  emit('tok.ctx_per_call_median', ctxs[ctxs.length >> 1], { p90: at(ctxs, 0.9), mean: Math.round(ctxs.reduce((a, b) => a + b, 0) / ctxs.length) });
  emit('tok.calls_per_session_median', callsPer[callsPer.length >> 1], { p90: at(callsPer, 0.9) });
  emit('tok.step_cost', totalUnits / calls3, { calls: calls3 });
  emit('tok.readonly_calls', 100 * roCalls / calls3, { tools_per_call: +(toolCalls / calls3).toFixed(2), parallel_pct: +(100 * multi / calls3).toFixed(1) });
  emit('tok.followup_reads_calls', 100 * collapsible / calls3, { calls: calls3 });
  emit('tok.followup_reads_cost', 100 * collapsibleUnits / totalUnits, { calls: calls3 });
  emit('tok.tool_results', resultCount, { o200k_tokens: allResTok });
  emit('tok.tool_result_median', at(sizes, 0.5), { results: resultCount });
  emit('tok.tool_result_p90', at(sizes, 0.9), { results: resultCount });
  emit('tok.tool_result_p99', at(sizes, 0.99), { results: resultCount });
  emit('tok.tool_errors', 100 * errN / resN, { results: resN });
  emit('tok.repeat_reads', 100 * rereads / reads, { reads, rereads });
  emit('tok.images', imgN, { unknown_dims: imgUnk });
  emit('tok.image_tokens_std', imgStd / 1e6, { images: imgN - imgUnk });
  emit('tok.image_tokens_hi', imgHi / 1e6, { images: imgN - imgUnk });
})();
