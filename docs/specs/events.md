# Spec: Event stream (JSONL) — v1 draft

Status: draft 0.1 (R1). Normative words: MUST / SHOULD / MAY. Tags: `[from-node file:line]` = derived from the frozen Node reference (`~/Vibe/nemo-cli`), `[new]` = new design per the design doc §5.12 / §7.3.

## 1. Purpose
One typed, append-only event stream per session. It is the only interface between the core and every client (TUI, headless CLI, eval harness, future GPUI). It replaces Node's untyped `EventEmitter` + `'update'` repaint signal [from-node agent.js:450-4027]. The same records are persisted (session-store.md `events` table) and streamed on stdout in headless mode.

## 2. Envelope [new]
One JSON object per line, UTF-8, `\n` terminated, no pretty printing. Max line size 64 KiB (see §5).

| Field | Type | Rule |
|---|---|---|
| `v` | u16 | Schema major version. `1` for this spec. |
| `seq` | u64 | Starts at 1 per session, +1 per event, no gaps, never reused. A gap seen by a client means loss: re-read from the store with cursor = last seq. |
| `ts` | string | RFC 3339 UTC, millisecond precision, `Z` suffix. Producer wall clock; ordering is by `seq`, never by `ts`. |
| `session_id` | string | UUIDv4 lowercase [from-node agent.js:929 `crypto.randomUUID()`]. |
| `task_id` | string \| null | UUIDv4 of the active task; null before the first task and for session-level events between tasks. |
| `turn_id` | string \| null | UUIDv4 of the current model turn (one provider request/response cycle; Node counts these as `turns` [from-node agent.js:3360]). Null outside a turn. |
| `agent_id` | string \| null | Null for the main agent; sub-agent id for `Task` delegations [from-node agent.js:879]. |
| `type` | string | Dotted lowercase `area.event`. |
| `payload` | object | Type-specific; schema per §3. Always an object (possibly `{}`). |

```json
{"v":1,"seq":17,"ts":"2026-10-26T09:12:03.481Z","session_id":"6f1c…","task_id":"a2b9…","turn_id":"0d44…","agent_id":null,"type":"tool.completed","payload":{"tool_call_id":"toolu_01X","name":"Bash","status":"error","is_error":true,"duration_ms":2310,"preview":"FAIL src/login.test.js…[exit code 1]","bytes":48211,"truncated":true,"result_ref":{"artifact_id":"9c7e…","path":"artifacts/9c7e….txt","size":48211,"sha256":"…","mime":"text/plain"}}}
```

## 3. Taxonomy
Doc §5.12 types are kept verbatim; additions are marked. Payload field types: `str`, `int` (i64), `u64`, `bool`, `obj`, `[T]`, `?` = optional/nullable. `ArtifactRef` defined in §4.

| Type | Payload | Source |
|---|---|---|
| `session.started` | `workspace:{workspace_id:str, root:str, cwd:str, additional_dirs:[str], git_worktree:str?}, model:str, permission_mode:str, source:"startup"\|"resume"\|"clear", resumed_from_seq:u64?` | doc §5.12; `source` [from-node agent.js:1229 SessionStart] |
| `session.status_changed` | `from:SessionStatus, to:SessionStatus, reason:str?` | [new] (state machine, workspace-context.md §3) |
| `session.completed` / `session.failed` / `session.cancelled` | `reason:str, error:str?` — exactly one per session, last event of the stream | doc §5.12 |
| `task.started` | `request:str, acceptance_criteria:[{id:str,text:str,check:str?}], budget:Budget` | [new] |
| `task.ended` | `status:"completed"\|"unverified"\|"blocked"\|"failed"\|"cancelled", reason:str, verification_report_id:str?, usage:Usage, changed_files:[str], artifacts_root:str` | [new]; carries the §6 exit code mapping |
| `user.message` | `text:str, display_text:str?, images:[ArtifactRef]` | [from-node agent.js:1256 transcript `user` + `displayText`] |
| `turn.started` | `model:str, provider:str, iteration:int` | doc §5.12 |
| `model.delta` | `block_index:int, kind:"text"\|"thinking"\|"tool_input", text:str` | [from-node agent.js:3401-3418 text_delta/thinking_delta/input_json_delta] |
| `assistant.message` | `message_id:str?, content:[Block], stop_reason:str, usage:Usage?` (final folded turn; thinking `signature` stripped) | [from-node agent.js:3365] |
| `turn.completed` | `stop_reason:"end_turn"\|"tool_use"\|"max_tokens"\|str, usage:Usage?, duration_ms:u64` | [from-node agent.js:3436 message_delta] |
| `model.retry` | `attempt:int, max:int, error:str, delay_ms:u64` | [from-node agent.js:3184-3195, MAX_MODEL_RETRIES=3 agent.js:50] |
| `model.error` | `status:int?, message:str, retryable:bool, context_overflow:bool` | [from-node agent.js:225-237] |
| `context.compacted` | `trigger:"auto"\|"manual", summary_ref:ArtifactRef, tokens_before:u64?` | [from-node agent.js:1953,3158 + compact_boundary agent.js:2018] |
| `tool.requested` | `tool_call_id:str, name:str, input:obj, input_truncated:bool` | doc §5.12; truncated input [from-node agent.js:3514] |
| `tool.approval_required` | `approval_id:str, tool_call_id:str, dialog:"permission"\|"plan"\|"question"\|"outside_read", prompt:str, options:[str], suggestions:[obj], deadline:str?` | doc §5.12; dialogs [from-node agent.js:3654-3696, 3793, 3828] |
| `tool.approval_resolved` | `approval_id:str, decision:"allow"\|"deny"\|"expired"\|"auto_denied", reviewer:"user"\|"policy"\|"hook"\|"classifier"\|"headless", input_changed:bool, message:str?` | [new]; expiry [from-node agent.js:3706] |
| `tool.started` | `tool_call_id:str, name:str, side_effect:"read"\|"write"\|"execute"\|"external", cwd:str` | doc §5.12 |
| `tool.completed` | `tool_call_id:str, name:str, status:ToolStatus, is_error:bool, not_run:bool, duration_ms:u64, preview:str, bytes:u64, truncated:bool, result_ref:ArtifactRef?` | doc §5.12; statuses [from-node agent.js:3510-3651] |
| `hook.ran` | `event:str, hook_name:str, exit_code:int?, outcome:"success"\|"non_blocking_error"\|"blocking_error", duration_ms:u64` | [from-node agent.js:1032-1041; never stdout/stderr] |
| `process.spawned` / `process.ready` / `process.exited` | `process_id:str, name:str?, pid:int?, exit_code:int?, signal:str?` | [new] TerminalRuntime |
| `artifact.created` | `ArtifactRef + kind:str, retention:str` | doc §5.12 |
| `verification.completed` | `report_id:str, status:VerificationStatus, attempt:int, summary:str` | doc §5.12; see verification.md |
| `todo.updated` | `todos:[{id:str?, content:str, status:"pending"\|"in_progress"\|"completed", active_form:str}]` | [from-node agent.js:3646, taskcreate.js] |
| `subagent.started` / `subagent.stopped` | `agent_id:str, agent_type:str, tool_call_id:str?, reason:str?` | [from-node agent.js:3913 'subagent-start', 874 'subagent-evicted', 3017 SubagentStop] |
| `model.changed` | `model:str, effort:str?` | [from-node agent.js:1539 'model-changed'] |
| `budget.exceeded` | `budget:str, limit:number, used:number` | [new] |
| `notice` | `level:"info"\|"warn"\|"error", text:str` | [from-node agent.js:919-925 note()/toast()] |

`ToolStatus` = `"ok"|"error"|"soft_error"|"denied"|"interrupted"|"timeout"|"budget_exceeded"` [from-node: Node item statuses done/error/interrupted + `soft` flag, soft.js:8-21]. `Usage` = `{input_tokens:u64, output_tokens:u64, cache_read_input_tokens:u64?, cache_creation_input_tokens:u64?, cost_usd:f64?}` [from-node providers/openai.js:78 mapUsage, Anthropic usage passthrough]. `Block` = Anthropic content block shape (`text`, `thinking`, `tool_use`, `tool_result`, `image` with `source` replaced by an `ArtifactRef`).

Node emitter names NOT carried into the stream (they are UI commands handled by the interface layer, not runtime facts) [from-node]: `update`, `view-main`, `view-subagent`, `picker-request`, `attach-request`, `resume-request`, `new-session-request`, `close-request`, `exit-request`, `fork-request`. Mapping of the rest: `initialized`+`session` → `session.started`; `permission` → `tool.approval_required`; `toast` → `notice`.

Ordering invariants: `tool.requested` < (`tool.approval_*`)* < `tool.started`? < `tool.completed`, all with the same `tool_call_id`; a denied call has no `tool.started` and `not_run:true`. Every `tool.requested` gets exactly one `tool.completed` (Node rule: every tool_use needs a tool_result [from-node agent.js:3455]). `task.ended` precedes `session.*` terminal event.

## 4. Artifact references [new]
`ArtifactRef = {artifact_id:str(UUIDv4), path:str, size:u64, sha256:str(hex), mime:str}`; `path` is relative to the session artifact root (`$NEMO_HOME/sessions/<session_id>/`, doc §5.7). Events never inline binary data. Clients read content via `artifact.read` or the file path.

## 5. Redaction and size rules [new unless tagged]
Applied by the producer before persistence AND before streaming; an unredacted copy is never written anywhere.
1. Never emitted: credential values; env values whose name matches `^NEMO_(?:[A-Z0-9]+_)*(?:API_KEY|AUTH_TOKEN|TOKEN)$` [from-node auth.js:46]; hook stdout/stderr [from-node agent.js:1030]; HTTP `Authorization`/`x-api-key` headers; thinking `signature`.
2. String values anywhere in `payload` matching built-in patterns are replaced by `"[REDACTED:<kind>]"`: `sk-ant-…`, `sk-…` (≥20 chars), `ghp_/gho_/github_pat_…`, `AKIA[0-9A-Z]{16}`, `-----BEGIN [A-Z ]*PRIVATE KEY-----…`, `Bearer <token>`. Browser `fill` values marked `sensitive` are always redacted.
3. Any string > 8 KiB becomes a preview (head 60 % / tail 40 %, marker `… [N chars truncated] …`, same algorithm as [from-node bash.js:18-26]) plus an `ArtifactRef`; images always become artifacts.
4. Redaction is lossy by design; tool results seen by the model are NOT redacted (the model must see real output), only their persisted/streamed copies.

## 6. Headless CLI contract [new]
`nemo run --headless --cwd <dir> [--model <provider/model>] [--mode <permission-mode>] [--criteria <file.json>] [--max-turns N] [--max-tool-calls N] [--max-wall-ms N] (--prompt <text> | --prompt-file <f> | stdin)`
- `--cwd` is REQUIRED (no implicit cwd, workspace-context.md §4).
- stdout: events only, one per line. stderr: human diagnostics, unstructured.
- No interactive approver: every `ask` resolves `auto_denied` (`reviewer:"policy:non-interactive"` in `nemo run`, which emits `tool.approval_required` + `tool.approval_resolved` for the audit trail; was `"headless"`) and the denial text goes back to the model as a tool error.

| Exit | Meaning (`task.ended.status`) |
|---|---|
| 0 | `completed` (verification passed, or not applicable — verification.md §4) |
| 1 | `failed` |
| 2 | usage or config error, no session started (Node uses 1 for this [from-node bin/nemo.js:269-685]; deliberately changed) |
| 3 | `unverified` |
| 4 | `blocked` |
| 70 | internal error (crash after the session started; `session.failed` emitted if possible) |
| 130 | `cancelled` by SIGINT/Ctrl-C [from-node bin/nemo.js:116] |
| 141 | stdout was closed by the reader (EPIPE, e.g. `nemo run ... \| head`): the task is cancelled cleanly, every event including the final ones is still persisted in the session store; `nemo doctor/models/auth` just exit 0 quietly [new] |

## 7. Versioning and compatibility [new]
- Additive changes (new `type`, new optional payload field, new enum value in a field documented as open) keep `v`. Consumers MUST ignore unknown types and unknown fields, and MUST treat unknown enum values as "other".
- Removing/renaming a field, changing a type or meaning, or making a field required bumps `v`. A producer emits one `v` per session; a store may hold sessions of different `v`.
- The JSON Schema for each payload lives in `crates/nemo-events/schema/v1/*.json` (to be generated from the Rust types); golden stream fixtures in `tests/golden/events/` pin it.

## Open
- Whether `model.delta` for `thinking` is streamed at all to non-TUI clients (doc §5.4 "never leak hidden internals").
- Live output streaming for long `Bash`/`terminal.exec` (no `tool.output` event in v1; use `terminal.logs`).
