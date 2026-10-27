'use strict';
const { ref } = require('../env');
const path = require('path');
const anth = require(path.join(ref, 'src/native/providers/anthropic.js'));
const oai = require(path.join(ref, 'src/native/providers/openai.js'));
const norm = require(path.join(ref, 'src/native/providers/modelnorm.js'));
const index = require(path.join(ref, 'src/native/providers/index.js'));

const source = 'src/native/providers/*.js';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });
const P = (provider, id, req, chunks, extra) => add(`${provider}-${id}`, { op: 'stream', provider, req, chunks }, extra);

const TOOLS = [{ name: 'Bash', description: 'Run a command', input_schema: { type: 'object', properties: { command: { type: 'string' } }, required: ['command'] }, extraIgnored: 1 }, { name: 'Read', description: 'Read', input_schema: { type: 'object', properties: {} } }];
const HIST = [
  { role: 'user', content: [{ type: 'text', text: 'run ls' }] },
  { role: 'assistant', content: [{ type: 'thinking', thinking: 'secret reasoning' }, { type: 'text', text: 'Sure. ' }, { type: 'tool_use', id: 'call_1', name: 'Bash', input: { command: 'ls' } }] },
  { role: 'user', content: [{ type: 'tool_result', tool_use_id: 'call_1', content: 'file.txt' }, { type: 'text', text: 'now what?' }] },
];

// ---------------- request mapping: normalized req -> wire body (events from an empty/minimal stream) ----------------
const MIN_A = [{ type: 'message_start', message: { id: 'msg_1', model: 'm', usage: { input_tokens: 1, output_tokens: 0 } } }, { type: 'message_stop' }];
const MIN_O = [{ id: 'c1', model: 'gpt-5', choices: [{ delta: { content: 'ok' } }] }, { id: 'c1', model: 'gpt-5', choices: [{ delta: {}, finish_reason: 'stop' }] }, { id: 'c1', model: 'gpt-5', choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } }];

P('anthropic', 'req-string-system-no-tools', { model: 'claude-sonnet-5', system: 'be helpful', maxTokens: 100, messages: [{ role: 'user', content: 'hi' }] }, MIN_A);
P('anthropic', 'req-block-system-cache-on-last-block', { model: 'claude-sonnet-5', system: [{ type: 'text', text: 'one' }, { type: 'text', text: 'two' }], maxTokens: 100, messages: [{ role: 'user', content: [{ type: 'text', text: 'a' }, { type: 'text', text: 'b' }] }] }, MIN_A);
P('anthropic', 'req-cache-last-user-block-not-last-message', { model: 'm', maxTokens: 10, messages: HIST.concat([{ role: 'assistant', content: [{ type: 'text', text: 'tail' }] }]) }, MIN_A);
P('anthropic', 'req-user-content-string-not-cached', { model: 'm', maxTokens: 10, messages: [{ role: 'user', content: 'plain' }] }, MIN_A);
P('anthropic', 'req-tools-strip-extra-fields', { model: 'm', maxTokens: 10, tools: TOOLS, messages: HIST }, MIN_A);
P('anthropic', 'req-empty-tools-omitted', { model: 'm', maxTokens: 10, tools: [], messages: [] }, MIN_A);
P('anthropic', 'req-temperature', { model: 'm', maxTokens: 10, temperature: 0.2, messages: [] }, MIN_A);
P('anthropic', 'req-thinking-suppresses-temperature', { model: 'm', maxTokens: 10, temperature: 0.2, thinking: { budgetTokens: 2048 }, messages: [] }, MIN_A);
P('anthropic', 'req-no-system-key-when-undefined', { model: 'm', maxTokens: 10, messages: [{ role: 'user', content: [] }] }, MIN_A);
P('openai', 'req-system-string-and-history', { model: 'gpt-5', system: 'be helpful', maxTokens: 10, messages: HIST }, MIN_O);
P('openai', 'req-system-blocks-joined', { model: 'gpt-5', system: [{ type: 'text', text: 'one' }, { type: 'text', text: 'two' }, {}], maxTokens: 10, messages: [] }, MIN_O);
P('openai', 'req-empty-system-omitted', { model: 'gpt-5', system: '', maxTokens: 10, messages: [{ role: 'user', content: 'hi' }] }, MIN_O);
P('openai', 'req-images-url-and-base64', { model: 'gpt-5', maxTokens: 10, messages: [{ role: 'user', content: [{ type: 'text', text: 'see' }, { type: 'image', source: { type: 'base64', media_type: 'image/png', data: 'QUJD' } }, { type: 'image', source: { type: 'url', url: 'https://x.test/a.png' } }, { type: 'image', source: { type: 'other' } }] }] }, MIN_O);
P('openai', 'req-tool-result-variants', { model: 'gpt-5', maxTokens: 10, messages: [{ role: 'user', content: [{ type: 'tool_result', tool_use_id: 'a', content: [{ type: 'text', text: 'l1' }, { type: 'image', source: { type: 'url', url: 'u' } }] }, { type: 'tool_result', tool_use_id: 'b', content: { k: 1 } }, { type: 'tool_result', tool_use_id: 'c', content: 'plain' }] }] }, MIN_O);
P('openai', 'req-assistant-string-and-tool-only', { model: 'gpt-5', maxTokens: 10, messages: [{ role: 'assistant', content: 'plain' }, { role: 'assistant', content: [{ type: 'tool_use', id: 't', name: 'X' }] }, { role: 'system', content: 'ignored' }, { role: 'user', content: 'u' }] }, MIN_O);
P('openai', 'req-tools-as-functions', { model: 'gpt-5', maxTokens: 10, tools: TOOLS, messages: [] }, MIN_O);
P('openai', 'req-reasoning-effort', { model: 'gpt-5', maxTokens: 10, reasoningEffort: 'high', messages: [] }, MIN_O);
P('openai', 'req-temperature-kept-for-non-reasoning-model', { model: 'gpt-4.1', maxTokens: 10, temperature: 0.3, messages: [] }, MIN_O);
P('openai', 'req-temperature-dropped-for-gpt5-o-series-and-prefixed', { model: 'gpt-5', maxTokens: 10, temperature: 0.3, messages: [] }, MIN_O);
P('openai', 'req-temperature-dropped-for-o3', { model: 'o3-mini', maxTokens: 10, temperature: 0.3, messages: [] }, MIN_O);
P('openai', 'req-temperature-dropped-for-routed-gpt5', { model: 'corp/gpt-5-mini', maxTokens: 10, temperature: 0.3, messages: [] }, MIN_O);
P('openai', 'req-temperature-kept-for-other-routed', { model: 'corp/claude-x', maxTokens: 10, temperature: 0, messages: [] }, MIN_O);

// ---------------- recorded chunks -> normalized events ----------------
const c = (extra) => ({ id: 'chatcmpl-1', model: 'gpt-5', ...extra });
const d = (delta, finish_reason) => c({ choices: [{ delta, ...(finish_reason ? { finish_reason } : {}) }] });
const REQ = { model: 'gpt-5', messages: [{ role: 'user', content: 'hi' }], maxTokens: 100 };
P('openai', 'stream-text-hello-world', REQ, [d({ content: 'Hello ' }), d({ content: 'world' }), d({}, 'stop'), c({ choices: [], usage: { prompt_tokens: 5, completion_tokens: 2 } })]);
P('openai', 'stream-two-parallel-tool-calls-interleaved', REQ, [
  d({ content: 'Hello ' }), d({ content: 'world' }),
  d({ tool_calls: [{ index: 0, id: 'call_a', function: { name: 'Bash', arguments: '' } }] }),
  d({ tool_calls: [{ index: 1, id: 'call_b', function: { name: 'Read', arguments: '' } }] }),
  d({ tool_calls: [{ index: 0, function: { arguments: '{"cmd":' } }] }), d({ tool_calls: [{ index: 1, function: { arguments: '{"path":"a.ts"}' } }] }), d({ tool_calls: [{ index: 0, function: { arguments: '"ls"}' } }] }),
  d({}, 'tool_calls'), c({ choices: [], usage: { prompt_tokens: 42, completion_tokens: 7, prompt_tokens_details: { cached_tokens: 10 } } }),
]);
P('openai', 'stream-finish-length-maps-max-tokens', REQ, [d({ content: 'hi' }), d({}, 'length'), c({ choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } })]);
P('openai', 'stream-finish-unknown-maps-end-turn', REQ, [d({ content: 'hi' }), d({}, 'content_filter'), c({ choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } })]);
P('openai', 'stream-finish-function_call-maps-end-turn', REQ, [d({ content: 'hi' }), d({}, 'function_call'), c({ choices: [], usage: {} })]);
P('openai', 'stream-usage-with-finish-in-same-chunk', REQ, [d({ content: 'x' }), c({ choices: [{ delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 3, completion_tokens: 4 } })]);
P('openai', 'stream-no-usage-chunk-ends-without-message-stop', REQ, [d({ content: 'x' }), d({}, 'stop')]);
P('openai', 'stream-usage-without-finish-defaults-end-turn', REQ, [d({ content: 'x' }), c({ choices: [], usage: { prompt_tokens: 3, completion_tokens: 4 } })]);
P('openai', 'stream-empty-stream', REQ, []);
P('openai', 'stream-empty-choices-and-null-delta', REQ, [c({ choices: [] }), c({ choices: [{ delta: null }] }), d({ content: '' }), d({ content: 'z' }), d({}, 'stop'), c({ choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } })]);
P('openai', 'stream-text-after-tool-gets-new-block-order', REQ, [d({ tool_calls: [{ index: 0, id: 'x', function: { name: 'N', arguments: '{}' } }] }), d({ content: 'late text' }), d({}, 'tool_calls'), c({ choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } })]);
P('openai', 'stream-tool-call-without-id-or-name', REQ, [d({ tool_calls: [{ index: 3, function: { arguments: '{"a":1}' } }] }), d({}, 'tool_calls'), c({ choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } })]);
P('openai', 'stream-cached-tokens-zero-omitted', REQ, [d({ content: 'x' }), d({}, 'stop'), c({ choices: [], usage: { prompt_tokens: 9, completion_tokens: 1, prompt_tokens_details: { cached_tokens: 0 } } })]);
P('openai', 'stream-gateway-model-and-id-from-first-chunk', REQ, [{ id: 'gw-1', model: 'corp/claude', choices: [{ delta: { content: 'a' } }] }, { id: 'gw-2', model: 'other', choices: [{ delta: { content: 'b' } }] }, { id: 'gw-2', choices: [{ delta: {}, finish_reason: 'stop' }] }, { choices: [], usage: { prompt_tokens: 1, completion_tokens: 1 } }]);

const AREQ = { model: 'claude-sonnet-5', maxTokens: 100, messages: [{ role: 'user', content: 'hi' }] };
P('anthropic', 'stream-events-pass-through-unchanged', AREQ, [
  { type: 'message_start', message: { id: 'msg_1', model: 'claude-sonnet-5', usage: { input_tokens: 10, output_tokens: 0 } } },
  { type: 'content_block_start', index: 0, content_block: { type: 'text', text: '' } },
  { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'Hel' } },
  { type: 'content_block_delta', index: 0, delta: { type: 'text_delta', text: 'lo' } },
  { type: 'content_block_stop', index: 0 },
  { type: 'content_block_start', index: 1, content_block: { type: 'tool_use', id: 'toolu_1', name: 'Bash', input: {} } },
  { type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: '{"command":' } },
  { type: 'content_block_delta', index: 1, delta: { type: 'input_json_delta', partial_json: '"ls"}' } },
  { type: 'content_block_stop', index: 1 },
  { type: 'message_delta', delta: { stop_reason: 'tool_use' }, usage: { output_tokens: 12 } },
  { type: 'message_stop' },
]);
P('anthropic', 'stream-thinking-and-signature-events', AREQ, [{ type: 'content_block_start', index: 0, content_block: { type: 'thinking', thinking: '' } }, { type: 'content_block_delta', index: 0, delta: { type: 'thinking_delta', thinking: 'hmm' } }, { type: 'content_block_delta', index: 0, delta: { type: 'signature_delta', signature: 'sig' } }, { type: 'content_block_stop', index: 0 }, { type: 'ping' }]);
P('anthropic', 'stream-empty', AREQ, []);

// ---------------- model list normalisation and client options ----------------
add('modelnorm-official-anthropic-shape', { op: 'normalizeModels', models: [{ id: 'claude-x', display_name: 'Claude X', created_at: '2026-01-01T00:00:00Z', type: 'model' }] }, { source: 'src/native/providers/modelnorm.js' });
add('modelnorm-gateway-shapes', { op: 'normalizeModels', models: [{ id: 'gw/a', object: 'model', owned_by: 'x', context_length: 128000, max_output_tokens: 8192, capabilities: { vision: true } }, { id: 'b', max_input_tokens: 1000 }, { id: 'c', context_window: 5, displayName: 'C!' }, { id: 'd', context_length: 0 }, { id: 'e', context_length: null, max_input_tokens: 7 }] }, { source: 'src/native/providers/modelnorm.js' });
add('anthropic-list-models-iterates-pages', { op: 'listModels', provider: 'anthropic', models: [{ id: 'm1', display_name: 'M1' }, { id: 'm2' }] });
add('openai-list-models-iterates-pages', { op: 'listModels', provider: 'openai', models: [{ id: 'gpt-5', context_length: 400000 }, { id: 'o3' }] });
add('anthropic-client-options', { op: 'clientOptions', provider: 'anthropic', cases: [{}, { baseURL: 'https://gw.test/v1', apiKey: 'k' }, { baseURL: 'https://gw.test/v1/', authToken: 't', apiKey: 'k' }, { baseURL: 'https://gw.test/prefix/v1', headers: { A: 'b' } }, { apiKey: undefined }] });
add('openai-client-options', { op: 'clientOptions', provider: 'openai', cases: [{}, { baseURL: 'https://gw.test/v1', apiKey: 'k' }, { authToken: 't', apiKey: 'k' }, { headers: { A: 'b' } }] });
add('anthropic-uses-bearer-table', { op: 'bearer', cases: [{}, { authHeader: 'bearer' }, { authHeader: 'x-api-key', providerId: 'gw' }, { providerId: 'anthropic', baseURL: 'https://gw.test' }, { providerId: 'gw', baseURL: 'https://api.anthropic.com' }, { providerId: 'gw', baseURL: 'https://eu.anthropic.com/v1' }, { providerId: 'gw', baseURL: 'https://gw.test', key: 'sk-ant-1' }, { providerId: 'gw', baseURL: 'https://gw.test', key: 'sk-x' }, { providerId: 'gw', baseURL: 'not a url' }, { providerId: 'gw', baseURL: 'https://gw.test' }] }, { source: 'anthropicUsesBearer in src/native/providers/index.js' });

exports.source = source;
exports.cases = () => out;

const asyncIter = (items) => (async function* () { for (const x of items) yield x; })();
exports.run = async (i) => {
  const lib = i.provider === 'openai' ? oai : anth;
  switch (i.op) {
    case 'stream': {
      const calls = [];
      const create = async (body, opts) => { calls.push({ body, opts_keys: Object.keys(opts || {}).sort() }); return asyncIter(i.chunks); };
      const clientImpl = i.provider === 'openai' ? { chat: { completions: { create } } } : { messages: { create } };
      const provider = (i.provider === 'openai' ? oai.createOpenAIProvider : anth.createAnthropicProvider)({ id: i.provider, clientImpl });
      const events = [];
      for await (const e of provider.stream(JSON.parse(JSON.stringify(i.req)))) events.push(e);
      return { wire_body: calls[0].body, create_call_option_keys: calls[0].opts_keys, events, provider_type: provider.type };
    }
    case 'normalizeModels': return i.models.map(norm.normalizeModel);
    case 'listModels': {
      const client = { models: { list: () => asyncIter(i.models) } };
      const p = (i.provider === 'openai' ? oai.createOpenAIProvider : anth.createAnthropicProvider)({ id: i.provider, clientImpl: client });
      return p.listModels({});
    }
    case 'clientOptions': return i.cases.map((o) => ({ in: o, out: lib.clientOptions(o) }));
    case 'bearer': return i.cases.map((o) => ({ in: o, bearer: index.anthropicUsesBearer(o) }));
    default: throw new Error(i.op);
  }
};
