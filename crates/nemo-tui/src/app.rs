//! The UI state machine. No I/O: keys, events and time go in, `Cmd`s and frames come out, so the
//! whole app is testable without a terminal. The app never owns a process or a model request;
//! it asks the runtime to start a task and cancels it through the task's token.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nemo_events::{Budget, Envelope, Event};
use nemo_harness::{Approval, DENY_MESSAGE, TaskOutcome, UserReply};
use nemo_permissions::{Mode, Suggestion};
use nemo_tools::NeedsUser;
use nemo_tui_base::complete::{self, Command as CmdDef, StdFs};
use nemo_tui_base::editor::LineEditor;
use nemo_tui_base::keys::{Key, MouseAction, Named, WheelDir};
use nemo_tui_base::screen::adoptable_size;
use nemo_tui_base::selection::{
    self as sel, Act, Event as SelEvent, Mode as SelMode, Point, Selection, selection_transition,
};
use nemo_tui_render::agent::{AgentOpts, EditorView};
use nemo_tui_render::agent::{
    INPUT_VIEW_ROWS, Opt, Pending, QOption, QState, Question as RQuestion, Search, Sug,
    Suggestions as RSug, body_text_rows,
};
use nemo_tui_render::dash::{
    DashState, DashView, Focus, Frame, Live, MIN_COLS, MIN_ROWS, SessionRow, Usage, Workspace,
    render_frame, workspace_key,
};
use nemo_tui_render::dialogs::{HELP, normalize_mode};
use nemo_tui_render::model::{
    self, Session, apply_event, parse_ts, session_from_events, session_phase,
};
use nemo_tui_render::picker::{Picker, PickerItem};
use nemo_tui_render::theme::{self, T_ERROR, T_SUCCESS};
use nemo_tui_render::util::basename;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::chat::Chat;
use crate::link::{ApprovalAsk, InputAsk};
use crate::links::{OpenTarget, existing_path, path_at, url_at};
use crate::sanitize::{clean_input, clean_line, clean_text};
use crate::status::{SandboxStatus, status_line};
use crate::{ExitReason, SavedSession};

const QUIT_ARM_MS: i64 = 1500;
const SCROLL_STEP: usize = 10;
const WHEEL_LINES: usize = 3;
/// Enter and digits cannot answer a dialog for this long after it comes to the front.
const ARM_MS: i64 = 400;
/// Dialogs also stay unarmed this long after any paste activity: the tail of a paste that forged
/// its end marker (`\r`, digits) arrives later than the parser's grace window.
const PASTE_QUIET_MS: i64 = 300;
const TOAST_MS: i64 = 1800;

/// Modes the UI offers. `auto` and `bypassPermissions` are not available interactively yet.
const OFFERED_MODES: [Mode; 4] = [Mode::Default, Mode::AcceptEdits, Mode::Plan, Mode::DontAsk];
const CYCLE: [Mode; 3] = [Mode::Default, Mode::AcceptEdits, Mode::Plan];

pub(crate) struct AppConfig {
    pub cwd: String,
    pub session_id: String,
    pub model: String,
    pub version: String,
    pub budget: Budget,
    pub sandbox: SandboxStatus,
    pub mode: Mode,
    pub models: Vec<String>,
    /// Skills / custom commands offered next to the local ones (sent to the model as text).
    pub extra_commands: Vec<CmdDef>,
    pub saved: Vec<SavedSession>,
    pub home: Option<PathBuf>,
    pub tz_offset_min: i32,
}

/// Effects for the runtime.
#[derive(Debug)]
pub(crate) enum Cmd {
    StartTask {
        text: String,
        cancel: CancellationToken,
    },
    ClearConversation,
    Copy(String),
    Open(OpenTarget),
    SwitchModel(String),
    /// Leave and re-enter the alternate screen (ctrl+l).
    ResetScreen,
}

enum DialogKind {
    Approval {
        id: String,
        suggestions: Vec<Suggestion>,
        reply: Option<oneshot::Sender<Approval>>,
    },
    Question {
        id: String,
        questions: Vec<nemo_tools::Question>,
        reply: Option<oneshot::Sender<Option<UserReply>>>,
    },
    Plan {
        id: String,
        reply: Option<oneshot::Sender<Option<UserReply>>>,
    },
}

struct Dialog {
    kind: DialogKind,
    pending: Pending,
    deadline_ms: Option<i64>,
}

impl Dialog {
    fn id(&self) -> &str {
        match &self.kind {
            DialogKind::Approval { id, .. }
            | DialogKind::Question { id, .. }
            | DialogKind::Plan { id, .. } => id,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Model,
    Mode,
    Command,
    Folder,
    Resume,
}

struct PickerState {
    kind: PickerKind,
    picker: Picker,
    /// One value per item of `picker.items`, same order.
    values: Vec<String>,
    dir: PathBuf,
}

struct HistSearch {
    query: String,
    matched: String,
    at: i64,
    from: i64,
}

#[derive(Default)]
struct SelState {
    mode: Option<SelMode>,
    selection: Option<Selection>,
    press: Option<Point>,
    extending: bool,
    pointer: Option<Point>,
}

impl SelState {
    fn mode(&self) -> SelMode {
        self.mode.unwrap_or(SelMode::Idle)
    }
}

struct Toast {
    text: String,
    until: i64,
    bad: bool,
}

struct TaskState {
    cancel: CancellationToken,
    cancelling: bool,
}

pub(crate) struct App {
    pub cfg: AppConfig,
    pub quit: Option<ExitReason>,
    pub mode: Mode,
    pub model: String,
    chat: Chat,
    activity: Session,
    editor: LineEditor,
    scroll: usize,
    expanded: bool,
    dialogs: Vec<Dialog>,
    /// Front dialog id and the time from which Enter / digits may answer it.
    armed: Option<(String, i64)>,
    paste_quiet_until: i64,
    picker: Option<PickerState>,
    search: Option<HistSearch>,
    sug: Option<complete::Suggestions>,
    sug_view: Vec<Sug>,
    sug_selected: usize,
    sug_dismissed: Option<String>,
    task: Option<TaskState>,
    ran_task: bool,
    quit_armed_at: Option<i64>,
    toast: Option<Toast>,
    sel: SelState,
    /// Last drawn frame: hit testing and copy read it.
    frame: Option<Frame>,
    lines: Vec<String>,
    size: (u32, u32),
    commands: Vec<CmdDef>,
}

fn local_commands() -> Vec<CmdDef> {
    let c = |name: &str, d: &str, hint: &str| CmdDef {
        name: name.into(),
        aliases: vec![],
        description: d.into(),
        argument_hint: hint.into(),
        local: true,
    };
    vec![
        c("help", "commands and keys", ""),
        c("clear", "start a new conversation", ""),
        c("new", "open a session in another folder", ""),
        c("resume", "continue a saved session", "[session id]"),
        c("model", "pick a model, or change it directly", "[model]"),
        c(
            "mode",
            "pick a permission mode",
            "[default | acceptEdits | plan | dontAsk]",
        ),
        c(
            "status",
            "session, model, sandbox, budgets, verification",
            "",
        ),
        c(
            "focus",
            "toggle focus view (prompt, summary and response only)",
            "[on|off]",
        ),
        c("exit", "quit", ""),
    ]
}

fn mode_str(m: Mode) -> &'static str {
    m.as_str()
}

fn is_key(k: &Key, n: Named) -> bool {
    matches!(k, Key::Named(x) if *x == n)
}

#[cfg(test)]
mod tests;

impl App {
    pub fn new(cfg: AppConfig, replay: &[Envelope], now: i64) -> Self {
        let mut commands = local_commands();
        commands.extend(cfg.extra_commands.iter().cloned());
        let mut chat = Chat::default();
        chat.agent.id = cfg.session_id.clone();
        chat.agent.kind = "main".into();
        chat.agent.cwd = clean_line(&cfg.cwd);
        chat.agent.model = clean_line(&cfg.model);
        chat.agent.mode = mode_str(cfg.mode).into();
        chat.agent.version = cfg.version.clone();
        chat.agent.status = "idle".into();
        let mut activity = session_from_events(&cfg.session_id, replay.iter());
        activity.cwd = clean_line(&cfg.cwd);
        activity.project = clean_line(&basename(&cfg.cwd));
        if activity.model.is_empty() {
            activity.model = clean_line(&cfg.model);
        }
        for e in replay {
            chat.apply(e, parse_ts(&e.ts).unwrap_or(now));
        }
        chat.agent.status = "idle".into();
        let ran_task = replay
            .iter()
            .any(|e| matches!(e.event, Event::TaskEnded { .. }));
        Self {
            mode: cfg.mode,
            model: cfg.model.clone(),
            cfg,
            quit: None,
            chat,
            activity,
            editor: LineEditor::new(),
            scroll: 0,
            expanded: false,
            dialogs: vec![],
            armed: None,
            paste_quiet_until: 0,
            picker: None,
            search: None,
            sug: None,
            sug_view: vec![],
            sug_selected: 0,
            sug_dismissed: None,
            task: None,
            ran_task,
            quit_armed_at: None,
            toast: None,
            sel: SelState::default(),
            frame: None,
            lines: vec![],
            size: (80, 24),
            commands,
        }
    }

    // ------------------------------------------------------------------ inputs from the harness

    pub fn on_event(&mut self, env: &Envelope, now: i64) {
        apply_event(&mut self.activity, env);
        self.chat.apply(env, now);
        if matches!(
            env.event,
            Event::UserMessage { .. } | Event::TaskEnded { .. }
        ) {
            self.scroll = 0;
        }
    }

    pub fn note(&mut self, text: impl Into<String>, tone: &str) {
        self.chat.note(text, tone);
    }

    pub fn startup_notes(&mut self, notes: &[String]) {
        for n in notes {
            self.chat.startup_note(n);
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, now: i64, bad: bool) {
        self.toast = Some(Toast {
            text: clean_line(&text.into()),
            until: now + TOAST_MS,
            bad,
        });
    }

    pub fn on_approval(&mut self, ask: ApprovalAsk, now: i64) {
        let ApprovalAsk {
            req,
            reply,
            deadline_ms,
        } = ask;
        let suggestions: Vec<Value> = req.suggestions.iter().map(suggestion_view).collect();
        let mut pending = Pending {
            kind: "permission".into(),
            dialog: "permission".into(),
            tool: clean_line(&req.tool),
            input: clean_input(&req.input),
            suggestions,
            status: "pending".into(),
            prompt: clean_line(&req.reason),
            display_name: req.agent_id.clone().unwrap_or_default(),
            ..Pending::default()
        };
        pending.hydrate();
        self.dialogs.push(Dialog {
            kind: DialogKind::Approval {
                id: req.approval_id,
                suggestions: req.suggestions,
                reply: Some(reply),
            },
            pending,
            deadline_ms,
        });
        self.sync_arming(now);
    }

    /// Whenever a different dialog comes to the front (opened, promoted after an answer or a
    /// withdrawal) Enter and digits are ignored for [`ARM_MS`]: a keystroke typed before the
    /// dialog appeared must not answer it. Esc / ctrl-c still cancel.
    fn sync_arming(&mut self, now: i64) {
        let front = self.dialogs.first().map(Dialog::id);
        if front != self.armed.as_ref().map(|a| a.0.as_str()) {
            self.armed = front.map(|id| (id.to_string(), now + ARM_MS));
        }
    }

    fn armed_in(&self, now: i64) -> Option<i64> {
        let open = self.armed.as_ref().map_or(0, |a| a.1);
        Some(open.max(self.paste_quiet_until) - now).filter(|ms| *ms > 0)
    }

    /// Paste start / bytes / end / forged marker / held sequence seen by the key parser.
    pub fn paste_activity(&mut self, now: i64) {
        self.paste_quiet_until = now + PASTE_QUIET_MS;
    }

    pub fn on_input(&mut self, ask: InputAsk, now: i64) {
        let InputAsk { id, req, reply } = ask;
        match req {
            NeedsUser::AskUserQuestion { questions } => {
                let rq: Vec<RQuestion> = questions
                    .iter()
                    .map(|q| RQuestion {
                        question: clean_text(&q.question),
                        header: q.header.as_deref().map(clean_line).unwrap_or_default(),
                        multi_select: q.multi_select,
                        options: q
                            .options
                            .iter()
                            .map(|o| QOption {
                                label: clean_line(&o.label),
                                description: o
                                    .description
                                    .as_deref()
                                    .map(clean_line)
                                    .unwrap_or_default(),
                            })
                            .collect(),
                    })
                    .collect();
                let mut pending = Pending {
                    kind: "permission".into(),
                    dialog: "question".into(),
                    tool: "AskUserQuestion".into(),
                    status: "pending".into(),
                    questions: rq,
                    ..Pending::default()
                };
                pending.hydrate();
                self.dialogs.push(Dialog {
                    kind: DialogKind::Question {
                        id,
                        questions,
                        reply: Some(reply),
                    },
                    pending,
                    deadline_ms: None,
                });
            }
            NeedsUser::ExitPlanMode { plan } => {
                let opt = |key: &str, label: &str, mode: Option<&str>| Opt {
                    key: key.into(),
                    label: label.into(),
                    mode: mode.map(String::from),
                };
                let pending = Pending {
                    kind: "permission".into(),
                    dialog: "plan".into(),
                    tool: "ExitPlanMode".into(),
                    input: json!({ "plan": clean_text(&plan) }),
                    status: "pending".into(),
                    options: Some(vec![
                        opt("manual", "Yes, manually approve edits", Some("default")),
                        opt("accept", "Yes, and auto-accept edits", Some("acceptEdits")),
                        opt("keep", "No, keep planning (esc)", None),
                    ]),
                    ..Pending::default()
                };
                self.dialogs.push(Dialog {
                    kind: DialogKind::Plan {
                        id,
                        reply: Some(reply),
                    },
                    pending,
                    deadline_ms: None,
                });
            }
        }
        self.sync_arming(now);
    }

    /// The harness side gave up on a dialog (answered elsewhere, cancelled, expired).
    pub fn on_withdraw(&mut self, id: &str, now: i64) {
        let n = self.dialogs.len();
        self.dialogs.retain(|d| d.id() != id);
        if self.dialogs.len() != n {
            self.chat
                .note("the request was withdrawn (cancelled or expired)", "warn");
        }
        self.sync_arming(now);
    }

    pub fn task_running(&self) -> bool {
        self.task.is_some()
    }

    pub fn cancel_task(&mut self) {
        self.interrupt();
    }

    /// A reply is merely streaming: nothing else needs fast frames.
    pub fn calm_streaming(&self) -> bool {
        self.chat.agent.status != "idle"
            && self.dialogs.is_empty()
            && self.toast.is_none()
            && self.sel.mode() != SelMode::Dragging
    }

    pub fn task_done(&mut self, result: &Result<TaskOutcome, String>, now: i64) {
        self.task = None;
        self.dialogs.clear();
        self.armed = None;
        self.ran_task = true;
        self.chat.agent.status = "idle".into();
        self.chat.agent.interrupting = false;
        if let Err(e) = result {
            self.chat.note(format!("task failed to run: {e}"), "error");
        }
        let _ = now;
    }

    // ------------------------------------------------------------------ time

    pub fn animating(&self) -> bool {
        self.chat.agent.status != "idle"
            || !self.dialogs.is_empty()
            || self.toast.is_some()
            || self.sel.mode() == SelMode::Dragging
    }

    /// Returns true when something on screen changes without input.
    pub fn tick(&mut self, now: i64) -> bool {
        let mut dirty = self.animating();
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
            dirty = true;
        }
        // approvals expire on the harness side; drop the dialog at the same time
        for d in &self.dialogs {
            if d.deadline_ms.is_some_and(|dl| now >= dl) {
                dirty = true;
            }
        }
        if self.sel.mode() == SelMode::Dragging
            && let (Some(g), Some(p)) = (self.geom(), self.sel.pointer)
        {
            let step = sel::edge_scroll_step(p.y, g.top, g.top + g.h - 1);
            let before = self.scroll;
            if step != 0 {
                let max = self.frame.as_ref().map_or(0, |f| f.rects.max_scroll);
                self.scroll = (before as i64 - step).clamp(0, max as i64) as usize;
                if self.scroll != before
                    && let Some(g) = self.geom()
                    && let Some(s) = self.sel.selection.as_mut()
                {
                    s.b = sel::point_to_content(&g, p);
                }
            }
        }
        dirty
    }

    pub fn set_size(&mut self, cols: u32, rows: u32) {
        if self.size != (cols, rows) {
            self.size = (cols, rows);
            self.clear_selection();
        }
    }

    // ------------------------------------------------------------------ keys

    pub fn on_key(&mut self, k: Key, now: i64) -> Vec<Cmd> {
        let mut out = vec![];
        if matches!(k, Key::Paste(_)) {
            self.paste_activity(now);
        }
        match &k {
            Key::CursorPosition { row, col } => {
                if let Some((c, r)) = adoptable_size(*col, *row, self.size) {
                    self.set_size(c, r);
                }
                return out;
            }
            Key::Mouse { .. } | Key::MouseDrag { .. } | Key::Wheel { .. } => {
                self.mouse(&k, now, &mut out);
                return out;
            }
            _ => {}
        }
        if is_key(&k, Named::CtrlL) {
            out.push(Cmd::ResetScreen);
            return out;
        }
        let paging = is_key(&k, Named::PageUp) || is_key(&k, Named::PageDown);
        if selection_transition(self.sel.mode(), SelEvent::Key { page: paging }).1 == Act::Clear {
            self.clear_selection();
        }
        if !self.dialogs.is_empty() {
            self.dialog_key(&k, now);
            self.sync_arming(now);
        } else if self.picker.is_some() {
            self.picker_key(&k, now, &mut out);
        } else if self.search.is_some() {
            self.search_key(&k);
        } else if !self.suggestion_key(&k, now, &mut out) {
            self.agent_key(&k, now, &mut out);
        }
        self.refresh_suggestions(now);
        out
    }

    fn running(&self) -> bool {
        self.task.is_some()
    }

    /// Esc: cancel the running task. Returns whether there was one.
    fn interrupt(&mut self) -> bool {
        match &mut self.task {
            Some(t) => {
                if !t.cancelling {
                    t.cancelling = true;
                    t.cancel.cancel();
                    self.chat.agent.interrupting = true;
                }
                true
            }
            None => false,
        }
    }

    fn agent_key(&mut self, k: &Key, now: i64, out: &mut Vec<Cmd>) {
        match k {
            Key::Text(t) => {
                if t == "?" && self.editor.chars.is_empty() {
                    self.run_slash("help", "", now, out);
                } else {
                    self.editor.insert(t);
                }
            }
            Key::Paste(t) => {
                if !self.editor.insert(t) {
                    self.chat
                        .note("paste is too large · use @file to attach it", "warn");
                }
            }
            Key::Named(n) => self.named(*n, now, out),
            _ => {}
        }
    }

    fn named(&mut self, n: Named, now: i64, out: &mut Vec<Cmd>) {
        use Named::*;
        let e = &mut self.editor;
        match n {
            Enter => {
                if e.chars.last() == Some(&'\\') {
                    e.backspace();
                    e.insert("\n");
                } else {
                    self.submit(now, out);
                }
            }
            AltEnter | ShiftEnter | CtrlJ => {
                e.insert("\n");
            }
            Backspace => e.backspace(),
            Delete => e.del(),
            Left => e.left(),
            Right => e.right(),
            CtrlLeft => e.word_left(),
            CtrlRight => e.word_right(),
            Home | CtrlA => e.home(),
            End | CtrlE => e.end(),
            CtrlU => e.kill_start(),
            CtrlK => e.kill_end(),
            CtrlW => e.kill_word(),
            Up => self.vertical(true),
            Down => self.vertical(false),
            PageUp => {
                let max = self.frame.as_ref().map_or(0, |f| f.rects.max_scroll);
                self.scroll = (self.scroll + SCROLL_STEP).min(max);
            }
            PageDown => self.scroll = self.scroll.saturating_sub(SCROLL_STEP),
            ShiftTab => self.cycle_mode(),
            CtrlO => self.expanded = !self.expanded,
            Escape => {
                self.interrupt();
            }
            CtrlR => {
                let from = e.history.len() as i64 - 1;
                let (matched, at) = self.search_history("", from);
                self.search = Some(HistSearch {
                    query: String::new(),
                    matched,
                    at,
                    from,
                });
            }
            CtrlP => self.open_command_picker(),
            CtrlN => self.open_folder_picker(PathBuf::from(&self.cfg.cwd)),
            CtrlC => {
                if !self.editor.chars.is_empty() {
                    self.editor.clear();
                } else if let Some(t) = &self.task {
                    // first press cancels, a second one while it winds down quits
                    if t.cancelling {
                        self.quit = Some(ExitReason::Quit);
                    } else {
                        self.interrupt();
                    }
                    self.quit_armed_at = None;
                } else if self.quit_armed_at.is_some_and(|t| now - t < QUIT_ARM_MS) {
                    self.quit = Some(ExitReason::Quit);
                } else {
                    self.quit_armed_at = Some(now);
                    self.chat.note("press ctrl+c again to quit", "");
                }
            }
            CtrlD => {
                if self.editor.chars.is_empty() {
                    self.quit = Some(ExitReason::Quit);
                } else {
                    self.editor.del();
                }
            }
            _ => {}
        }
    }

    /// Up/Down: inside a multi-line prompt move by line, else walk the history.
    fn vertical(&mut self, up: bool) {
        let e = &mut self.editor;
        let start = |e: &LineEditor| {
            e.chars[..e.cur]
                .iter()
                .rposition(|c| *c == '\n')
                .map_or(0, |i| i + 1)
        };
        let end = |e: &LineEditor| {
            e.chars[e.cur..]
                .iter()
                .position(|c| *c == '\n')
                .map_or(e.chars.len(), |i| e.cur + i)
        };
        if up && e.multiline() && start(e) > 0 {
            let col = e.cur - start(e);
            e.cur = start(e) - 1;
            e.cur = (start(e) + col).min(end(e));
        } else if !up && e.multiline() && end(e) < e.chars.len() {
            let col = e.cur - start(e);
            e.cur = end(e) + 1;
            e.cur = (start(e) + col).min(end(e));
        } else if up {
            e.up();
        } else {
            e.down();
        }
    }

    fn cycle_mode(&mut self) {
        let i = CYCLE
            .iter()
            .position(|m| *m == self.mode)
            .map_or(0, |i| i + 1);
        self.mode = CYCLE[i % CYCLE.len()];
        self.mode_changed();
    }

    fn mode_changed(&mut self) {
        self.chat.agent.mode = mode_str(self.mode).into();
    }

    fn submit(&mut self, now: i64, out: &mut Vec<Cmd>) {
        let text = self.editor.text();
        if text.trim().is_empty() {
            return;
        }
        if let Some(rest) = text.trim_start().strip_prefix('/') {
            let (name, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if self.cfg.extra_commands.iter().all(|c| c.name != name) {
                self.editor.submit();
                self.run_slash(name, arg.trim(), now, out);
                return;
            }
        }
        if self.running() {
            self.toast("a task is running · esc cancels it", now, true);
            return;
        }
        self.editor.submit();
        let cancel = CancellationToken::new();
        self.task = Some(TaskState {
            cancel: cancel.clone(),
            cancelling: false,
        });
        self.chat.agent.status = "running".into();
        self.chat.agent.turn_start = now;
        self.chat.agent.turn_mode = Some(mode_str(self.mode).into());
        self.scroll = 0;
        self.clear_selection();
        out.push(Cmd::StartTask { text, cancel });
    }

    // ------------------------------------------------------------------ slash commands

    fn out_item(&mut self, text: impl Into<String>, error: bool) {
        self.chat.push(item! {
            kind: "command-output".into(),
            text: clean_text(&text.into()),
            error,
        });
    }

    fn run_slash(&mut self, name: &str, arg: &str, now: i64, out: &mut Vec<Cmd>) {
        match name {
            "help" => {
                self.chat.push(item! {
                    kind: "help".into(),
                    lines: HELP.iter().map(|s| (*s).to_string()).collect(),
                });
            }
            "exit" | "quit" => self.quit = Some(ExitReason::Quit),
            "clear" => {
                if self.running() {
                    self.out_item("/clear: a task is running · esc cancels it first", true);
                } else {
                    self.chat.agent.items.clear();
                    self.chat.note("new conversation", "");
                    out.push(Cmd::ClearConversation);
                }
            }
            "new" => self.open_folder_picker(PathBuf::from(&self.cfg.cwd)),
            "resume" => {
                if arg.is_empty() {
                    self.open_resume_picker();
                } else {
                    self.quit = Some(ExitReason::Resume(arg.to_string()));
                }
            }
            "model" => {
                if arg.is_empty() {
                    self.open_model_picker();
                } else {
                    self.switch_model(arg, out);
                }
            }
            "mode" => {
                if arg.is_empty() {
                    self.open_mode_picker();
                } else {
                    self.set_mode_arg(arg);
                }
            }
            "status" => self.status_item(),
            "focus" => {
                let on = match arg {
                    "on" => true,
                    "off" => false,
                    _ => !self.chat.agent.focus_view,
                };
                self.chat.agent.focus_view = on;
            }
            other => self.out_item(format!("unknown command: /{other} (try /help)"), true),
        }
        let _ = now;
    }

    fn set_mode_arg(&mut self, arg: &str) {
        match normalize_mode(Some(arg)).and_then(Mode::parse) {
            Some(m) if OFFERED_MODES.contains(&m) => {
                self.mode = m;
                self.mode_changed();
            }
            Some(m) => self.out_item(
                format!(
                    "mode {} is not available in the interactive app",
                    m.as_str()
                ),
                true,
            ),
            None => self.out_item(format!("unknown mode: {arg}"), true),
        }
    }

    fn switch_model(&mut self, name: &str, out: &mut Vec<Cmd>) {
        if self.running() {
            self.out_item("/model: a task is running · esc cancels it first", true);
        } else {
            out.push(Cmd::SwitchModel(name.to_string()));
        }
    }

    pub fn model_switched(&mut self, label: &str) {
        self.model = label.to_string();
        self.chat.agent.model = clean_line(label);
        self.activity.model = clean_line(label);
        self.chat.note(format!("model: {label}"), "");
    }

    fn status_item(&mut self) {
        let t = &self.chat.totals;
        let v = self
            .chat
            .verification
            .as_ref()
            .map_or("none".to_string(), |v| {
                format!("{}: {}", v.status, v.summary)
            });
        let rows = [
            ("session", self.cfg.session_id.clone()),
            ("model", clean_line(&self.model)),
            ("folder", clean_line(&self.cfg.cwd)),
            ("mode", mode_str(self.mode).to_string()),
            ("sandbox", format!("{:?}", self.cfg.sandbox)),
            (
                "budget",
                format!(
                    "turns {}/{} · tool calls {}/{}",
                    t.turns,
                    self.cfg.budget.max_turns,
                    t.tool_calls,
                    self.cfg.budget.max_tool_calls
                ),
            ),
            ("verification", v),
        ];
        self.chat.push(item! {
            kind: "info".into(),
            title: "Status".into(),
            rows: rows
                .iter()
                .map(|(k, v)| vec![(*k).to_string(), clean_text(v)])
                .collect(),
        });
    }

    // ------------------------------------------------------------------ pickers

    fn open_picker(
        &mut self,
        kind: PickerKind,
        title: &str,
        sub: &str,
        items: Vec<(PickerItem, String)>,
        dir: PathBuf,
    ) {
        let (pi, values): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        self.picker = Some(PickerState {
            kind,
            picker: Picker::new(title, title, sub, pi),
            values,
            dir,
        });
    }

    fn open_model_picker(&mut self) {
        let items = self
            .cfg
            .models
            .iter()
            .map(|m| {
                (
                    PickerItem {
                        label: m.clone(),
                        detail: String::new(),
                        current: *m == self.model,
                    },
                    m.clone(),
                )
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.out_item("/model: no models were provided by the host", true);
        } else {
            self.open_picker(
                PickerKind::Model,
                "Model",
                "enter selects",
                items,
                PathBuf::new(),
            );
        }
    }

    fn open_mode_picker(&mut self) {
        let items = OFFERED_MODES
            .iter()
            .map(|m| {
                (
                    PickerItem {
                        label: nemo_tui_render::dialogs::mode_name(Some(m.as_str())),
                        detail: m.as_str().into(),
                        current: *m == self.mode,
                    },
                    m.as_str().to_string(),
                )
            })
            .collect();
        self.open_picker(
            PickerKind::Mode,
            "Permission mode",
            "applies from the next task",
            items,
            PathBuf::new(),
        );
    }

    fn open_command_picker(&mut self) {
        let items = self
            .commands
            .iter()
            .map(|c| {
                (
                    PickerItem {
                        label: format!("/{}", c.name),
                        detail: c.description.clone(),
                        current: false,
                    },
                    c.name.clone(),
                )
            })
            .collect();
        self.open_picker(
            PickerKind::Command,
            "Commands",
            "enter inserts",
            items,
            PathBuf::new(),
        );
    }

    fn open_resume_picker(&mut self) {
        let items: Vec<_> = self
            .cfg
            .saved
            .iter()
            .map(|s| {
                (
                    PickerItem {
                        label: format!(
                            "{}  {}",
                            s.id.chars().take(8).collect::<String>(),
                            clean_line(&s.title)
                        ),
                        detail: format!("{} · {}", clean_line(&s.cwd), s.updated_at),
                        current: s.id == self.cfg.session_id,
                    },
                    s.id.clone(),
                )
            })
            .collect();
        if items.is_empty() {
            self.out_item("/resume: no saved sessions", true);
        } else {
            self.open_picker(
                PickerKind::Resume,
                "Resume a session",
                "enter continues it",
                items,
                PathBuf::new(),
            );
        }
    }

    fn open_folder_picker(&mut self, dir: PathBuf) {
        let mut items = vec![
            (
                PickerItem {
                    label: "✓ use this folder".into(),
                    detail: dir.display().to_string(),
                    current: false,
                },
                "use".to_string(),
            ),
            (
                PickerItem {
                    label: "..".into(),
                    detail: "parent folder".into(),
                    current: false,
                },
                "up".to_string(),
            ),
        ];
        for name in list_dirs(&dir) {
            items.push((
                PickerItem {
                    label: format!("{}/", clean_line(&name)),
                    detail: String::new(),
                    current: false,
                },
                format!("cd:{name}"),
            ));
        }
        let mut picker = Picker::new(
            "folder",
            "New session in folder",
            &dir.display().to_string(),
            vec![],
        );
        let (pi, values): (Vec<_>, Vec<_>) = items.into_iter().unzip();
        picker.items = pi;
        picker.choice = 0;
        self.picker = Some(PickerState {
            kind: PickerKind::Folder,
            picker,
            values,
            dir,
        });
    }

    fn picker_key(&mut self, k: &Key, now: i64, out: &mut Vec<Cmd>) {
        let Some(ps) = self.picker.as_mut() else {
            return;
        };
        let p = &mut ps.picker;
        match k {
            Key::Named(Named::Escape | Named::CtrlC) => self.picker = None,
            Key::Named(Named::Up) => p.move_by(-1),
            Key::Named(Named::Down) => p.move_by(1),
            Key::Named(Named::PageUp) => p.move_by(-(p.window_size.max(1) as i64)),
            Key::Named(Named::PageDown) => p.move_by(p.window_size.max(1) as i64),
            Key::Named(Named::Home) => p.home(),
            Key::Named(Named::End) => p.end(),
            Key::Named(Named::Backspace) => p.backspace(),
            Key::Text(t) | Key::Paste(t) => p.type_text(&t.replace('\n', " ")),
            Key::Named(Named::Enter) => {
                let Some(item) = p.filtered().get(p.choice).copied() else {
                    return;
                };
                let Some(idx) = p.items.iter().position(|i| std::ptr::eq(i, item)) else {
                    return;
                };
                let value = ps.values[idx].clone();
                let (kind, dir) = (ps.kind, ps.dir.clone());
                self.pick(kind, &value, dir, now, out);
            }
            _ => {}
        }
    }

    fn pick(&mut self, kind: PickerKind, value: &str, dir: PathBuf, now: i64, out: &mut Vec<Cmd>) {
        match kind {
            PickerKind::Folder => match value {
                "use" => {
                    self.picker = None;
                    self.quit = Some(ExitReason::OpenFolder(dir));
                }
                "up" => {
                    let parent = dir.parent().map_or(dir.clone(), Path::to_path_buf);
                    self.open_folder_picker(parent);
                }
                v => {
                    let name = v.strip_prefix("cd:").unwrap_or(v);
                    self.open_folder_picker(dir.join(name));
                }
            },
            PickerKind::Resume => {
                self.picker = None;
                self.quit = Some(ExitReason::Resume(value.to_string()));
            }
            PickerKind::Model => {
                self.picker = None;
                self.switch_model(value, out);
            }
            PickerKind::Mode => {
                self.picker = None;
                self.set_mode_arg(value);
            }
            PickerKind::Command => {
                self.picker = None;
                let c = self.commands.iter().find(|c| c.name == value).cloned();
                if let Some(c) = c {
                    if c.argument_hint.is_empty() && c.local {
                        self.run_slash(&c.name, "", now, out);
                    } else {
                        self.editor.set(&format!("/{} ", c.name));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ history search

    fn search_history(&self, query: &str, from: i64) -> (String, i64) {
        let h = &self.editor.history;
        let q = query.to_lowercase();
        let mut i = from.min(h.len() as i64 - 1);
        while i >= 0 {
            let entry = &h[i as usize];
            if q.is_empty() || entry.to_lowercase().contains(&q) {
                return (entry.clone(), i);
            }
            i -= 1;
        }
        (String::new(), -1)
    }

    fn search_key(&mut self, k: &Key) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        let mut again = true;
        match k {
            Key::Text(t) | Key::Paste(t) => s.query.push_str(t),
            Key::Named(Named::Backspace) => {
                s.query.pop();
            }
            Key::Named(Named::CtrlR) => {
                s.from = s.at - 1;
                again = false;
            }
            Key::Named(Named::Enter | Named::Right | Named::Tab) => {
                let m = s.matched.clone();
                if !m.is_empty() {
                    self.editor.set(&m);
                }
                self.search = None;
                return;
            }
            Key::Named(Named::Escape | Named::CtrlC) => {
                self.search = None;
                return;
            }
            _ => return,
        }
        let (query, mut from) = (s.query.clone(), s.from);
        if again {
            from = self.editor.history.len() as i64 - 1;
        }
        let (matched, at) = self.search_history(&query, from);
        if let Some(s) = self.search.as_mut() {
            s.from = from;
            s.matched = matched;
            s.at = at;
        }
    }

    // ------------------------------------------------------------------ suggestions

    fn refresh_suggestions(&mut self, now: i64) {
        if !self.dialogs.is_empty() || self.picker.is_some() || self.search.is_some() {
            self.sug = None;
            self.sug_view.clear();
            return;
        }
        let key = format!("{}|{}", self.editor.text(), self.editor.cur);
        if self.sug_dismissed.as_deref() == Some(&key) {
            self.sug = None;
            self.sug_view.clear();
            return;
        }
        let cwd = PathBuf::from(&self.cfg.cwd);
        let mut opts = complete::Options::new(&cwd);
        opts.commands = &self.commands;
        opts.limit = 50;
        let next = complete::suggest(&self.editor, &opts, &StdFs);
        let same = match (&next, &self.sug) {
            (Some(a), Some(b)) => a.token == b.token,
            _ => false,
        };
        if !same {
            self.sug_selected = 0;
        }
        self.sug_view = next
            .as_ref()
            .map(|s| {
                s.items
                    .iter()
                    .map(|i| Sug {
                        label: i.label.clone(),
                        detail: i.detail(&StdFs, now),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.sug = next.filter(|s| !s.items.is_empty());
        if self.sug.is_none() {
            self.sug_view.clear();
        }
    }

    /// Returns true when the key was consumed.
    fn suggestion_key(&mut self, k: &Key, now: i64, out: &mut Vec<Cmd>) -> bool {
        let Some(s) = self.sug.as_ref() else {
            return false;
        };
        let n = s.items.len();
        match k {
            Key::Named(Named::Up) => self.sug_selected = (self.sug_selected + n - 1) % n,
            Key::Named(Named::Down) => self.sug_selected = (self.sug_selected + 1) % n,
            Key::Named(Named::PageUp) => self.sug_selected = self.sug_selected.saturating_sub(8),
            Key::Named(Named::PageDown) => self.sug_selected = (self.sug_selected + 8).min(n - 1),
            Key::Named(n @ (Named::Tab | Named::Enter)) => {
                let item = &s.items[self.sug_selected.min(n_idx(s))];
                let exact = self.editor.chars[s.token.start..s.token.end]
                    .iter()
                    .collect::<String>()
                    == item.insert.trim_end();
                let detail = item.detail(&StdFs, now);
                let is_cmd = s.token.kind == complete::TokenKind::Command;
                let (token, item) = (s.token.clone(), item.clone());
                complete::apply(&mut self.editor, &token, &item);
                // enter on a complete command runs it right away
                if *n == Named::Enter && is_cmd && (exact || !detail.contains('<')) {
                    self.sug = None;
                    self.sug_view.clear();
                    self.submit(now, out);
                }
            }
            Key::Named(Named::Escape) => {
                self.sug_dismissed = Some(format!("{}|{}", self.editor.text(), self.editor.cur));
            }
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------------ dialogs

    fn dialog_key(&mut self, k: &Key, now: i64) {
        let Some(d) = self.dialogs.first_mut() else {
            return;
        };
        // any key keeps an expiring approval waiting: only the clock or an answer ends it
        match &d.kind {
            DialogKind::Question { .. } => self.question_key(k),
            _ => self.choice_key(k, now),
        }
    }

    fn choice_key(&mut self, k: &Key, now: i64) {
        let wait = self.armed_in(now).is_some();
        let d = &mut self.dialogs[0];
        let n = d.pending.options.as_ref().map_or(0, Vec::len);
        if n == 0 {
            return;
        }
        let pick = match k {
            Key::Named(Named::Up) => {
                d.pending.choice = (d.pending.choice + n - 1) % n;
                None
            }
            Key::Named(Named::Down) => {
                d.pending.choice = (d.pending.choice + 1) % n;
                None
            }
            // `x`: the whole (sanitised) input, scrollable; harmless, so allowed while arming
            Key::Text(t) if t == "x" => {
                d.pending.view_all = !d.pending.view_all;
                d.pending.input_scroll = 0;
                None
            }
            Key::Named(Named::PageUp | Named::PageDown) if d.pending.view_all => {
                let max = d
                    .pending
                    .input_rows
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .saturating_sub(INPUT_VIEW_ROWS);
                let step = INPUT_VIEW_ROWS / 2;
                d.pending.input_scroll = if matches!(k, Key::Named(Named::PageUp)) {
                    d.pending.input_scroll.saturating_sub(step)
                } else {
                    (d.pending.input_scroll + step).min(max)
                };
                None
            }
            Key::Text(_) | Key::Named(Named::Enter) if wait => None,
            Key::Text(t) if t.len() == 1 && t.as_bytes()[0].is_ascii_digit() && t != "0" => {
                let i = usize::from(t.as_bytes()[0] - b'1');
                (i < n).then_some(i)
            }
            Key::Named(Named::Enter) => Some(d.pending.choice),
            Key::Named(Named::Escape | Named::CtrlC) => Some(usize::MAX), // the last option is "no"
            _ => None,
        };
        let Some(i) = pick else { return };
        let key = if i == usize::MAX {
            if matches!(d.kind, DialogKind::Plan { .. }) {
                "keep"
            } else {
                "no"
            }
            .to_string()
        } else {
            d.pending
                .options
                .as_ref()
                .map(|o| o[i].key.clone())
                .unwrap_or_default()
        };
        self.answer(&key);
    }

    fn answer(&mut self, key: &str) {
        let mut d = self.dialogs.remove(0);
        match &mut d.kind {
            DialogKind::Approval {
                suggestions, reply, ..
            } => {
                let a = match key {
                    "yes" => Approval::allow(),
                    "always" => Approval {
                        accepted: suggestions.clone(),
                        ..Approval::allow()
                    },
                    _ => Approval::deny(DENY_MESSAGE),
                };
                if let Some(r) = reply.take() {
                    let _ = r.send(a);
                }
            }
            DialogKind::Plan { reply, .. } => {
                let (approved, mode) = match key {
                    "manual" => (true, Some(Mode::Default)),
                    "accept" => (true, Some(Mode::AcceptEdits)),
                    _ => (false, None),
                };
                if let Some(m) = mode {
                    self.mode = m;
                    self.mode_changed();
                }
                let message = (!approved).then(|| {
                    "The user doesn't want to proceed with this plan yet. Stay in plan mode, keep planning, and ask what to change.".to_string()
                });
                if let Some(r) = reply.take() {
                    let _ = r.send(Some(UserReply::Plan {
                        approved,
                        message,
                        mode,
                    }));
                }
            }
            DialogKind::Question {
                questions, reply, ..
            } => {
                let mut answers = serde_json::Map::new();
                for (i, q) in questions.iter().enumerate() {
                    let text = if key == "no" {
                        "The user declined to answer.".to_string()
                    } else {
                        let st = &d.pending.qstate[i];
                        st.picked
                            .iter()
                            .filter_map(|&k| {
                                if k == q.options.len() {
                                    Some(st.other.trim().to_string())
                                } else {
                                    q.options.get(k).map(|o| o.label.clone())
                                }
                            })
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    answers.insert(q.question.clone(), Value::String(text));
                }
                if let Some(r) = reply.take() {
                    let _ = r.send(Some(UserReply::Answers(Value::Object(answers))));
                }
            }
        }
    }

    /// Port of Node `questionKey`.
    fn question_key(&mut self, k: &Key) {
        let d = &mut self.dialogs[0];
        let p = &mut d.pending;
        let many = p.questions.len() > 1;
        let last = if many {
            p.questions.len()
        } else {
            p.questions.len() - 1
        };
        if matches!(k, Key::Named(Named::Escape | Named::CtrlC)) {
            self.answer("no");
            return;
        }
        if many && is_key(k, Named::Tab) {
            p.qi = last.min(p.qi + 1);
            return;
        }
        if many && is_key(k, Named::ShiftTab) {
            p.qi = p.qi.saturating_sub(1);
            return;
        }
        if many && p.qi >= p.questions.len() {
            match k {
                Key::Named(Named::Enter) => self.answer("yes"),
                Key::Named(Named::Left) => p.qi -= 1,
                _ => {}
            }
            return;
        }
        let q = p.questions[p.qi].clone();
        let st: &mut QState = &mut p.qstate[p.qi];
        let n = q.options.len() + 1; // + Other
        let is_other = st.choice == q.options.len();
        let mut finished = false;
        let advance = |qi: &mut usize, finished: &mut bool| {
            if many {
                *qi = last.min(*qi + 1);
            } else {
                *finished = true;
            }
        };
        if is_other && st.typing {
            match k {
                Key::Text(t) | Key::Paste(t) => st.other.push_str(&t.replace('\n', " ")),
                Key::Named(Named::Backspace) => {
                    st.other.pop();
                }
                Key::Named(Named::Enter) => {
                    st.typing = false;
                    if !st.other.trim().is_empty() {
                        if !q.multi_select {
                            st.picked = BTreeSet::from([st.choice]);
                        } else {
                            st.picked.insert(st.choice);
                        }
                        if !q.multi_select {
                            advance(&mut p.qi, &mut finished);
                        }
                    }
                }
                Key::Named(Named::Up | Named::Down) => st.typing = false,
                _ => {}
            }
            if st.typing || is_key(k, Named::Enter) {
                if finished {
                    self.answer("yes");
                }
                return;
            }
        }
        let toggle = |st: &mut QState| {
            if !st.picked.remove(&st.choice) {
                st.picked.insert(st.choice);
            }
        };
        match k {
            Key::Named(Named::Up) => st.choice = (st.choice + n - 1) % n,
            Key::Named(Named::Down) => st.choice = (st.choice + 1) % n,
            Key::Named(Named::Left) if many => p.qi = p.qi.saturating_sub(1),
            Key::Named(Named::Right) if many => p.qi = last.min(p.qi + 1),
            Key::Text(t)
                if t.len() == 1
                    && (b'1'..=b'9').contains(&t.as_bytes()[0])
                    && usize::from(t.as_bytes()[0] - b'0') <= n =>
            {
                st.choice = usize::from(t.as_bytes()[0] - b'1');
                if st.choice == q.options.len() {
                    st.typing = true;
                } else if q.multi_select {
                    toggle(st);
                } else {
                    st.picked = BTreeSet::from([st.choice]);
                    advance(&mut p.qi, &mut finished);
                }
            }
            Key::Text(t) if t == " " && q.multi_select => {
                if is_other {
                    st.typing = true;
                } else {
                    toggle(st);
                }
            }
            Key::Named(Named::Enter) => {
                if is_other && (st.other.is_empty() || !st.picked.contains(&st.choice)) {
                    st.typing = true;
                } else if q.multi_select {
                    advance(&mut p.qi, &mut finished);
                } else {
                    st.picked = BTreeSet::from([st.choice]);
                    advance(&mut p.qi, &mut finished);
                }
            }
            Key::Text(t) if is_other => {
                st.typing = true;
                st.other.push_str(t);
            }
            _ => {}
        }
        if finished {
            self.answer("yes");
        }
    }

    // ------------------------------------------------------------------ mouse + selection

    fn geom(&self) -> Option<sel::Geom> {
        let f = self.frame.as_ref()?;
        let r = f.rects.chat_rect?;
        let b = f.chat_body.as_ref()?;
        sel::body_geom(
            Some(sel::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
            }),
            Some(sel::Body {
                total: b.total as i64,
                h: b.h as i64,
            }),
            self.scroll as i64,
        )
    }

    fn in_chat(&self, p: Point) -> bool {
        self.frame
            .as_ref()
            .and_then(|f| f.rects.chat_rect)
            .is_some_and(|r| p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h)
    }

    pub fn clear_selection(&mut self) {
        self.sel = SelState::default();
    }

    fn mouse(&mut self, k: &Key, now: i64, out: &mut Vec<Cmd>) {
        match *k {
            Key::Wheel { dir, x, y, mods } => {
                let ev = SelEvent::Wheel {
                    shift: mods.shift,
                    alt: mods.alt,
                };
                let (mode, act) = selection_transition(self.sel.mode(), ev);
                if act == Act::Clear {
                    self.clear_selection();
                } else {
                    self.sel.mode = Some(mode);
                }
                let p = Point {
                    x: i64::from(x) - 1,
                    y: i64::from(y) - 1,
                };
                if self.in_chat(p) && matches!(dir, WheelDir::Up | WheelDir::Down) {
                    let max = self.frame.as_ref().map_or(0, |f| f.rects.max_scroll);
                    self.scroll = if dir == WheelDir::Up {
                        (self.scroll + WHEEL_LINES).min(max)
                    } else {
                        self.scroll.saturating_sub(WHEEL_LINES)
                    };
                    self.follow_pointer();
                }
            }
            Key::MouseDrag { x, y, .. } => {
                if self.sel.mode() != SelMode::Dragging || self.sel.selection.is_none() {
                    return;
                }
                let p = Point {
                    x: i64::from(x) - 1,
                    y: i64::from(y) - 1,
                };
                self.sel.pointer = Some(p);
                if let Some(g) = self.geom() {
                    let c = sel::point_to_content(&g, p);
                    if let Some(pr) = self.sel.press
                        && pr != c
                    {
                        if let Some(s) = self.sel.selection.as_mut() {
                            s.a = pr;
                        }
                        self.sel.press = None;
                        self.sel.extending = false;
                    }
                    if let Some(s) = self.sel.selection.as_mut() {
                        s.b = c;
                    }
                }
            }
            Key::Mouse {
                x,
                y,
                button: 0,
                action: MouseAction::Up,
                mods,
            } if self.sel.mode() == SelMode::Dragging => {
                let _ = (x, y);
                let extending = std::mem::take(&mut self.sel.extending);
                self.sel.press = None;
                let Some(s) = self.sel.selection else {
                    self.sel.mode = Some(SelMode::Idle);
                    return;
                };
                let moved = s.a != s.b;
                let (mode, act) = selection_transition(
                    self.sel.mode(),
                    SelEvent::Up {
                        moved,
                        shift: mods.shift,
                        alt: mods.alt,
                        extending,
                    },
                );
                self.sel.mode = Some(mode);
                match act {
                    Act::Copy => self.copy_selection(now, out),
                    Act::Pend => self.toast(
                        "selection started · hold Shift and scroll, then click where it should end · esc cancels",
                        now,
                        false,
                    ),
                    _ => self.clear_selection(),
                }
            }
            Key::Mouse {
                x,
                y,
                button: 0,
                action: MouseAction::Down,
                mods,
            } => self.mouse_down(x, y, mods, now, out),
            _ => {}
        }
    }

    fn follow_pointer(&mut self) {
        if self.sel.mode() == SelMode::Dragging
            && let (Some(g), Some(p)) = (self.geom(), self.sel.pointer)
            && let Some(s) = self.sel.selection.as_mut()
        {
            s.b = sel::point_to_content(&g, p);
        }
    }

    fn mouse_down(
        &mut self,
        x: u32,
        y: u32,
        mods: nemo_tui_base::keys::Mods,
        now: i64,
        out: &mut Vec<Cmd>,
    ) {
        let p = Point {
            x: i64::from(x) - 1,
            y: i64::from(y) - 1,
        };
        let row = self
            .lines
            .get(p.y.max(0) as usize)
            .cloned()
            .unwrap_or_default();
        // ctrl+click opens a link or a path under the pointer (looked up in the drawn frame)
        if mods.ctrl && self.in_chat(p) {
            if let Some(u) = url_at(&row, p.x as usize) {
                out.push(Cmd::Open(OpenTarget::Url(u)));
                self.clear_selection();
                return;
            }
            if let Some(t) = path_at(&row, p.x as usize)
                .and_then(|raw| existing_path(&raw, self.cfg.home.as_deref()))
            {
                out.push(Cmd::Open(OpenTarget::from_path(&t)));
                self.clear_selection();
                return;
            }
        }
        let g = if self.in_chat(p) { self.geom() } else { None };
        let b = g.as_ref().map(sel::body_bounds);
        let in_body = b.is_some_and(|b| p.y >= b.y0 && p.y <= b.y1);
        let on_text = in_body
            && b.is_some_and(|b| {
                let g = g.as_ref().unwrap();
                !(p.x < b.x0 || p.x >= b.x1 || p.y - g.top + g.row_at0 < 0)
                    && sel::cell_has_text(&row, p.x)
            });
        let (mode, act) = selection_transition(
            self.sel.mode(),
            SelEvent::Down {
                text: on_text,
                body: in_body,
                shift: mods.shift,
                alt: mods.alt,
            },
        );
        if act == Act::Extend
            && self.sel.selection.is_some()
            && let Some(g) = g
        {
            self.sel.pointer = Some(p);
            self.sel.press = (self.sel.mode() == SelMode::Selected && !(mods.shift || mods.alt))
                .then(|| sel::point_to_content(&g, p));
            self.sel.extending = true;
            if let Some(s) = self.sel.selection.as_mut() {
                s.b = sel::point_to_content(&g, p);
            }
            self.sel.mode = Some(mode);
            return;
        }
        self.clear_selection();
        if let (Some(g), true) = (g, on_text) {
            self.sel.pointer = Some(p);
            let c = sel::point_to_content(&g, p);
            self.sel.selection = Some(Selection { a: c, b: c });
            self.sel.mode = Some(SelMode::Dragging);
        }
        let _ = now;
    }

    fn copy_selection(&mut self, now: i64, out: &mut Vec<Cmd>) {
        let (Some(s), Some(g)) = (self.sel.selection, self.geom()) else {
            self.clear_selection();
            return;
        };
        let text = if s.a != s.b {
            self.frame
                .as_ref()
                .and_then(|f| f.chat_body.as_ref())
                .map_or(String::new(), |body| {
                    sel::text_of_content(&s, g.x0, g.x1, |from, to| {
                        body_text_rows(body, from.max(0) as usize, to.max(0) as usize)
                    })
                })
        } else {
            String::new()
        };
        if text.is_empty() {
            self.clear_selection();
        } else {
            out.push(Cmd::Copy(text));
        }
        let _ = now;
    }

    // ------------------------------------------------------------------ rendering

    fn session_rows(&self, now: i64) -> Vec<SessionRow> {
        let phase = session_phase(&self.activity, now);
        let busy = self.chat.agent.status != "idle";
        let phase_s = if busy {
            match phase.phase {
                model::Phase::Idle => "thinking",
                model::Phase::Tool => "tool",
                model::Phase::Agents => "agents",
                model::Phase::Thinking => "thinking",
            }
        } else {
            "idle"
        };
        let mut rows = vec![SessionRow {
            id: self.cfg.session_id.clone(),
            project: clean_line(&basename(&self.cfg.cwd)),
            cwd: clean_line(&self.cfg.cwd),
            mtime: now,
            live: Some(Live {
                name: "nemo".into(),
                cwd: clean_line(&self.cfg.cwd),
                status: if !self.dialogs.is_empty() {
                    "waiting"
                } else if busy {
                    "running"
                } else {
                    "idle"
                }
                .into(),
            }),
            phase: phase_s.into(),
            tokens: self.activity.tokens.total() as f64,
            ctx_pct: None,
            waiting: !self.dialogs.is_empty(),
            mode: Some(mode_str(self.mode).into()),
        }];
        for s in &self.cfg.saved {
            if s.id == self.cfg.session_id {
                continue;
            }
            rows.push(SessionRow {
                id: s.id.clone(),
                project: clean_line(&basename(&s.cwd)),
                cwd: clean_line(&s.cwd),
                mtime: s.mtime,
                live: None,
                phase: "idle".into(),
                tokens: 0.0,
                ctx_pct: None,
                waiting: false,
                mode: None,
            });
        }
        rows
    }

    /// One frame of `cols` x `rows` painted lines (the last one is the status line).
    pub fn render(&mut self, cols: u32, rows: u32, now: i64) -> Vec<String> {
        self.size = (cols, rows);
        let (w, h) = (cols as usize, rows as usize);
        if (cols as i64) < MIN_COLS || (rows as i64) < MIN_ROWS + 1 {
            return too_small(w, h);
        }
        let frame_no = (now.max(0) / 80) as usize;
        // dialogs -> pending list, with the live countdown
        self.chat.agent.pending = self
            .dialogs
            .iter()
            .map(|d| {
                let mut p = d.pending.clone();
                if let Some(dl) = d.deadline_ms {
                    let left = ((dl - now) as f64 / 1000.0).ceil().max(0.0) as i64;
                    p.prompt = format!(
                        "{} · expires unanswered in {}:{:02}",
                        p.prompt,
                        left / 60,
                        left % 60
                    )
                    .trim_start_matches(" · ")
                    .to_string();
                }
                p
            })
            .collect();
        if self.armed_in(now).is_some()
            && let Some(p) = self.chat.agent.pending.first_mut()
        {
            p.prompt = format!("{} · wait…", p.prompt)
                .trim_start_matches(" · ")
                .to_string();
        }
        self.chat.agent.mode = mode_str(self.mode).into();
        self.chat.agent.model = clean_line(&self.model);
        let rows_s = self.session_rows(now);
        let key = workspace_key(&self.cfg.cwd);
        let ws = [Workspace {
            key: key.clone(),
            name: clean_line(&basename(&self.cfg.cwd)),
            cwd: clean_line(&self.cfg.cwd),
            count: rows_s.len() as u32,
            waiting: !self.dialogs.is_empty(),
            busy: self.chat.agent.status != "idle",
        }];
        let mut opts = AgentOpts {
            scroll: self.scroll,
            expanded: self.expanded,
            selected: self.sug_selected,
            suggestions: (!self.sug_view.is_empty()).then(|| RSug {
                items: self.sug_view.clone(),
            }),
            ..AgentOpts::default()
        };
        if let Some(ps) = &self.picker {
            opts.picker = Some(ps.picker.clone());
        }
        if let Some(s) = &self.search {
            opts.search = Some(Search {
                query: s.query.clone(),
                matched: (!s.matched.is_empty()).then(|| s.matched.clone()),
            });
        }
        let view = DashView {
            opts,
            ..DashView::default()
        };
        let usage = Usage::default();
        let sid = self.cfg.session_id.clone();
        let frame = {
            let state = DashState {
                session: &self.activity,
                sessions: &rows_s,
                index: 0,
                usage: &usage,
                follow: false,
                history: &[],
                usage_metric: "tokens",
                mcp_servers: &[],
                mcp_live: None,
                workspaces: Some(&ws),
                active_workspace: &key,
                agent: Some(&self.chat.agent),
                chat_agent: None,
                editor: Some(EditorView {
                    chars: &self.editor.chars,
                    cur: self.editor.cur,
                }),
                focus: Focus::Agent,
                view: &view,
                agent_session_id: Some(&sid),
                tz_offset_min: self.cfg.tz_offset_min,
            };
            render_frame(&state, w, h - 1, frame_no, now)
        };
        let mut lines = frame.lines.clone();
        lines.push(status_line(
            &self.cfg.sandbox,
            &self.cfg.budget,
            &self.chat.totals,
            self.chat.verification.as_ref(),
            self.ran_task,
            self.chat
                .task_started
                .map_or(0, |t| (now - t).max(0) as u64),
            w,
        ));
        self.frame = Some(frame);
        self.lines.clone_from(&lines);
        // selection highlight is painted on a copy: hit tests keep reading the plain lines
        if let (Some(s), Some(g)) = (self.sel.selection, self.geom()) {
            let b = sel::body_bounds(&g);
            lines =
                sel::apply_selection(&lines, Some(&sel::to_screen(&s, &g)), Some(&b)).into_owned();
        }
        if let Some(t) = &self.toast {
            let color = if t.bad { T_ERROR } else { T_SUCCESS };
            let last = lines.len() - 1;
            lines[last] = format!(
                "{}\x1b[7m{}{}",
                theme::fg(color),
                theme::fit(
                    &format!(" {} {} ", if t.bad { "✗" } else { "✓" }, t.text),
                    w
                ),
                theme::RESET
            );
        }
        lines
    }
}

fn n_idx(s: &complete::Suggestions) -> usize {
    s.items.len() - 1
}

/// Suggestion in the shape the pane's `always_label` reads.
fn suggestion_view(s: &Suggestion) -> Value {
    match s {
        Suggestion::AddRules { rules, behavior } => json!({
            "type": "addRules",
            "behavior": format!("{behavior:?}").to_lowercase(),
            "rules": rules.iter().map(|r| json!({
                "toolName": clean_line(&r.tool_name),
                "ruleContent": r.rule_content.as_deref().map(clean_line).unwrap_or_default(),
            })).collect::<Vec<_>>(),
        }),
        Suggestion::SetMode(m) => json!({"type": "setMode", "mode": m.as_str()}),
    }
}

fn list_dirs(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort_by_key(|n| n.to_lowercase());
    v.truncate(500);
    v
}

pub(crate) fn too_small(w: usize, h: usize) -> Vec<String> {
    let text = format!("terminal too small · need {}x{}", MIN_COLS, MIN_ROWS + 1);
    let mut lines = vec![" ".repeat(w); h];
    if h > 0 {
        let pad = w.saturating_sub(text.chars().count()) / 2;
        lines[h / 2] = theme::fit(&format!("{}{text}", " ".repeat(pad)), w);
    }
    lines
}
