# Spec: Verification, completion gate, self-correction, budgets — reviewed 0.1

Tags as in events.md. Node has no verification engine (doc Appendix C #11). The closest Node piece is the `/goal` judge: a small model reads a summary and decides whether a condition is met [from-node agent.js:1881-1946]. That is a model opinion, not evidence, and it is NOT a verification check here.

## 1. Types [new]

```rust
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus { Passed, Failed, Unverified, Blocked }

pub struct Criterion {
    pub id: String,                  // "c1"; user-given ids are kept
    pub text: String,
    pub source: CriterionSource,     // User | Inferred
    pub check: Option<String>,       // shell command that proves it (user-declared)
    pub status: VerificationStatus,
    pub evidence: Vec<String>,       // check ids
}

pub struct Check {
    pub id: String,                  // "k1"
    pub kind: CheckKind,             // Typecheck|Lint|Unit|Integration|Build|BrowserE2e|Http|Custom
    pub command: Option<String>,     // None for browser/http assertions
    pub cwd: PathBuf,
    pub declared_by: Declarer,       // User | Harness (inferred) | Model (verify_run)
    pub started_at: String, pub finished_at: String, pub duration_ms: u64,
    pub exit_code: Option<i32>,
    pub status: VerificationStatus,
    pub blocked_reason: Option<String>,   // e.g. "command not found: pytest"
    pub summary: String,             // <= 2 KiB, e.g. "12 passed, 1 failed: login.test.js:44"
    pub failure_fingerprint: Option<String>, // sha256 of normalised first failing lines
    pub criteria_ids: Vec<String>,
    pub artifacts: Vec<ArtifactRef>, // full stdout/stderr, screenshots, traces
}

pub struct VerificationReport {
    pub id: String, pub task_id: String, pub session_id: String,
    pub attempt: u32,                // 1-based
    pub status: VerificationStatus,
    pub criteria: Vec<Criterion>,
    pub checks: Vec<Check>,
    pub changed_files: Vec<String>,  // workspace-relative, from git diff against the session baseline
    pub limitations: Vec<String>,    // what was NOT checked, in plain words
    pub started_at: String, pub finished_at: String,
}
```
Stored in `verification_reports.report` (JSON) and announced by `verification.completed`.

## 2. Per-check status [new]
| Status | Rule |
|---|---|
| `passed` | the check ran to completion and its assertion held (exit 0 for commands) |
| `failed` | the check ran and the assertion did not hold: non-zero exit, assertion false, or timeout (a hang counts as a failure) |
| `blocked` | the check could not run for environmental reasons: command/runtime missing, missing credentials or env var, dependency install failed, port unavailable, network denied by policy, approval denied |
| `unverified` | it ran but proves nothing: zero tests collected, output unparseable, or the check does not cover any changed file or criterion |

Evidence MUST come from runtime-executed checks (TerminalRuntime / browser runtime). Assistant text such as "tests pass" is never evidence. Command output classification uses exit code first and simple per-runner parsing second (`node --test`, `cargo test`, `pytest`, `jest`/`vitest` summaries); parsing may only downgrade passed → unverified (e.g. "0 tests"), never upgrade.

## 3. Aggregation [new]
Criterion status = best evidence: `passed` if at least one linked check passed and no linked check failed; otherwise, by precedence, `failed` > `blocked` > `unverified`.
Report status, first match wins:
1. any check `failed` → `failed`
2. any user criterion `blocked` → `blocked`
3. any user criterion not `passed` → `unverified`
4. no user criteria: no check failed and ≥ 1 check `passed` (a pass is authoritative: it ran to completion, exit 0, not downgraded by output parsing) → `passed`; `limitations` += "no explicit acceptance criteria; inferred checks only" and one line per `blocked`/`unverified` check. Checks that could not run do not turn a pass into `unverified`, they are listed. `nemo.verification.inferred_only: "cap_unverified"` restores the old cap (inferred checks alone → `unverified`).
5. otherwise → `unverified`

## 4. Completion gate [new]
The gate runs when the model ends a turn with `end_turn` and no tool calls (Node simply stops the turn there [from-node agent.js:3166]).
1. Gate applies if the task has user criteria OR a side effect happened: a successful Write/Edit, or an Execute call that is not read-only/build/test/lint (`Bash` commands classified by `nemo_permissions::is_auto_safe_bash` do not count). Otherwise (pure Q&A, read-only exploration, `ls`, `cargo test`) the task ends `completed` with `verification_report_id: null`, and `task.ended.reason = "no side effects"`. Once a side effect happened the gate always applies: if no check is declared, inferred or relevant to the changed files (e.g. only `hello.txt` changed in a Rust project: `cargo` checks need a changed `.rs`/`.toml`/`.lock`; npm needs js/ts/json; python needs `.py`; go needs `.go`/`go.mod`; `make test` needs a non-doc file), nothing ran, so the report is `unverified` (limitation "no checks could be planned or run", `changed_files` listed) and the task ends `unverified` (exit 3) after the one feedback round. Relevance only selects which inferred checks run; it never turns a changed workspace into `completed`. When `changed_files` is empty (a shell changed things untracked) every detected check is relevant.
2. Check plan, in order: user-declared `check` commands → checks the model registered via `verify_run` → inferred checks for the changed files (discovered from `package.json` scripts `test|lint|typecheck|build`, `Cargo.toml` → `cargo test`, `pyproject.toml`/`pytest.ini` → `pytest`, `go.mod` → `go test ./...`). Inferred commands go through policy like any Execute call; user-declared ones count as pre-approved by declaration. **Approval:** commands that policy would allow (rules, mode) run without a dialog, so an earlier `allow always` for `Bash(cargo test:*)` also covers the gate and `verify_run`. All remaining checks are asked in ONE dialog (the commands as a multi-line `Bash` input) with allow once / allow always (always adds one prefix rule per simple command, max 5). An approved set is remembered for the session (the gate only; a later Bash call follows the normal rules); a refusal blocks those checks for this evaluation. Headless runs ask nothing and report `needs_approval`. While the dialog is open the session is `waiting_approval` and returns to `verifying`.
3. Task status from the report: `passed` → `completed`; `failed` → self-correction (§5) or `failed`; `blocked` → `blocked`; `unverified` → one self-correction round in which the model is asked to add or run a focused check, then `unverified`.
4. The final assistant message is shown as-is, but `task.ended.status` comes from the gate, never from the model's wording.

## 5. Bounded self-correction [new]
- On a `failed`/`unverified` report with `usage.verify_attempts < budget.max_verify_attempts` (default 3), the harness appends a user-role message: report summary, failing check ids, the last 4 KiB of each failing output, and artifact ids for the rest. Then the loop goes back to `running`.
- No-progress stop: if two consecutive attempts give the same set of `failure_fingerprint`s and `changed_files` did not change, stop with `failed`, reason `no_progress`.
- Regression guard: a check that passed in attempt *n* and fails in *n+1* is reported in `limitations` as a regression even if the target check now passes.

## 6. Budgets and stop conditions [new unless tagged; doc §6.2]
| Budget | Default | On exhaustion |
|---|---|---|
| `max_turns` (model calls per task) | 50 [from-node agent.js:45] | stop; run the gate once if side effects happened; status from the gate, at best `unverified` if the gate cannot finish |
| `max_tool_calls` | 200 | same |
| `max_wall_ms` | none (headless eval sets it) | same; running processes are killed |
| `max_cost_usd` | none | same |
| `max_verify_attempts` | 3 | final report status |
| model retries per call | 3, backoff ~1/2/4 s with jitter, only 429/5xx/529/network errors [from-node agent.js:50, 176-195, 222-232] | `failed`, reason `provider_error` |
| per-check timeout | 600 000 ms (Bash max [from-node tools/bash.js:10]) | check `failed` (timeout) |

Other stops: user cancel → `cancelled`; approval needed but impossible (headless) and the model cannot continue → `blocked`, reason `approval_unavailable`; workspace unsafe (root missing, outside-root writes required) → `blocked`; context overflow after one automatic compaction [from-node agent.js:3154-3160] → `failed`, reason `context_overflow`. Every stop emits `budget.exceeded` (if a budget) and then `task.ended` with `reason`.

## 7. Report to the user
`task.ended` plus the report must state: changed files, checks run with exact commands and exit codes, what passed or failed, artifacts, and `limitations` (doc §5.11: "A pass in one test suite is not proof that all behavior is correct").

## Open
- Rule 4 of §3 (inferred-only can reach `passed`) versus "unverified unless the user gave criteria": product decision (doc §13.7).
- Is a dev server that never becomes ready `failed` (often the bug itself) or `blocked`? Currently `failed` (timeout).
