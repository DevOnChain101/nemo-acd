# Spec: TUI redesign (brand theme, panel registry, /config) — draft 0.1

Date 2026-10-29. Status: **draft, waiting for an owner decision** (see §11). Tags: `[fact file:line]` = read in the repo, `[assumption]` = not verified, `[derived]` = a value this spec adds that is not in the kit.

**Inputs:** `design/02-design-tokens/tokens.css`, the desktop and landing previews, and `design/01-brand/logo/png/synthesis-symbol-transparent-256.png`. The brand boards (`04`) were never copied into the repo (`design/README.md` "Not included"), so they were not used.

**Current UI:** read from `crates/synthesis-tui-render/src/{dash,theme,agent}.rs` and `crates/synthesis-tui-base/src/sgr.rs`. Real frames were taken from the goldens `frame-agent-chat-{80x24,120x40,200x50}` and `frame-agent-permission-120x40`.

**Not run:** `synth --once` and `tests/tty-qa` captures (the design session had no shell). Frames at 100x30 and 140x40 were not seen. Every mockup here is hand-drawn. The contact sheet from §8.4 becomes the reference once the code exists.

## 0. Problem and non-negotiables

**Problem.** The TUI does not look like the Synthesis brand. It has heavy `┏━┓` boxes, an indigo/cyan gradient "MISSION CONTROL" pill, eight saturated hues and shimmer animations (`theme.rs:10-65`, `dash.rs:301-345`, `dash.rs:2398-2423`). The layout cannot be configured.

**Current structure** `[fact dash.rs:1462-3307]`:
- **Left column** (`side_column`): Workspace, Running, Session, MCP, Tools & Skills, Activity.
- **Centre** (`agent_detail`): a row with Status, Tokens and Tasks, then Usage, then the chat.
- **Right column** (`topology_column`): Topology, a sub-agent dock that appears only while a sub-agent is being viewed, then Tasks and Usage.
- **Layout choice:** `layout_for` picks one of three layouts (Right, Bottom, None).
- **Bottom:** two rows, the key-hint footer and the status line from `app.rs`.
- **Colour:** about 250 call sites read RGB constants. All output is 24-bit, and `sgr::encode` downgrades it for smaller palettes.
- **The app always passes an agent** (`app.rs` `render`). The dashboard layout without an agent is only exercised by the `dash-*` goldens.

**Non-negotiables (owner rules):**
- Workspace, Running and Chat are never hidden.
- Every other panel can be hidden from `/config`.
- When all optional left panels are hidden, Workspace and Running split the left column 50/50.
- The sub-agent transcript always lives in the right sidebar.
- Panels reflow with no holes.
- Below 100 columns the sidebars become a switcher. Below 60 columns the screen is chat only.
- Settings persist in the USER file only.
- The design works on macOS, Linux and Windows, and degrades through truecolor, 256, 16 and `NO_COLOR`.
- Credentials and other products' environment variables are untouched (ADR-011).

## 1. Options and choice

| Axis | Options | Pick |
|---|---|---|
| **Theme mechanism** | **A1** Pass `&Theme` through every render function. Explicit, but about 250 call sites and many signatures change.<br>**A2** Semantic token functions (`t().body`) read a theme that `render_frame`, `render_agent` and `transcript_rows` set for the duration of one call, through a thread-local scope guard.<br>**A3** Renderers keep fixed "sentinel" RGB values and `sgr::encode` swaps them for theme colours. Smallest diff, but colours mixed by `lerp` match no table entry and the mapping becomes magic. | **A2** |
| **Layout** | **B1** Add visibility flags to the three existing layouts. The sizing logic is spread across `side_column`, `topology_column` and `agent_detail` with ad-hoc constants, and the owner's rules do not fit it.<br>**B2** A pure planner (`layout.rs`) plus a static panel registry. Each panel renders by id and reuses the existing panel functions. | **B2** |
| **Panel chrome** | **C1** Rounded light boxes. Each costs 2 rows and 2 columns.<br>**C2** Borderless sections: a label row, one blank row between sections, and one `│` rule between columns. Frames are kept for floating elements only (prompt, dialogs, pickers, `/config`). | **C2**. It is closer to the kit (1 px lines, generous space) and saves about 2 rows per panel. |
| **Logo** | **L1** Braille line art generated offline from the PNG, with per-cell truecolor sheen.<br>**L2** Half-block art (`▀▄`). Chunkier.<br>**L3** Inline image (iTerm2, kitty, sixel). Exact, but only some terminals support it, tmux breaks it, and the diff renderer can erase it. | **L1** as the default. L3 is optional, off by default, and limited to the welcome screen. |

**Strongest reason against this choice.** It moves Status, Tokens and Tasks out of the centre column, which is a visible change for current users. It also rebaselines nearly every render golden while another developer is editing `terminal.rs`, `e2e.rs` and `app.rs`. The `app.rs` line numbers moved while this spec was being written. The phase gates in §8.4 contain this risk.

**Reason against A2 specifically.** It adds hidden per-thread state to a crate that is documented as pure. Each entry point sets the scope explicitly, tests set it explicitly, and the default is Graphite.

## 2. Design principles

1. **Graphite, silver, one pearl.** Grey carries the hierarchy. The warm pearl marks focus and the logo. Status colours appear only for status.
2. **Space instead of lines.** One gap row between panels, a 2-column chat gutter, light rules, never heavy borders.
3. **One focus.** The focused element gets a pearl `▍` bar and an accent label.
4. **Never colour alone.** Every state has a glyph or a word, so the UI still reads in Mono and in 16 colours.
5. **Quiet motion.** One braille spinner per active item. The shimmer, gradient borders and pulses go (`pulse`, `cycle`, the shimmer in `bar`, `BORDER_GRAD`, `LOGO_GRAD`). With the Motion setting off, spinners become static glyphs.
6. **Respect the terminal.** The global background is never painted `[assumption: users who choose Graphite run a dark terminal]`. Only the header band, overlays, selection and diff lines are painted.

## 3. Palette and token mapping

Contrast is the WCAG ratio against `#0B0C0F` (the kit background). Each 256-colour index is what the existing `to256` produces automatically, so no override table is needed. In 16-colour mode a dedicated ANSI theme is used, and its RGB values match the ANSI16 table exactly, so `to16` maps them one to one.

| Token | Graphite (truecolor) | Source | Contrast | 256 | 16-colour | Mono |
|---|---|---|---|---|---|---|
| `bg_ref` (not painted) | `#0B0C0F` | kit bg | — | 232 | default | default |
| `surface` (header band, toasts) | `#15171B` | kit | — | 234 | none | none |
| `elevated` (dialogs, selection) | `#242830` | kit | — | 235 | none | reverse (selection) |
| `border` (rules, frames) | `#3A3F4A` | kit | 1.84 (decorative) | 238 | 8 | dim |
| `label` (uppercase section labels) | `#8C929B` | desktop.html `.micro` | 6.20 | 246 | 7 | dim |
| `muted` (times, hints, folders) | `#7A808C` | [derived] | 4.90 (4.53 on surface) | 244 | 8 | dim |
| `body` (chat text, rows) | `#C8CCD2` | [derived] | 12.05 | 252 | 7 | default |
| `strong` (values, user turn) | `#E5E7EB` | kit text-strong | 15.69 | 254 | 15 | bold |
| `accent` (active label, wordmark) | `#D9D9E0` | kit accent | 13.82 | 253 | 15 | bold |
| `accent_light` (logo highlight) | `#F7F8FA` | kit | 18.29 | 231 | 15 | bold |
| `pearl` (focus bar, caret, spinner) | `#D7CEC7` | kit soft-beige | 12.53 | 252 | 15 | bold |
| `ok` | `#A8C5B2` | desktop.html `.tick` | 10.45 | 151 | 2 | glyph ✓ |
| `warn` | `#D8B46A` | [derived] | 9.84 | 179 | 3 | glyph ! |
| `err` | `#E08A8A` | [derived] | 7.56 | 174 | 1 | glyph ✗ + bold |
| `info` (MCP, permission, links) | `#8EA4C8` | [derived] | 7.67 | 110 | 6 | underline (links) |
| `diff_add_bg` | `#14241B` | [derived] | body on it 10.03 | 234 | none | `+` glyph |
| `diff_del_bg` | `#2A1618` | [derived] | body on it 10.62 | 234 | none | `−` glyph |

Text on painted surfaces: `strong` on `elevated` is 11.93, `body` on `surface` is 11.1 (computed), and the kit's own `text` `#9CA3AF` on `elevated` is 5.82.

**Theme variants.** `/config` offers the first four. `ansi16` is chosen automatically.
- **Graphite** (default): the table above.
- **Midnight:** the header band and overlays use kit muted-blue `#0B1322` and elevated `#16213A`. Border `#2C3A55`, body `#C3CBD9`, muted `#7E8AA3` (5.60), accent unchanged, pearl becomes ice `#BFD3F2`. In 256-colour mode the blue collapses to grey 233, so Midnight looks almost like Graphite there.
- **High contrast:** body `#F2F3F5`, muted `#B8BDC6`, border `#8C929B` (6.2, so it meets the 3:1 rule for UI components), ok `#9BE7B0`, warn `#FFD479`, err `#FF9C9C`. `strong` is bold and selection is bold on `elevated`.
- **Mono:** no colour at all. Strong is bold, muted and label are dim (SGR 2), selection is reverse, and focus is `▍` plus bold. `sgr` Mono mode already keeps attributes `[fact sgr.rs:17]`.
- **ansi16** (automatic): the slots in the 16-colour column above.

**Light terminals.** Graphite on a white background falls to 1.6–2.5:1, which is unreadable. There is no detection in P1. The user has two options:
- **Mono:** works on any background.
- **Pearl (light):** proposed in decision D5. Text `#3A3F4A` (9.9 on `#F7F8FA`), strong `#15171B` (16.9), muted `#5B616E` (5.85).

Automatic detection would need an OSC 11 query at startup in `terminal.rs`. That is a P4 option and needs a decision.

**Selection rule:**

```
mode  = ColorMode::detect(env)                 // existing, sgr.rs:28
theme = Mono            if mode == Mono        // NO_COLOR always wins; /config shows "NO_COLOR is set"
        ansi16          if mode == Ansi16
        user.theme      otherwise              // default graphite
glyphs = user.glyphs, else ascii when TERM in {linux, vt100, vt220, cons25}, else unicode
```

**Contrast test.** A unit test in `theme.rs` computes the WCAG ratio of every theme against its reference background and asserts these floors:

| Pair | Floor |
|---|---|
| body, strong | ≥ 7 |
| label, muted, ok, warn, err, info | ≥ 4.5 |
| body on diff backgrounds | ≥ 7 |
| border | ≥ 1.5 (≥ 3 for High contrast) |

**Mapping from today's constants:**

| Today | Becomes |
|---|---|
| `T_TEXT` | strong |
| `T_BODY` | body |
| `T_INACTIVE` | label |
| `T_SUBTLE` | muted |
| `T_BORDER` | border |
| `T_CLAUDE`, `ACCENT_CORE` | accent |
| `ACCENT_LIGHT`, `ACCENT_GLOW` | accent_light |
| `T_SUCCESS` | ok |
| `T_WARNING` | warn |
| `T_ERROR` | err |
| `T_PERMISSION`, `T_MCP` | info |
| mode colours (`T_AUTO_ACCEPT`, `T_PLAN_MODE`, `T_BASH`) | label; the mode is named in text |
| `AGENT_COLORS` (8 hues) | 4 tints: ok, info, sand `#CDBB9A` [derived], pearl |
| `HEAT` (green ramp) | silver ramp `#1B1E24 #3A3F4A #6B717C #9CA3AF #E5E7EB` |
| `BORDER_GRAD`, `LOGO_GRAD` | removed |

**Unrelated finding.** `sgr.rs:30` reads `CLAUDE_MONITOR_COLOR`. That breaks the environment-variable rule (only `SYNTHESIS_*` plus standard variables may be read). It should be renamed to `SYNTHESIS_COLOR` or removed; see follow-up F1.

## 4. Typography, spacing, glyphs

**Wordmark.**
- 80 columns and wider: `S Y N T H E S I S` in `accent_light`.
- 60–79 columns: `SYNTHESIS`.
- Below 60: the mark only.
- Welcome screen: the art, the spaced wordmark, and "Autonomous development environment" in `label`.

**Text.**
- Section labels are UPPERCASE in `label`, without letter spacing. Counts sit right-aligned in `strong`.
- Graphite and Midnight use no bold, following the Node rule `BOLD_ON = false`. Only High contrast and Mono use attributes.
- Tokens become `Style { fg, bg, bold, dim, reverse, italic }`, and `paint_style` replaces `paint` where attributes matter.

**Spacing.**
- 1 column of padding inside sidebar sections.
- 2-column chat gutter.
- Inside frames: 1 column of padding, plus 1 blank row above and below dialog content.
- 1 blank row between chat turns.

**Lines.**
- Frames: rounded light `╭─╮│╰╯` in `border`.
- Column rules: `│`.
- Separators inside a panel: `┄`.
- No `┏━┓` and no `━` rule anywhere.

**Key hints.** The key is in `body` and the action in `muted`, three spaces apart. Hints that do not fit are dropped whole by priority, using the existing `fit_items` (`theme.rs:208`).

| Use | Unicode | ASCII |
|---|---|---|
| mark | `⢎⡱` (braille ring) | none |
| spinner | `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | `\| / - \` |
| live / on | `●` | `*` |
| off / idle | `○` | `o` |
| needs you (approval, question) | `◆` | `!` |
| success / failure | `✓` / `✗` | `+` / `x` |
| tool row | `◇` | `>` |
| user prompt | `›` | `>` |
| thinking | `∴` | `:` |
| focus / selection | `▍` | `>` |
| locked | `▣` | `#` |
| meter / bar | `▰▱` / `▇` | `#-` / `#` |
| diff | `+` `−` | `+` `-` |
| branch | `⎇` | `git:` |
| separators | `·` `│` `┄` | `-` `\|` `-` |

**Wide-character rule.** Every glyph must have `unicode-width` 1. Emoji-presentation code points (U+1F300 and above, or anything with VS16) are banned, and a unit test enumerates the set. Ambiguous-width glyphs such as `●○◆▣·▍` are already used today, but they take two cells in terminals set to treat ambiguous width as wide (common with CJK locales). The ASCII set (`/config` → Glyphs) is the fallback for those terminals.

**Logo.**
- The header mark is `⢎⡱`: the left cell in `accent`, the right in `pearl`.
- The welcome art is braille, about 18×6 cells, with per-cell colour running from accent through accent_light to pearl to imitate the sheen.
- The art is generated offline by `tools/logo-art/gen.py` (Python standard library only, `zlib` for PNG) from the 256 px PNG, and committed as a `const` table in `logo.rs`. Nothing is decoded at runtime.
- In 16-colour and Mono the same dots are drawn in one colour. With the ASCII glyph set, only the wordmark is shown.
- Inline image (optional, L3): only when `TERM_PROGRAM` is `iTerm.app` or `WezTerm` (iTerm2 protocol), or `TERM` is `xterm-kitty` or `xterm-ghostty` (kitty protocol). Never inside tmux or screen (`TERM` starting with `screen` or `tmux`). Sixel is left out because it needs a DA1 round-trip. A 64 px PNG is embedded with `include_bytes!`.

## 5. Layout

### 5.1 Regions

| Region | Rule |
|---|---|
| Header | 1 row, or 2 when H ≥ 30 (adds a thin `─` rule). Left: mark, wordmark, project, branch. Right chips, in drop order: cost, waiting count, live/busy sessions, clock. |
| Tab strip | Tabs mode only. 1 row: `Chat · Sessions n · Agents n · Panels n`. |
| Bottom bar | 1 row. Merges the current footer and the status line: key hints on the left, sandbox · budget · verification on the right. Toasts replace it, as today. |
| Columns mode (W ≥ 100) | Left sidebar, chat, right sidebar (§6). |
| Tabs mode (60 ≤ W < 100) | One view at a time. **Sessions** = Workspace + Running (always split 50/50). **Agents** = the sub-agent panel at full height. **Panels** = the visible optional panels (left ones, then right ones) stacked with the same fit rules; the tab disappears when none are visible. **Chat** is the default view, and a pending approval switches back to it. **Tab** cycles the views (it reuses today's focus cycle); a mouse click on a tab selects it. |
| Chat only (40 ≤ W < 60) | No tab strip. `/config` still works and notes that panels appear at 60 columns and wider. The new minimum size is 40×12, down from 60×16 (`dash.rs:2730`). |

### 5.2 Mockups

80×24 (Tabs mode, Chat view):

```
 ⢎⡱  S Y N T H E S I S   proj ⎇ main                         ● 2 live  │  12:00
  ▍Chat    Sessions 2    Agents 1    Panels 3                      tab switches
  › fix the flaky login test
  ∴ thought for 2s · ctrl+o to show
  I will check login.ts and retry():
    • first
    • second
  ◇ Bash   npm test                                            ✗ exit 1 · 1.2s
    │ FAIL login.test.ts
    │ expected 2
  ◇ Read   src/login.ts                                                running

  ⠹ Conjuring…  4s · ↓ 0 tokens · esc to interrupt






  ╭──────────────────────────────────────────────────────────────────────────╮
  │ › hello▏                                                                 │
  ╰──────────────────────────────────────────────────────────────────────────╯
  manual · ⇧tab to cycle                     sonnet-4.5 · effort auto · ctx 3%
 enter send   esc interrupt   ⇧tab mode   /config   ctrl+c ×2 quit
```

120×36 (Columns mode: L=22, chat 68, R=30; default panels):

```
 ⢎⡱  S Y N T H E S I S    proj  ⎇ main                                                      ● 2 live · 1 busy  │  12:00
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 WORKSPACE         1 │  › fix the flaky login test                                        │ STATUS
 ▍proj             2 │                                                                    │ ⠹ running tool        9m54s
                     │  ∴ thought for 2s · ctrl+o to show                                 │   explorer · find files
 RUNNING           2 │                                                                    │
 ▍⠹ proj-0       10% │  I will check login.ts and retry():                                │ CONTEXT                  3%
    project-0        │    • first                                                         │ ▱▱▱▱▱▱▱▱▱▱▱▱  5.9k / 200k
  ● proj-1       10% │    • second                                                        │ in 9.0k · out 1.5k · $ –
    project-1        │                                                                    │
                     │  ◇ Bash   npm test                                ✗ exit 1 · 1.2s  │ TASKS                   1/3
 SESSION             │    │ FAIL login.test.ts                                            │ ✓ Read
 model  sonnet-4.5   │    │ expected 2                                                    │ ⠹ Fixing
 mode   manual       │  ◇ Read   src/login.ts                                    running  │ ○ Test
 effort auto         │  ◇ Edit   src/login.ts                                      +2 −1  │
 branch main         │     12     const r = await retry(fn, {                             │ SUB-AGENTS         1 active
                     │     13 −   tries: 2,                                               │ ▍⠹ explorer  find files  9m
 MCP               1 │     13 +   tries: 3,                                               │ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄
 ● github   12 tools │     14 +   backoff: 50,                                            │ › find files matching auth
                     │     15     })                                                      │ ◇ Grep  "login"     14 hits
                     │                                                                    │ ◇ Read  src/login.ts
                     │  ⠹ Conjuring…  4s · ↓ 0 tokens · esc to interrupt                  │ Two candidates: login.ts,
                     │                                                                    │ retry.ts. Checking retry…
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │                                                                    │
                     │  ╭──────────────────────────────────────────────────────────────╮  │
                     │  │ › hello▏                                                     │  │
                     │  ╰──────────────────────────────────────────────────────────────╯  │
                     │  manual · ⇧tab to cycle         sonnet-4.5 · effort auto · ctx 3%  │
 enter send   esc interrupt   ⇧tab mode   /config panels   ctrl+c ×2 quit              budget 3/50 turns  ·  unverified
```

200×50 (L=32, chat 120, R=48). All left panels and Status, Context, Tasks, Activity and Usage are on. Rows 25–30 are cut.

```
 ⢎⡱  S Y N T H E S I S    proj  ⎇ main                                                                                                            ● 2 live · 1 busy  │  ! 1 waiting  │  $0.42  │  12:00
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 WORKSPACE                   1 │  › fix the flaky login test                                                                                            │ STATUS
 ▍proj                       4 │                                                                                                                        │ ⠹ running tool                          9m54s
                               │  ∴ thought for 2s · ctrl+o to show                                                                                     │   explorer · find files
 RUNNING                     4 │                                                                                                                        │ 5h quota  ▰▰▰▱▱▱▱▱▱▱▱▱  23%  ↻ 2h10m
 ▍⠹ proj-0                 10% │  I will check login.ts and retry():                                                                                    │
    project-0 · main           │    • first                                                                                                             │ CONTEXT                                    3%
  ● proj-1                 10% │    • second                                                                                                            │ ▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱▱  5.9k / 200k
    project-1 · parser         │                                                                                                                        │ in 9.0k · out 1.5k · cache 20.0k · $0.42
  ● proj-2                 48% │  ◇ Bash   npm test                                                                                    ✗ exit 1 · 1.2s  │
    project-2 · main           │    │ FAIL login.test.ts                                                                                                │ TASKS                                     1/3
  ○ proj-3                     │    │ expected 2                                                                                                        │ ✓ Read
    project-3 · main           │  ◇ Read   src/login.ts                                                                                     ✓ 42 lines  │ ⠹ Fixing
                               │                                                                                                                        │ ○ Test
 SESSION                       │  ◆ explorer   find files matching auth                                                             running in sidebar  │
 model   sonnet-4.5            │                                                                                                                        │ SUB-AGENTS                           1 active
 mode    manual                │                                                                                                                        │ ▍⠹ explorer   find files                   9m
 effort  auto                  │                                                                                                                        │ ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄
 branch  main                  │                                                                                                                        │ › find files matching auth
 uptime  1m00s                 │                                                                                                                        │ ◇ Grep  "login"                       14 hits
 turns   1 prompts · 5 tools   │                                                                                                                        │ ◇ Read  src/login.ts
                               │                                                                                                                        │ Two candidates: login.ts, retry.ts.
 MCP                         2 │                                                                                                                        │ Checking retry…
 ● github             12 tools │                                                                                                                        │
 ⠹ linear           connecting │                                                                                                                        │
┆ body rows 25–30 cut: TOOLS & SKILLS starts on row 26 (left); the sub-agent transcript continues to row 29 (right)
 TodoWrite  ▇▇▇▇▇▇▇▇▇▇  1      │  ◇ Edit   src/login.ts                                                                                          +2 −1  │ ACTIVITY
 skills  scout ×1              │     12     const r = await retry(fn, {                                                                                 │ 11:50:00  › refactor the parser
                               │     13 −   tries: 2,                                                                                                   │ 11:50:01  ◇ Read src/parser.rs
                               │     13 +   tries: 3,                                                                                                   │ 11:50:04  ◇ Bash npm test
                               │     14 +   backoff: 50,                                                                                                │ 11:50:06  ✗ exit 1
                               │     15     })                                                                                                          │ 11:52:10  ◆ explorer started
                               │                                                                                                                        │
                               │  The retry now waits 50 ms between attempts; running the suite again.                                                  │
                               │                                                                                                                        │ USAGE                         Oct 2026 · 1.0k
                               │  ⠹ Conjuring…  4s · ↓ 0 tokens · esc to interrupt                                                                      │   M T W T F S S
                               │                                                                                                                        │   · · · · ▪ ■ ·
                               │                                                                                                                        │   ■ ▪ ■ ■ ■ · ·
                               │                                                                                                                        │   ■ ■ ■ ▪ ■ · ·
                               │  ╭──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮  │   ▪ ■ ■ ■ ■ · ·
                               │  │ › hello▏                                                                                                         │  │   ■ ■ ▪ ■ · · ·
                               │  ╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯  │
                               │  manual · ⇧tab to cycle                                                             sonnet-4.5 · effort auto · ctx 3%  │   less ▪ ■ more
 enter send   esc interrupt   ⇧tab mode   /config panels   ctrl+r history   ctrl+o expand   ctrl+c ×2 quit                                              sandbox on  ·  budget 3/50 turns  ·  unverified
```

### 5.3 Chat details

| Element | Style |
|---|---|
| User turn | `›` and the text in `strong`. No box. |
| Assistant text | `body`, rendered with the existing markdown and `highlight`. |
| Thinking | Collapsed: `∴ thought for 2s · ctrl+o to show` in `muted`. Expanded: italic `muted` lines behind a `┆` gutter. |
| Tool row | `◇ Name   target`, with Name in `label` and target in `body`. A right-aligned outcome: spinner while running, then `✓ summary` (`ok`) or `✗ exit n` (`err`). Output sits behind a `│` gutter in `muted`, 3 lines when collapsed (ctrl+o, as today). |
| Sub-agent launch | `◆ name   task   running in sidebar`, with `◆` in `pearl`. |
| Diff | Line numbers in `muted`, then the sign. Added lines on `diff_add_bg` with the sign in `ok`; removed lines on `diff_del_bg` with the sign in `err`. Collapsed unless "Expand diffs" is on. |
| Working line | Spinner, verb, elapsed time, tokens, `esc to interrupt`. |
| Queue | `+2 queued` in `muted`, above the prompt. |
| Prompt | Rounded frame in `border`, caret `▏` in `pearl`. In bash mode the frame turns `info` and the prefix is `!`. |
| Footer (inside the chat) | Left: the mode. `bypass` is the only place a mode is shown in `err`. Right: model · effort · context %. |

### 5.4 Overlays

All overlays sit in the chat column in Columns mode (so the sidebars stay visible) and take the full width in Tabs and chat-only modes. They are framed in `border` with `elevated` text colours. The background is painted only in High contrast.

```
╭─ PERMISSION ────────────────────────────────── expires 4:52 ─╮
│                                                              │
│  Edit  src/login.ts                                   +2 −1  │
│                                                              │
│   12     const r = await retry(fn, {                         │
│   13 −   tries: 2,                                           │
│   13 +   tries: 3,                                           │
│   14 +   backoff: 50,                                        │
│                                                              │
│  Make this edit to src/login.ts?                             │
│ ▍1  Yes                                                      │
│  2  Yes, and allow edits in src/ for this session            │
│  3  No, tell Synthesis what to do instead                    │
│                                                              │
│  ↑↓ choose  enter confirm  esc reject                        │
╰──────────────────────────────────────────────────────────────╯

╭─ BYPASS PERMISSIONS ─────────────────────────────────────────╮   (the only frame drawn in warn)
│  Bypass skips permission prompts for this session.           │
│  Deny rules and protected paths still apply.                 │
│                                                              │
│  enter enable bypass        esc keep auto mode               │
╰──────────────────────────────────────────────────────────────╯

╭─ RESUME ─────────────────────────────────────────── 12 saved ─╮
│  filter  log▏                                                │
│                                                              │
│ ▍fix the flaky login test      proj · main     2h ago        │
│  refactor the log parser       proj · parser   1d ago        │
│  add login rate limiting       api · main      3d ago        │
│                                                              │
│  ↑↓ choose  enter resume  esc cancel                         │
╰──────────────────────────────────────────────────────────────╯
```

The welcome screen (chat column, empty session) shows hand-drawn braille standing in for the generated art:

```
            ⢀⣀⣤⣤⣤⣤⣀⡀
         ⢀⣴⡿⠛⠉⠁  ⠈⠉⠻⣷⣄
        ⣰⡿⠋   ⣠⣶⣦⡀   ⠙⢿⣆
        ⣿⡇  ⢠⡾⠋ ⠙⢷⡄  ⢸⣿
        ⠹⣷⣄⣠⡿⠁   ⠈⢿⣄⣠⣾⠏
          ⠉⠛⠛⠋      ⠙⠛⠛⠉
        S Y N T H E S I S
  Autonomous development environment
       1.0.0 · /work/proj

  Ask Synthesis to explain, fix or build something here.
  /  commands     @  files     !  shell     /config  panels
```

## 6. Panel registry and reflow

### 6.1 Registry

The registry is a static table in `layout.rs`. **Drop** is the order in which panels are dropped when space runs out: 1 goes first; "never" means the panel is locked.

| id | Title | Side | Default | Hideable | min h | want h | max h | flex (weight) | Drop | Content source (existing function) |
|---|---|---|---|---|---|---|---|---|---|---|
| `workspace` | WORKSPACE | L | on | **locked** | 2 | 1+n | max(2, H/4) | 50/50 rule | never | `workspace_panel` |
| `running` | RUNNING | L | on | **locked** | 3 | 1+2n (+arrows) | ∞ | yes (1) | never | `sessions_panel` |
| `session` | SESSION | L | on | yes | 4 | 1+rows | 9 | no | 3 | `session_info_panel` / `_essential` |
| `mcp` | MCP | L | on | yes | 2 | 1+servers | 8 | no | 2 | `mcp_panel` / `mcp_summary_line` |
| `tools` | TOOLS & SKILLS | L | off | yes | 3 | 1+tools(+2) | 10 | no | 1 | `tools_panel` |
| `chat` | — | C | on | **locked** | — | — | — | the whole centre | never | `agent_panel` |
| `status` | STATUS | R | on | yes | 3 | 1+2(+quotas) | 6 | no | 6 | `status_panel`, without its context bar |
| `context` | CONTEXT | R | on | yes | 3 | 3–4 | 4 | no | 5 | context bar + `token_panel` (this replaces Tokens) |
| `tasks` | TASKS | R | on | yes | 3 | 1+todos | 12 | no | 4 | `tasks_panel` |
| `subagents` | SUB-AGENTS | R | on | **locked** | 6 | ∞ | ∞ | yes (2) | never | sub-agent list (≤3 rows) + `transcript_rows`. Empty state: "No sub-agents yet · /agents" |
| `activity` | ACTIVITY | R | off | yes | 4 | ∞ | ∞ | yes (1) | 3 | `feed_panel` |
| `usage` | USAGE | R | off | yes | 9 | 9 | 9 | no | 2 | `usage_panel` (month view only) |
| `topology` | TOPOLOGY | R | off | yes | 12, and inner width ≥ 46 | natural | 32 | no | 1 | `topology_panel` |

The right column's order, top to bottom, is status, context, tasks, subagents, activity, usage, topology. Users cannot reorder panels.

### 6.2 Algorithm

```
plan(W, H, vis, demand) -> Plan                    // pure, no I/O
  if W < 40 or H < 12: return TooSmall
  header = 1 + (H >= 30); bottom = 1
  mode   = Columns if W >= 100 else Tabs if W >= 60 else ChatOnly
  body   = H - header - bottom - (mode == Tabs)
  Columns:
    L = clamp(W*16/100, 22, 34); R = clamp(W*24/100, 30, 56)      // each includes its 1-column rule
    if W-L-R < 48: R = max(30, W-L-48); L = max(22, W-R-48)       // chat keeps at least 48 columns
    left  = fit(LEFT,  L-1, body, left_rule=true)
    right = fit(RIGHT, R-1, body, left_rule=false)
    chat  = Rect(L, header, W-L-R, body)
  Tabs:  Chat → chat=full; Sessions → fit([workspace,running], W, body, true)
         Agents → subagents=full; Panels → fit(visible optional L ++ R, W, body, false)
  ChatOnly: chat = full

fit(spec, colW, H, left_rule) -> [(id, y, h)], starved
  P = [p in spec where p.locked or vis[p]]; starved = []
  move p from P to starved where p.min_width > colW              // topology on a narrow sidebar
  gap = 1
  if Σmin(P) + gap*(|P|-1) > H: gap = 0                           // gaps go first
  while Σmin(P) > H and P has an optional panel:
      drop the optional panel with the lowest Drop value; add it to starved
  if Σmin(P) > H:                                                // only locked panels left, very short column
      h_i = H / |P|, remainder to the flex panel; return
  if left_rule and P == [workspace, running]:                   // owner rule: 50/50 split
      ws = (H-gap)/2; run = H-gap-ws; return
  h = min for each p; spare = H - Σh - gap*(|P|-1)
  for p in P where not flex (in registry order):                // fixed panels grow to their want
      t = min(spare, min(want_p, max_p) - h_p); h_p += t; spare -= t
  while spare > 0 and there are flex panels below their want:   // flex panels share by weight
      share spare by weight, capping each at its want; take back what a capped panel left over
  h[last(P)] += spare                                            // leftover goes to the bottom panel: never a hole between panels
  lay out top to bottom with gap rows
```

**Inside a panel.** When content is taller than `h-1` rows, the panel shows a window with `↑ n more` / `↓ n more` rows and keeps the selection visible (the existing `sessions_window`). The mouse wheel scrolls the panel under the pointer. The sub-agent transcript follows the tail unless the user has scrolled it (the existing `dock_scroll`).

**Resize.** The plan has no state. Scroll offsets are clamped. When the focused panel gets dropped, focus moves to the chat.

**Worked examples** (body = 33 at 120×36; 47 at 200×50):

| Case | Allocation |
|---|---|
| Default left, 120×36 | ws 2, running 5, session 5, mcp 2+16 (the trailing rows). Total 33 = 2+1+5+1+5+1+18. |
| Left with every optional panel hidden, 120×36 | ws 16, gap 1, running 16 |
| Right with every optional panel hidden | subagents 33 (the whole column) |
| Right at 200×50 with status, context, tasks, activity and usage | status 4, context 3, tasks 4, subagents 15, activity 7, usage 9, plus 5 gaps = 47 |
| 100×16 (body 14), default left | ws 2, running 3, session 4, mcp 2, plus 3 gaps = 14. At 15 rows or fewer, gaps are dropped before panels. |

### 6.3 Invariants (property tests, no new dependency)

The tests sweep W from 40 to 260 and H from 12 to 80 with 8 representative visibility masks, then all 512 masks at 6 sizes. They assert:

1. The rectangles of each column tile the body exactly: no overlap, and nothing between panels except gap rows.
2. Locked panels are always present with h ≥ 1. The chat is at least 48 columns wide in Columns mode.
3. The 50/50 rule holds exactly, and the sub-agent panel gets the whole right column when it is the only visible right panel.
4. Hiding an optional panel never makes a locked panel shorter, unless a panel that had been dropped for lack of room comes back.
5. The plan is deterministic, and a resize back to the previous size restores the previous plan.

## 7. `/config`

`/config` opens a frame like the overlays in §5.4: in the chat column in Columns mode, so changes preview live in the sidebars, and full screen in the other modes.

```
╭─ C O N F I G ──────────────────────────────────── esc close ─╮
│  / filter                                                    │
│                                                              │
│  PANELS   ·   Appearance   ·   Behaviour   ·   Mouse         │
│                                                              │
│  LEFT SIDEBAR                                                │
│  ▣ Workspace          locked · always visible                │
│  ▣ Running sessions   locked · always visible                │
│ ▍● Session            model, mode, effort, branch            │
│  ● MCP                servers and tool counts                │
│  ○ Tools & skills     calls this session                     │
│                                                              │
│  CHAT                                                        │
│  ▣ Chat               locked · always visible                │
│                                                              │
│  RIGHT SIDEBAR                                               │
│  ▣ Sub-agents         locked · always on the right           │
│  ● Status             phase, running tool, quotas            │
│  ● Context            meter, tokens, cost                    │
│  ● Tasks              todo list                              │
│  ○ Activity           event feed                             │
│  ○ Usage              daily heatmap                          │
│  ○ Topology           needs a 46-column sidebar              │
│                                                              │
│  ↑↓ move  space toggle  tab group  / filter  r reset  esc    │
╰──────────────────────────────────────────────────────────────╯
```

**The other groups:**
- **Appearance:**
  - Theme: Graphite / Midnight / High contrast / Mono (+ Pearl if D5 is accepted).
  - Glyphs: Unicode / ASCII.
  - Logo: Line art / Inline image (shown only when a protocol is detected) / Off.
- **Behaviour:** Show thinking, Expand diffs, Queue indicators, Motion.
- **Mouse & clipboard:** Mouse capture (off means the terminal's own selection) and Copy on select. Both are new switches in `runtime.rs` and `terminal.rs`; see D9.

**Keys:**

| Key | Action |
|---|---|
| ↑ ↓ | Move between rows. Locked rows can be focused too. |
| Space, Enter | Toggle, or cycle a choice. |
| ← → | Choose within a choice row. |
| Tab, Shift+Tab | Next or previous group. Inside `/config` this replaces the mode-cycling meaning of Shift+Tab. |
| `/` | Filter across all groups with `picker::matches`. |
| Backspace | Edit the filter. |
| `r` | Reset the focused group to its defaults. |
| Esc | Clear the filter first; a second Esc closes. |
| PgUp, PgDn | Scroll. |
| Mouse | A click toggles, the wheel scrolls. |

**Locked rows.**
- Space on a locked row changes nothing. The hint line shows the reason instead: "Workspace is always visible: it switches workspaces." "Sub-agents always live in the right sidebar."
- A starved panel shows `on · no room at this size`.
- While `NO_COLOR` is set, the Theme row is locked to Mono with the reason "NO_COLOR is set".

**Live preview.** Every change updates the app's in-memory `TuiSettings`, and the next frame uses it. Nothing is written until the screen closes.

**Persistence.** On close, and only when something changed, `synthesis_settings::persist_user_tui(home, tui: &Value)`:
1. Re-reads `$SYNTHESIS_HOME/settings.json`.
2. Stops if the file cannot be parsed. It shows the toast "settings.json is unreadable; changes kept for this session" and never overwrites the file, following the pattern in `lib.rs:284`.
3. Otherwise replaces only the `synthesis.tui` subtree and writes through a temp file plus rename. Key order is kept because `preserve_order` is on.

`synthesis.tui` in the project, local or managed files is ignored, with the diagnostic "synthesis.tui is read from user settings only".

**Schema and defaults:**

```json
{"synthesis": {"tui": {
  "theme": "graphite",
  "glyphs": "auto",
  "logo": "art",
  "panels": {"session": true, "mcp": true, "tools": false, "status": true, "context": true,
             "tasks": true, "activity": false, "usage": false, "topology": false},
  "chat": {"show_thinking": true, "diffs_expanded": false, "queue_indicators": true},
  "motion": true,
  "mouse": {"capture": true, "copy_on_select": true}
}}}
```

- **Allowed values:** `theme` is `graphite|midnight|high-contrast|mono` (plus `pearl` if D5). `glyphs` is `auto|unicode|ascii`. `logo` is `art|image|off`.
- **Panel ids:** an unknown id is ignored with a diagnostic. A locked id (`workspace`, `running`, `chat`, `subagents`) is ignored with the diagnostic "<id> is always visible".
- **Bad values:** a wrong type keeps the default, with a diagnostic that names the key but never the value. The parser is `TuiSettings::from_settings(&Value) -> (Self, Vec<String>)`, following the `AutoCompact` pattern (`harness.rs:65`).
- **Where diagnostics appear:** the `/config` hint line and `synth doctor`.

**Test hooks (no new CLI flag):**
- `config_view::render(&ConfigModel, w, h)` is pure, so it gets goldens.
- `App` unit tests drive `/config` through keys, as in the existing `app/tests.rs` `screen(&mut a)`.
- `synth --once` with `SYNTHESIS_HOME` pointing at a temp directory that contains `settings.json` renders any panel set. Whether `--once` respects `COLUMNS`/`LINES` is `[assumption]` and needs checking.
- `tests/tty-qa/qa_config.py` toggles rows, asserts the reflow and checks the written file.

## 8. Implementation within the current architecture

1. **Theme layer** (`theme.rs`):
   - `struct Theme` holds the tokens as `Style`.
   - `const GRAPHITE, MIDNIGHT, HIGH_CONTRAST, MONO, ANSI16` (and `PEARL`).
   - `thread_local! CURRENT`, with `pub fn t() -> &'static Theme` and a `ThemeScope` guard set at the entry points.
   - A `glyphs()` table with Unicode and ASCII sets.
   - `contrast(a, b)` and the contrast tests.
   - The old constants are replaced site by site through the §3 mapping. `fit`, `fit_items`, `lerp` and `spaces` stay.
2. **Layout engine:** a new `layout.rs` holding the registry, `plan`, `fit` and the property tests. `render_frame` becomes: build the plan, render each panel id into its rectangle, assemble the frame. `boxed` becomes `section` (a label row and content, no frame).
   - The `Rects` fields stay (`session_rows`, `workspace_rows`, `activity_rect`, `dock_rect`, `chat_rect`, `caret`) but are filled from the plan, so the `app.rs` hit tests change very little.
   - `layout_for`, `topology_column`, `agent_detail` and `tasks_usage_row` are deleted once nothing calls them.
3. **Panels:** reuse the existing panel functions; only their chrome and colours change.
4. **Goldens.** The 95 render goldens are rebaselined on purpose, behind a gate:
   - `SYNTHESIS_UPDATE_GOLDEN=redesign-p1|p2|p3` rewrites only the cases on that phase's allow-list.
   - **P1:** only `expected.ansi.json`, plus the header rows of frame cases. The tool fails if any other text changes.
   - **P2:** frame and dash text and rectangles.
   - **P3:** agent, dialog and picker cases.
   - Each rewrite sets `meta.json.reviewed_by` to `null`. A new assertion in `golden.rs` fails on `null`, so CI stays red until the owner writes `reviewed_by: owner:DevOnChain101 <date> (redesign Pn)` after reviewing the contact sheet. That is the approval gate.
   - `tools/golden-contact/contact.py` (standard library only) converts `expected.ansi.json` from two git revisions (`git show`) into one HTML page with before/after side by side, a text diff and a phase filter, written to `target/golden-contact/`. It runs locally, not in CI.
   - New cases: plans at 80×24, 100×30, 120×36, 140×40 and 200×50 with three visibility sets each (defaults, all optional off, all on); `/config` in each group; a locked-row hint; one frame per theme, plus `NO_COLOR` and 16-colour.
5. **tty-qa:** `qa_visual.py` captures the four owner sizes for each theme, for `NO_COLOR=1` and for `TERM=xterm`. It asserts:
   - no row is wider than the terminal;
   - no glyph is two cells wide;
   - Mono output carries no colour;
   - the bottom bar is present;
   - `/config` toggles reflow the screen.

   It runs on macOS and Linux. On Windows the code is checked in CI, but ConPTY rendering stays `unverified` until someone checks it on a real machine.
6. **Accessibility:**
   - `NO_COLOR` gives Mono.
   - The High contrast theme.
   - The ASCII glyph set.
   - The Motion toggle.
   - No meaning carried by colour alone.
   - Focus is shown by a glyph as well as colour.
   - Screen-reader users still have `synth run` headless; the alternate screen is not made accessible.
7. **Performance:** the planner is O(panels) per frame. `tests/perf.rs` must stay within its current budget.

## 9. Phases (files to touch)

| Phase | Work | Files | Exit criteria |
|---|---|---|---|
| **P1** Tokens, header, logo | Themes and tokens, colour sweep, header mark and wordmark, logo art generator, theme read from user settings (no `/config` yet), contrast and glyph-width tests | render: `theme.rs`, new `logo.rs`, `dash.rs` (`header`, colours), `agent.rs`, `flow.rs`, `dialogs.rs`, `picker.rs`; tui: `app.rs` (theme choice), new `tui_settings.rs`, `status.rs` (colours); `tools/logo-art/gen.py` | Only ANSI files and header rows changed (enforced by the tool); contact sheet approved; CI green on 3 OSes |
| **P2** Layout and `/config` | Planner and registry, sections, sub-agent panel always on, Tabs and chat-only modes, centre panels moved to the right, merged bottom bar, `/config` with persistence | render: new `layout.rs`, new `config_view.rs`, `dash.rs` (`render_frame` rewritten); tui: `app.rs` (`/config` command and keys, Tab view cycling), `status.rs`; settings: `lib.rs` (`persist_user_tui`); goldens, `golden.rs` | Property tests green; new layout goldens; frame/dash rebaseline approved; `qa_config.py` green on macOS and Linux |
| **P3** Chat, diffs, dialogs | Tool rows, thinking, diff gutter, prompt, footer, welcome with art, approval with diff, bypass, pickers and resume | render: `agent.rs`, `agent/diffview.rs`, `agent/thinking.rs`, `dialogs.rs`, `picker.rs` | agent/picker/dialog rebaseline approved |
| **P4** Polish and docs | Motion, ASCII audit, Mouse group (coordinated with whoever owns `terminal.rs`), optional inline image, optional OSC 11 light-background detection, docs, final contact sheet, 3-OS tty-qa | `runtime.rs`, `terminal.rs`; docs: `design/README.md` (TUI line), `docs/specs/README.md`, `tests/golden/README.md` | Every golden reviewed, nothing in an unreviewed state |

P2 and P3 change `app.rs`. Start them after the current `terminal.rs`, `e2e.rs` and `app.rs` work has merged.

## 10. Risks

| Risk | Mitigation |
|---|---|
| Golden churn hides real regressions | Phase allow-lists (P1 may change colour only), the contact sheet, and the `reviewed_by` gate |
| Ambiguous-width glyphs misalign in CJK locales | Glyph-width test, ASCII set, the `auto` rule |
| A light terminal makes Graphite unreadable | Mono or Pearl (D5); OSC 11 detection later |
| Thread-local theme state | Set only by the entry points; tests set it explicitly; Graphite is the default |
| A user edits `settings.json` while the TUI is open | Re-read before writing, never overwrite an unparsable file, atomic rename, write only on close |
| 256 and 16 colours flatten the palette (Midnight's blue, the pearl's warmth, the diff tints) | Meaning also rides on glyphs; this loss is accepted |
| Moving the Mission-Control panels changes muscle memory | Defaults keep Status, Context and Tasks visible on the right |
| Inline images under tmux or in diff redraws | Off by default, welcome screen only, never inside tmux or screen |
| Concurrent edits to `app.rs` and `terminal.rs` | Sequencing in §9 |

## 11. Owner decisions

- **D1 Panel list.** Merge Tokens into a new Context panel. Move Activity to the right. Keep Topology as optional and off by default, or drop it.
- **D2 Defaults.** On: Session, MCP, Status, Context, Tasks. Off: Tools, Activity, Usage, Topology.
- **D3 Logo.** Braille art (recommended) or half-block. Inline image yes/no (recommended: optional, off by default, welcome screen only).
- **D4 Palette.** Adopt the kit fully plus the `[derived]` tokens (body `#C8CCD2`, muted, status colours, diff tints), or use the kit's colours only, which makes all body text `#9CA3AF`.
- **D5 Light terminals.** Add a Pearl theme (recommended). Later, automatic detection through OSC 11.
- **D6 Background.** Never paint the terminal background (recommended), or paint `#0B0C0F` everywhere.
- **D7 Chrome.** Borderless sections (recommended) or rounded boxes.
- **D8 Misc layout.** One merged bottom bar; minimum size 40×12; Tab cycles the views in Tabs mode.
- **D9 Scope of the Behaviour and Mouse groups.** Motion, mouse capture and copy-on-select are new runtime switches. Ship them in P4, or drop them.
- **D10 No-agent dashboard.** Retire the no-agent dashboard path and its `dash-*` goldens (only goldens reach it), or restyle it as well.
- **D11 `CLAUDE_MONITOR_COLOR`** (`sgr.rs:30`). Rename it to `SYNTHESIS_COLOR` or remove it (see F1).

## 12. Proposed changes to the design doc (for the lead to apply)

- **Revision history:** add `| 1.5 | 2026-10-29 | TUI redesign: FR-16, ADR-014 (proposed); spec docs/specs/tui-design.md |`. The header says Version 1.3 while the history already lists 1.4.
- **§3.1:** add `| FR-16 | Branded TUI: semantic theme tokens (kit palette) with truecolor/256/16/NO_COLOR fallbacks, logo mark and wordmark, a fixed panel registry with locked Workspace/Running/Chat/Sub-agents panels, reflow on hide/show, and a /config screen persisted in user settings only | P1 |`.
- **§9:** add `| ADR-014 | **Status: Proposed.** TUI theme is a set of semantic tokens chosen per render call (scoped thread-local); layout is a pure planner over a static panel registry; TUI preferences live only under `synthesis.tui` in the user settings file | Keeps the renderer deterministic and golden-testable, reuses the existing panel functions, needs no new dependency. Cost: one deliberate rebaseline of the render goldens gated by owner review, hidden per-call theme state, and a layout change for current users |`.

## Follow-ups for the planner

- **F1:** `CLAUDE_MONITOR_COLOR` in `sgr.rs:30` breaks the environment-variable rule.
- **F2:** Check that `--once` respects `COLUMNS` and `LINES`.
- **F3:** Check how 16-colour terminals render braille, and the Windows conhost font.
- **F4:** Golden `agent-welcome-empty-70x24/expected.txt` row 11 is wider than 70 columns. It looks like a leftover from the rename. Check whether `golden_divergences.txt` lists it.
- **F5:** Apply the §12 changes and add this spec to `docs/specs/README.md`.
