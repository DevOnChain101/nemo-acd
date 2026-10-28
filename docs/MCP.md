# MCP servers in Nemo

Nemo is an MCP client (spec 2025-06-18). It never advertises sampling, roots or elicitation, and it never stores credentials (ADR-011).

## Configure

User scope lives in `<NEMO_HOME>/nemo.json` under `mcpServers`; project scope in `<project>/.mcp.json` (needs `--trust-project` plus `--approve-mcp NAME@HASH`; `nemo mcp get NAME` prints the hash). `nemo mcp add|list|get|remove` edit these files.

```json
{ "mcpServers": {
  "files":   { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] },
  "docs":    { "type": "http", "url": "https://example.com/mcp",
               "headers": { "Authorization": "Bearer ${DOCS_TOKEN}" } },
  "legacy":  { "type": "sse",  "url": "https://example.com/sse" },
  "atlassian": { "command": "npx", "args": ["-y", "mcp-remote", "https://mcp.atlassian.com/v1/sse"],
                 "tool_timeout_sec": 120, "startup_timeout_sec": 60 }
} }
```

* `${VAR}` / `${VAR:-default}` work in command, args, env, url and headers. A user-scope entry may read the variables listed in `nemo.mcp.env_passthrough` (user `settings.json`); project and local entries only the child-env allowlist. A variable that is unset or empty makes the server fail to start with a message naming it (it never starts unauthenticated).
* A stdio server's own `env` map is the opt-in path for credentials: names like `JIRA_API_TOKEN` are passed as written (precedence: inherited allowlist < server `env`), the parent's other secrets never are. HOME/USERPROFILE/APPDATA are inherited (`mcp-remote` keeps its OAuth cache in `~/.mcp-auth`).
* HTTP/SSE `headers` are sent on every request (`Authorization: Bearer ...`, `Basic ...`, any other scheme, `X-*`, `Cookie`) and never to another origin after a redirect.
* `type: http` is streamable HTTP; `type: sse` is the legacy transport; a bare `url` tries http, then sse. SSE needs https (http only to localhost) and refuses private addresses.
* Per server: `tool_timeout_sec | tool_timeout_ms | timeout` (ms), `startup_timeout_sec | startup_timeout_ms`, `enabled_tools`, `disabled_tools`.
* Tools are named `mcp__<server>__<tool>`; permission rules use that name (`mcp__docs`, `mcp__docs__getPage`).
* Servers that offer resources/prompts get read-only tools `mcp__<server>__resource_read` and `mcp__<server>__prompt_get`.

### Atlassian (Jira, Confluence)

Atlassian's remote server uses OAuth, which Nemo does not implement. Run it through `mcp-remote` as a stdio server (it opens the browser login itself and keeps its own tokens), as above, or supply a Bearer token from an environment variable: `"headers": {"Authorization": "Bearer ${ATLASSIAN_TOKEN}"}`.

## Safe updates (read-before-write guard)

Update tools of servers like Confluence/Jira replace what they are sent. Nemo therefore refuses to send a write to an MCP tool until the same task has read that resource (page id, issue key, uri) through a read tool of the same server, and refuses again after a write until it is re-read. The model gets a normal tool error telling it to fetch, merge, retry. If the update would drop more than 30 % of the fetched text or a heading, or the tool is destructive, a person must approve it (headless runs refuse).

Tune it in your own `settings.json` (a project file can only add `write_tools`):

```json
{ "nemo": { "mcp": { "servers": {
  "atlassian": { "write_tools": ["publishDraft"], "read_tools": ["lookupThing"] }
} } } }
```

## Troubleshooting

### Writes show up as `anonymous`

First run `nemo mcp test NAME` (and `nemo mcp get NAME`, `nemo doctor`): they show per server which `env` / `headers` will be applied (names and state only, never values) and call the server's `whoami`-style tool. Nemo also appends a warning to any tool result that names `anonymous` as author/user. If Nemo applies the credentials and the server still says anonymous, check:

* The OAuth consent / token belongs to the app, not to you (remote Atlassian MCP): re-run the login as the right account (`rm -r ~/.mcp-auth`, then retry).
* The API token is tied to a service account or to another email than `JIRA_USERNAME`/`CONFLUENCE_USERNAME`.
* Wrong site or `cloudId`: the token is valid on a different Atlassian site than the one written to.
* `mcp-remote` could not read its cache because `HOME`/`USERPROFILE` was missing or unwritable (sandbox, container): `nemo mcp get` shows `inherited profile dirs`.
* The token is read-only or lacks the write scope: the server falls back to anonymous or rejects.
* A project `.mcp.json` entry of the same name is shadowed by (or shadows) your user entry: the user entry wins completely, credentials are never merged.

* `nemo doctor --cwd .` lists servers and why one was skipped; `nemo mcp list` shows the same.
* "not started: not approved": pass the `--approve-mcp NAME@HASH` it prints.
* "connect timed out": raise `startup_timeout_sec` (first `npx` runs download packages).
* Results marked "omitted" or "full result saved to": the server returned more than 28 000 characters; read the saved file for everything.
* A write is "Not sent": fetch the resource first with the read tool named in the message.
* OAuth-only server: use `mcp-remote` (above). Tokens are never written by Nemo.
