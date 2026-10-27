'use strict';
const { ref } = require('../env');
const path = require('path');
const { attempt } = require('../lib');
const cfg = require(path.join(ref, 'src/native/config.js'));
const schema = require(path.join(ref, 'src/native/configschema.js'));
const pcfg = require(path.join(ref, 'src/native/providercfg.js'));

const source = 'src/native/config.js';
const V1 = 'config.json/v1';
const NH = '<NEMO_HOME>';
const WS = '<TMP>/ws';
const out = [];
const add = (id, input, extra = {}) => out.push({ id, input, ...extra });

// op loadConfig: files written into the sandbox, env passed explicitly, cwd/home explicit.
const lc = (id, files, env = {}, extra) => add(id, { op: 'loadConfig', files, env, cwd: WS, home: NH }, extra);
const home = (o) => ({ [`${NH}/nemo.json`]: { $schema: V1, ...o } });
const proj = (o) => ({ [`${WS}/.nemo/config.json`]: { $schema: V1, ...o } });
const GW = { gateway: { type: 'openai', baseURL: 'https://gw.example.test/v1', apiKey: 'sk-test-inline', models: ['m-1', 'm-2'] } };

lc('load-no-files-defaults', {});
lc('load-home-model-and-provider', home({ model: 'gateway/m-1', providers: GW }));
lc('load-cwd-overrides-home-model', { ...home({ model: 'gateway/m-1', providers: GW }), ...proj({ model: 'gateway/m-2' }) });
lc('load-state-model-beats-home', { ...home({ model: 'gateway/m-1', providers: GW }), [`${NH}/state.json`]: { model: 'gateway/m-2', effort: 'high' } });
lc('load-cwd-beats-state', { ...home({ providers: GW }), [`${NH}/state.json`]: { model: 'gateway/m-2' }, ...proj({ model: 'gateway/m-1' }) });
lc('load-env-model-beats-all', { ...home({ model: 'gateway/m-1', providers: GW }), ...proj({ model: 'gateway/m-2' }) }, { NEMO_MODEL: 'anthropic/claude-env' });
lc('load-env-small-model', home({ model: 'gateway/m-1', providers: GW }), { NEMO_SMALL_MODEL: 'gateway/m-2' });
lc('load-env-base-url-default-anthropic', {}, { NEMO_BASE_URL: 'https://proxy.example.test/anthropic', NEMO_API_KEY: 'k' });
lc('load-env-provider-new-gateway-openai-default', {}, { NEMO_PROVIDER: 'corp', NEMO_BASE_URL: 'https://corp.example.test/v1', NEMO_API_KEY: 'k' });
lc('load-env-provider-new-gateway-sk-ant-key', {}, { NEMO_PROVIDER: 'corp', NEMO_BASE_URL: 'https://corp.example.test/v1', NEMO_API_KEY: 'sk-ant-abc' });
lc('load-env-provider-credentials-only-creates-nothing', {}, { NEMO_PROVIDER: 'corp', NEMO_API_KEY: 'k' });
lc('load-env-provider-case-insensitive-id', home({ providers: { Corp: { type: 'openai', baseURL: 'https://c.example.test/v1' } } }), { NEMO_PROVIDER: 'corp', NEMO_BASE_URL: 'https://other.example.test/v1' });
lc('load-env-custom-headers', {}, { NEMO_BASE_URL: 'https://p.example.test', NEMO_CUSTOM_HEADERS: 'X-One: 1\r\nX-Two:two\nbad line\nx-one:  override  ' });
lc('load-env-alias-overrides', home({ aliases: { opus: 'anthropic/claude-opus-1' } }), { NEMO_DEFAULT_OPUS_MODEL: 'gateway/o-9', NEMO_DEFAULT_SONNET_MODEL: 'gateway/s-9', NEMO_DEFAULT_CUSTOM_MODEL: 'gateway/c-9' });
lc('load-env-retargets-model-provider', home({ model: 'anthropic/foo', providers: GW }), { NEMO_PROVIDER: 'gateway', NEMO_BASE_URL: 'https://gw2.example.test/v1' });
lc('load-env-retarget-bare-model', home({ model: 'foo' }), { NEMO_PROVIDER: 'gateway', NEMO_AUTH_TOKEN: 't', NEMO_BASE_URL: 'https://gw2.example.test/v1' });
lc('load-ignores-anthropic-openai-env', {}, { ANTHROPIC_API_KEY: 'x', ANTHROPIC_BASE_URL: 'https://nope.example.test', OPENAI_API_KEY: 'y' });
lc('load-deep-merge-providers-home-cwd', { ...home({ providers: GW }), ...proj({ providers: { gateway: { baseURL: 'https://proj.example.test/v1' }, extra: { type: 'anthropic' } } }) });
lc('load-opencode-provider-key-npm-options', home({ provider: { oc: { npm: '@ai-sdk/anthropic', name: 'OpenCode GW', options: { baseURL: 'https://oc.example.test', apiKey: 'sk-oc', headers: { 'X-A': 'a' } }, models: { 'claude-x': { name: 'Claude X', limit: { context: 200000, output: 8192 } } } } } }));
lc('load-provider-wins-over-providers-on-collision', home({ providers: { dup: { type: 'openai', baseURL: 'https://a.example.test' } }, provider: { dup: { type: 'anthropic', baseURL: 'https://b.example.test' } } }));
lc('load-env-substitution-ok', home({ providers: { p: { type: 'openai', apiKey: '{env:MY_KEY}', headers: { Auth: '{env:MY_HDR}' } } } }), { MY_KEY: 'sk-from-env', MY_HDR: 'h' });
lc('load-env-substitution-missing-warns', home({ providers: { p: { type: 'openai', apiKey: '{env:NOPE}', authToken: '{env:NOPE2}', headers: { A: '{env:NOPE3}' } } } }));
lc('load-file-substitution-relative', { ...home({ providers: { p: { type: 'openai', apiKey: '{file:secret.txt}' } } }), [`${NH}/secret.txt`]: '  sk-file-key \n' });
lc('load-file-substitution-missing-warns', home({ providers: { p: { type: 'openai', apiKey: '{file:missing.txt}' } } }));
lc('load-file-substitution-tilde-home', { ...home({ providers: { p: { type: 'openai', authToken: '{file:~/tok.txt}' } } }), '<HOME>/tok.txt': 'tok-home' });
lc('load-api-key-false-is-noauth', home({ providers: { local: { type: 'openai', baseURL: 'http://localhost:11434/v1', apiKey: false, authToken: 'ignored' } } }));
lc('load-models-object-with-limits-and-caps', home({ providers: { p: { type: 'anthropic', models: { 'm-a': { name: 'A', vendor: 'v', family: 'sonnet', qualityScore: 3, reasoning: true, limit: { context: 1000, output: 100 }, capabilities: { effort_tiers: ['low', 'high'], vision: true } }, 'm-b': {} } } } }));
lc('load-type-detected-from-url-and-id', home({ providers: { a1: { baseURL: 'https://api.anthropic.com' }, a2: { baseURL: 'https://x.example.test/anthropic/v1' }, a3: { baseURL: 'https://api.openai.com/v1' }, a4: { npm: '@ai-sdk/openai-compatible' }, a5: {} } }));
lc('load-aliases-profiles-mcp-projects-claude', home({ aliases: { fast: 'gateway/m-1' }, profiles: { deep: ['gateway/m-2', { model: 'gateway/m-1' }], fast: { candidates: ['gateway/m-1'] } }, mcp: { s1: { type: 'local', command: ['node', 'x.js'], enabled: true, environment: { A: 'b' } } }, mcpServers: { s2: { type: 'http', url: 'https://mcp.example.test', headers: { A: 'b' } } }, projects: { '/p': { enabledMcpjsonServers: ['a'], enableAllProjectMcpServers: true } }, claude: { readUserConfig: false }, maxTokens: 4096 }));
lc('load-max-tokens-only', home({ maxTokens: 123 }));
// errors
lc('load-error-missing-schema', { [`${NH}/nemo.json`]: { model: 'a/b' } });
lc('load-error-unsupported-schema', { [`${NH}/nemo.json`]: { $schema: 'config.json/v2' } });
lc('load-error-schema-not-string', { [`${NH}/nemo.json`]: { $schema: 1 } });
lc('load-error-malformed-json-home', { [`${NH}/nemo.json`]: '{"apiKey": "sk-secret", ' });
lc('load-error-malformed-json-cwd', { [`${WS}/.nemo/config.json`]: 'not json' });
lc('load-error-root-is-array', { [`${NH}/nemo.json`]: '[1,2]' });
lc('load-error-unknown-top-level-field', home({ bogus: true, alsoBogus: 1 }));
lc('load-error-bad-provider-fields', home({ providers: { p: { type: 'gemini', apiKey: '', authHeader: 'basic', headers: { a: 1 }, discover: 'yes', extra: 1 } } }));
lc('load-error-bad-maxtokens-and-aliases', home({ maxTokens: -5, aliases: { a: '' } }));
lc('load-error-bad-models-and-capabilities', home({ providers: { p: { models: { m: { limit: { context: 0 }, capabilities: { effort_tiers: ['low', 'low'], x: 'y' }, junk: 1 } } } } }));
lc('load-error-bad-mcp-and-profiles', home({ mcp: { s: { type: 'weird', command: 5 } }, profiles: { p: { candidates: 'x' }, q: [{ model: '' }] } }));
lc('load-error-more-than-12-problems-truncated', home(Object.fromEntries(Array.from({ length: 20 }, (_, i) => [`k${i}`, i]))));
lc('load-error-cwd-config-invalid-after-valid-home', { ...home({ model: 'a/b' }), [`${WS}/.nemo/config.json`]: { $schema: V1, model: 5 } });
lc('load-state-json-garbage-ignored', { [`${NH}/state.json`]: 'garbage' });

// op validate
const val = (id, raw, file = 'config.json') => add(id, { op: 'validate', raw, file }, { source: 'src/native/configschema.js' });
val('validate-ok-minimal', { $schema: V1 });
val('validate-ok-full', { $schema: V1, model: 'a/b', smallModel: 'a/c', providers: { p: { type: 'openai', models: ['x'] } } });
val('validate-error-control-chars-sanitized', { $schema: V1, 'bad\u0001key': 1 }, 'we\u0007ird.json');
val('validate-error-null-root', null);
val('validate-error-long-diagnostic-truncated', { $schema: V1, ['k'.repeat(600)]: 1 });

// pure resolver ops (config literal = defaults() merged shallowly, stored in input so the case is self-contained)
const base = (o = {}) => ({ ...cfg.defaults(), ...o });
const IDS = ['claude-sonnet-4-5', 'claude-sonnet-5', 'claude-opus-4-1-20250805', 'claude-opus-5-5', 'claude-haiku-4-5', 'kr/claude-sonnet-9', 'combo-claude-sonnet-5'];
const R = (id, config, discovered, opts) => add(id, { op: 'resolveModelDefaults', config, discovered, modelExplicit: !!opts.modelExplicit, explicitAliases: opts.explicitAliases || [] }, { source: 'resolveModelDefaults in src/native/config.js' });
R('resolve-swaps-nonexplicit-model-same-family', base({ model: 'gateway/claude-sonnet-4' }), { gateway: IDS }, {});
R('resolve-explicit-model-never-swapped-warns', base({ model: 'gateway/claude-sonnet-4' }), { gateway: IDS }, { modelExplicit: true });
R('resolve-model-offered-unchanged', base({ model: 'gateway/claude-sonnet-5' }), { gateway: IDS }, {});
R('resolve-null-alias-filled-silently', base(), { gateway: IDS }, {});
R('resolve-explicit-alias-kept-with-warning', base({ aliases: { opus: 'gateway/claude-opus-3', sonnet: null, haiku: null, fable: null } }), { gateway: IDS }, { explicitAliases: ['opus'] });
R('resolve-nonexplicit-alias-swapped-with-note', base({ aliases: { opus: 'gateway/claude-opus-3', sonnet: null, haiku: null, fable: null } }), { gateway: IDS }, {});
R('resolve-unknown-provider-discovery-untouched', base({ model: 'other/claude-sonnet-4' }), { gateway: IDS }, {});
R('resolve-no-family-candidate-keeps-target', base({ model: 'gateway/gpt-4' }), { gateway: IDS }, {});

const F = (id, ids, family) => add(id, { op: 'bestModelForFamily', ids, family }, { source: 'bestModelForFamily in src/native/config.js' });
F('family-sonnet-prefers-unprefixed-highest', IDS, 'sonnet');
F('family-opus-ignores-date-suffix', IDS, 'opus');
F('family-fable-none', IDS, 'fable');
F('family-null-family', IDS, null);
F('family-only-routed-ids', ['kr/claude-haiku-4-8', 'agy/claude-haiku-4-9'], 'haiku');
F('family-bare-no-version', ['claude-sonnet', 'claude-sonnet-1'], 'sonnet');

const P = (id, str, config) => add(id, { op: 'parseModel', str, config }, { source: 'parseModel in src/native/config.js' });
P('parse-provider-slash-model', 'gateway/m-1', base());
P('parse-model-with-extra-slash', 'kr/claude/x', base());
P('parse-alias-resolved', 'fast', base({ aliases: { fast: 'gateway/m-1' } }));
P('parse-bare-model-found-in-provider-list', 'm-2', base({ providers: { a: { models: ['m-1'] }, b: { models: ['m-2'] } } }));
P('parse-bare-model-falls-back-to-config-model-provider', 'zzz', base({ model: 'b/m-2', providers: { a: { models: [] }, b: { models: [] } } }));
P('parse-empty-uses-config-model', '', base({ model: 'a/b' }));
P('parse-empty-no-model-throws', '', base());

add('list-models-includes-alias-and-small-targets', { op: 'listModels', config: base({ model: 'gateway/m-9', smallModel: 'm-1', aliases: { x: 'gateway/m-3' }, providers: { gateway: { models: ['m-1', 'm-2'] }, openai: { models: [] } } }), providerIds: null }, { source: 'listModels in src/native/config.js' });
add('list-models-restricted-to-usable-providers', { op: 'listModels', config: base({ providers: { a: { models: ['1'] }, b: { models: ['2'] } } }), providerIds: ['b'] }, { source: 'listModels in src/native/config.js' });

const D = (id, config, usable, discovered, saved) => add(id, { op: 'pickDefaultModel', config, usableProviderIds: usable, discovered, savedModel: saved }, { source: 'pickDefaultModel in src/native/config.js' });
D('pick-default-no-usable-providers', base(), [], {}, undefined);
D('pick-default-saved-model-trusted-when-undiscovered', base(), ['gateway'], {}, 'gateway/m-1');
D('pick-default-saved-model-dropped-when-not-offered', base(), ['gateway'], { gateway: IDS }, 'gateway/gone');
D('pick-default-best-sonnet-of-first-provider', base(), ['x', 'gateway'], { gateway: IDS }, undefined);
D('pick-default-first-id-when-no-sonnet', base(), ['gateway'], { gateway: ['b', 'a'] }, undefined);

const T = (id, args) => add(id, { op: 'detectProtocol', args }, { source: 'detectProtocol in src/native/providercfg.js' });
T('protocol-explicit-type', { id: 'x', type: 'anthropic', baseURL: 'https://api.openai.com' });
T('protocol-npm-anthropic', { id: 'x', npm: '@ai-sdk/anthropic' });
T('protocol-npm-other-is-openai', { id: 'x', npm: 'whatever' });
T('protocol-builtin-id', { id: 'openai' });
T('protocol-url-anthropic-subdomain', { id: 'x', baseURL: 'https://eu.anthropic.com/v1' });
T('protocol-url-anthropic-path-segment', { id: 'x', baseURL: 'https://gw.example.test/anthropic' });
T('protocol-url-malformed-no-signal', { id: 'x', baseURL: 'not a url' });
T('protocol-sk-ant-key', { id: 'x', credential: 'sk-ant-1' });
T('protocol-sk-key-is-no-signal', { id: 'x', credential: 'sk-proj-1' });

exports.source = source;
exports.cases = () => out;
exports.run = async (i, sb) => {
  switch (i.op) {
    case 'loadConfig': {
      sb.setup({ files: i.files });
      return attempt(() => cfg.loadConfig({ cwd: i.cwd, home: i.home, env: i.env }));
    }
    case 'validate': return attempt(() => schema.validateVersionedConfig(i.raw, i.file));
    case 'resolveModelDefaults': return cfg.resolveModelDefaults(i.config, i.discovered, { modelExplicit: i.modelExplicit, explicitAliases: new Set(i.explicitAliases) });
    case 'bestModelForFamily': return { value: cfg.bestModelForFamily(i.ids, i.family) };
    case 'parseModel': return attempt(() => cfg.parseModel(i.str, i.config));
    case 'listModels': return cfg.listModels(i.config, i.providerIds);
    case 'pickDefaultModel': return { value: cfg.pickDefaultModel({ config: i.config, usableProviderIds: i.usableProviderIds, discoveredByProvider: i.discovered, savedModel: i.savedModel }) };
    case 'detectProtocol': return pcfg.detectProtocol(i.args);
    default: throw new Error(`unknown op ${i.op}`);
  }
};
