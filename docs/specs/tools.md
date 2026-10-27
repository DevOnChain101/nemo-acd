# Spec: Tool registry, schemas and execution pipeline — draft 0.1

Tags as in events.md. Exact `description` strings are not copied here; they are captured verbatim into `tests/golden/tools/schemas.<os>.json` (eval-and-golden.md §B), because Node varies them per OS (Bash text differs on win32 [from-node tools/bash.js:131]).

## 1. Registry contract

```rust
pub enum SideEffectClass { Read, Write, Execute, External }      // doc §7.1
pub enum ToolRuntime { Builtin, Harness, Mcp { server: String } } // Harness = run by the agent loop itself [from-node tools/index.js:37-40]
pub struct ToolSpec {
    pub name: String,            // wire name sent to providers, ^[A-Za-z0-9_-]{1,64}$ [from-node mcp.js:110]
    pub logical_id: String,      // doc §5.6 id, e.g. "terminal.spawn"
    pub input_schema: serde_json::Value, // JSON Schema object, exactly as sent to providers
    pub side_effect: SideEffectClass,
    pub rule_name: String,       // name used in permission rules, normally == name
    pub runtime: ToolRuntime,
}
pub struct ToolResult {          // [from-node tools/index.js:3-6]
    pub content: Content,        // String | Vec<Block{text|image}>
    pub is_error: bool,
    pub soft: bool,              // refusal/not-applicable, drawn neutral [from-node tools/soft.js:3-7]
    pub meta: serde_json::Value, // e.g. {"replacements":2} [from-node edit.js:124]
    pub artifacts: Vec<ArtifactRef>,
}
```
Rules:
1. Expected failures are returned as `is_error: true`, never as a Rust `Err`; only cancellation propagates [from-node tools/index.js:3-6].
2. Naming [new]: tools ported from Node keep their names (`Read`, `Bash`, …): the transcript parser keys on them [from-node parser.js:225, history.js:108] and two-way compatibility needs them. Doc §5.6 ids map to these instead of being added again: `file.read`=`Read`, `file.write`=`Write`, `file.patch`=`Edit`, `file.search`=`Grep`+`Glob`, `terminal.exec`=`Bash`. New tools use wire name = logical id with `.` → `_` (`terminal_spawn`), because dots are invalid in Anthropic/OpenAI tool names.
3. MCP tools: `mcp__<server>__<tool>`, sanitized, truncated to 64 chars with a sha1-6 suffix [from-node mcp.js:115-140]. Default class `External`.
4. Registry order is stable (it feeds prompt caching): the 17 built-ins in Node order [from-node tools/index.js:26], then Task/AskUserQuestion/ExitPlanMode, then new tools, then MCP.

## 2. Side-effect classes

| Class | Tools |
|---|---|
| Read | Read, Glob, Grep, BashOutput, TodoWrite, TaskCreate/Get/List/Update, Skill, Task¹, AskUserQuestion, ExitPlanMode, terminal_logs, terminal_status, git_status, git_diff, artifact_read, browser_snapshot, browser_screenshot, browser_console, browser_network |
| Write | Write, Edit, NotebookEdit, git_apply_patch |
| Execute | Bash, KillShell, terminal_spawn, terminal_signal, verify_run, git_revert |
| External | WebFetch, WebSearch, browser_navigate, browser_click, browser_fill, mcp__* |

¹ Task runs a sub-agent whose own calls are gated one by one. The class drives defaults; permission rules still decide (Node's READ_ONLY_TOOLS/EDIT_TOOLS sets [from-node permissions.js:11-12] are ported verbatim for the `Permissions::check` port).

## 3. Execution pipeline (per call, calls in one batch run sequentially [from-node agent.js:3489-3508])
0. **Parse** — tool input cut off or not valid JSON → error `Tool input was cut off …` without running [from-node agent.js:3514-3522]. Unknown name → `Unknown tool "<name>"` [from-node agent.js:3528-3535].
1. **Validate** [new] — JSON Schema validation of `input` against `input_schema` (Node only checks by hand in each tool). Failure → `is_error`, `soft:false`, message lists the first 5 violations.
2. **Policy** — `Permissions::check(name, input, turn_mode)` ported verbatim [from-node permissions.js:719-829], order: deny rule → dialog tools → critical `rm` → outside-read (blocked/network) → ask rule → bypassPermissions → plan → protected paths → allow rules → mode defaults. `classify` → auto-mode classifier [from-node agent.js:3723]. `ask` → `tool.approval_required`; headless resolves it `auto_denied`. Then **PreToolUse hooks** (may block or rewrite input) [from-node agent.js:3585-3603].
   - Change [new]: if a hook or the approver returns `updatedInput`, re-run `check` on the new input; `deny` → not run, `ask` → ask again. Node skips this re-check, so a hook can change the input after permission was granted.
   - Change [new]: Node lets a read outside the working dirs through when no prompt can be shown [from-node agent.js:3543]; the Rust core denies it in headless.
3. **Workspace bind** — build `ToolContext`; resolve every path field (`file_path`, `path`, `notebook_path`, `cwd`) via workspace-context.md §4.2.
4. **Budget** — check `max_tool_calls`, `max_wall_ms`, per-call timeout; exceeded → `tool.completed{status:"budget_exceeded", not_run:true}` and the task stops.
5. **Execute** — Write-class tools first snapshot the target for /rewind [from-node agent.js:3607]. Cancellation → `Interrupted.` [from-node agent.js:3610-3615].
6. **Normalize** — model-visible text capped at 30 000 chars, head 60 % + tail 40 % [from-node tools/bash.js:8-26, agent.js:255-259]; the full text goes to an artifact [new]. PostToolUse hooks run; their `additionalContext` is appended [from-node agent.js:3625-3644]. `soft` comes from the tool or from the soft-pattern list [from-node tools/soft.js:8-21].
7. **Persist** — `tool.completed` event, `tool_calls` row, artifacts (session-store.md).

## 4. Ported tool schemas [from-node, verbatim field names; `type:"object"` omitted]
`number` stays `number` (Node never declares `integer`).

| Tool | properties (required in **bold**) | Source |
|---|---|---|
| Read | **file_path**:string, offset:number (1-based), limit:number (default 2000), pages:string ("3", "2-5", PDF only) | tools/read.js:35-44 |
| Write | **file_path**:string, **content**:string | tools/write.js:8-21 |
| Edit | **file_path**:string, **old_string**:string, **new_string**:string, replace_all:boolean | tools/edit.js:53-62 |
| NotebookEdit | **notebook_path**:string, cell_id:string, new_source:string, cell_type:enum[code,markdown], edit_mode:enum[replace,insert,delete] | tools/notebook.js:53-62 |
| Glob | **pattern**:string, path:string | tools/glob.js:18-24 |
| Grep | **pattern**:string, path, glob, type:string, output_mode:enum[files_with_matches,content,count], "-i","-n":boolean, "-A","-B","-C":number, multiline:boolean, head_limit:number | tools/grep.js:253-270 |
| Bash | **command**:string, timeout:number (ms, default 120000, max 600000), description:string, run_in_background:boolean | tools/bash.js:139-148 |
| BashOutput | **bash_id**:string, filter:string (regex) | tools/bash.js:259-266 |
| KillShell | **shell_id**:string | tools/bash.js:277-281 |
| WebFetch | **url**:string, **prompt**:string | tools/webfetch.js:40-47 |
| WebSearch | **query**:string, allowed_domains:string[], blocked_domains:string[] | tools/websearch.js:77-85 |
| Skill | **skill**:string, args:string | tools/skill.js:11-18 |
| TodoWrite | **todos**:[{**content**:string, **status**:enum[pending,in_progress,completed], **activeForm**:string}] | tools/todowrite.js:14-31 |
| TaskCreate | **subject**:string, description:string, activeForm:string | tools/taskcreate.js:23-31 |
| TaskGet | **taskId**:string | tools/taskget.js:6-10 |
| TaskList | (none) | tools/tasklist.js:6 |
| TaskUpdate | **taskId**:string, status:enum[pending,in_progress,completed], subject, activeForm, description:string | tools/taskupdate.js:9-18 |
| Task | **description**:string, **prompt**:string, **subagent_type**:string, model, profile, effort:string | tools/index.js:41-59 |
| AskUserQuestion | **questions**:[{**question**:string, header:string, multiSelect:boolean, **options**:[{**label**:string, description:string}]}] | tools/index.js:60-94 |
| ExitPlanMode | **plan**:string (markdown) | tools/index.js:95-107 |

Behaviour that is part of the contract [from-node]: Read output is `cat -n` style `%6d\t<line>`, lines over 2000 chars truncated [read.js:9-114]; Read records `path → mtime` and Edit/Write refuse unless the file was read and has not changed since [edit.js:86-89, write.js:41-44]; Edit keeps CRLF files CRLF [edit.js:91-105]; Bash merges stdout+stderr, appends `[exit code N]`, `is_error` on non-zero or timeout [bash.js:225-231], strips secret env [bash.js:160-163], kills the whole process tree on timeout/abort [bash.js:204-242].

Change [new]: `Bash` with `run_in_background` and `BashOutput`/`KillShell` are served by `TerminalRuntime` (`spawn`/`read_logs`/`signal`); `bash_id` == `process_id`. Node's 1 MiB in-memory tail [bash.js:34] becomes a log artifact with a cursor.

## 5. New tools [new, drafts; schemas in JSON Schema]
```json
{"terminal_spawn":{"properties":{"command":{"type":"string"},"name":{"type":"string"},"cwd":{"type":"string"},"env":{"type":"object","additionalProperties":{"type":"string"}},"tty":{"type":"boolean"},
  "ready":{"type":"object","properties":{"kind":{"enum":["port","http","log"]},"port":{"type":"integer"},"url":{"type":"string"},"pattern":{"type":"string"},"timeout_ms":{"type":"integer"}},"required":["kind"]}},"required":["command"]},
 "terminal_logs":{"properties":{"process_id":{"type":"string"},"cursor":{"type":"string"},"filter":{"type":"string"},"max_bytes":{"type":"integer"}},"required":["process_id"]},
 "terminal_signal":{"properties":{"process_id":{"type":"string"},"signal":{"enum":["int","term","kill"]}},"required":["process_id","signal"]},
 "terminal_status":{"properties":{"process_id":{"type":"string"}}},
 "git_status":{"properties":{}},
 "git_diff":{"properties":{"path":{"type":"string"},"staged":{"type":"boolean"},"base":{"type":"string"},"context":{"type":"integer"}}},
 "git_apply_patch":{"properties":{"patch":{"type":"string"},"check_only":{"type":"boolean"}},"required":["patch"]},
 "git_revert":{"properties":{"paths":{"type":"array","items":{"type":"string"}}},"required":["paths"]},
 "verify_run":{"properties":{"check_ids":{"type":"array","items":{"type":"string"}},"command":{"type":"string"},"criteria_ids":{"type":"array","items":{"type":"string"}},"timeout_ms":{"type":"integer"}}},
 "artifact_read":{"properties":{"artifact_id":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"},"filter":{"type":"string"}},"required":["artifact_id"]},
 "browser_navigate":{"properties":{"url":{"type":"string"},"wait_until":{"enum":["load","domcontentloaded","networkidle"]},"timeout_ms":{"type":"integer"}},"required":["url"]},
 "browser_snapshot":{"properties":{"ref":{"type":"string"},"max_nodes":{"type":"integer"}}},
 "browser_click":{"properties":{"ref":{"type":"string"},"role":{"type":"string"},"name":{"type":"string"},"selector":{"type":"string"}}},
 "browser_fill":{"properties":{"ref":{"type":"string"},"selector":{"type":"string"},"value":{"type":"string"},"sensitive":{"type":"boolean"}},"required":["value"]},
 "browser_screenshot":{"properties":{"full_page":{"type":"boolean"},"ref":{"type":"string"}}},
 "browser_console":{"properties":{"cursor":{"type":"string"},"level":{"enum":["error","warning","info","debug"]}}},
 "browser_network":{"properties":{"cursor":{"type":"string"},"failed_only":{"type":"boolean"}}}}
```
Semantics: `terminal_spawn` returns `{process_id, pid, status, ready:bool, log_cursor}` and outlives the tool call but not the session (doc §5.8). `terminal_status` without id lists the session's processes only. `git_revert` restores paths to the session baseline (agent edits only), never runs `git reset`/`checkout` on user changes, and is always `ask` (doc §5.10). `browser_*` use accessibility refs first, selectors as fallback, no coordinates (doc §5.9); backend decided by the R6 spike. Permission rule syntax for new tools follows Node: `terminal_spawn(<cmd prefix>:*)` like Bash, `browser_navigate(domain:<host>)` like WebFetch [from-node permissions.js:442-507].

## Open
- `TodoWrite` vs `TaskCreate/Update` share one store in Node [from-node taskcreate.js:3-7]; keep both on the wire?
- Should `verify_run` be callable by the model, or only by the harness at the completion gate?
