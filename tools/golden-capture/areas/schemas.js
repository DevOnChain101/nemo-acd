'use strict';
const { ref } = require('../env');
const path = require('path');
const t = require(path.join(ref, 'src/native/tools/index.js'));

const source = 'src/native/tools/index.js';
const schemas = t.toolSchemas(t.builtinTools());
const out = [];
// per-tool cases carry the tool name; the order case pins registry order and the agent-loop-only schemas
out.push({ id: 'registry-order-and-names', input: { op: 'names' } });
for (const s of schemas) out.push({ id: `builtin-${s.name.toLowerCase()}`, input: { op: 'builtin', name: s.name }, source: 'toolSchemas(builtinTools()) in src/native/tools/index.js', ...(s.name === 'Bash' ? { note: 'Bash description text may be platform-specific; verify on win32/linux before treating as 3-OS spec' } : {}) });
for (const s of t.agentToolSchemas) out.push({ id: `agent-${s.name.toLowerCase()}`, input: { op: 'agent', name: s.name }, source: 'agentToolSchemas in src/native/tools/index.js' });

exports.source = source;
exports.cases = () => out;
exports.run = async (i) => {
  if (i.op === 'names') return { builtin: t.builtinTools().map((x) => x.name), agent: t.agentToolSchemas.map((x) => x.name), schema_keys: Object.keys(schemas[0]).sort() };
  const list = i.op === 'agent' ? t.agentToolSchemas : t.toolSchemas(t.builtinTools());
  return list.find((x) => x.name === i.name);
};
