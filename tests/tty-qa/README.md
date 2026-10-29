# tty-qa: black-box driver for the real Synthesis TUI

Spawns `target/debug/synth` in a pseudo-terminal, feeds it keys / SGR mouse / bracketed paste, renders the
byte stream through a VT screen model ([`pyte`](https://pypi.org/project/pyte/)) and lets a script assert on
or dump what a user would see. Python 3.9+ stdlib plus `pyte`. It never touches product code.

## Setup (never `pip install` globally)

```sh
python3 -m venv /path/to/venv && /path/to/venv/bin/pip install pyte
cargo build -p synthesis-cli            # builds target/debug/synth
/path/to/venv/bin/python tests/tty-qa/qa_smoke.py          # offline smoke (no provider call)
/path/to/venv/bin/python tests/tty-qa/qa_smoke.py --live   # + one real turn through your configured provider
```

Copy the binary first (`cp target/debug/synth /tmp/synthesis-qa`, then `SYNTHESIS_BIN=/tmp/synthesis-qa`) when someone else is
rebuilding the workspace; a half-written binary gives confusing results.

## API (`ttyqa.py`)

```python
from ttyqa import Session, key, make_repo
repo = make_repo("/tmp/qa/repo", "rust")            # throwaway git repo: rust | node | empty
s = Session([BIN, "--cwd", repo], repo, rows=40, cols=120,
            shell=True,        # True: run via `zsh -ic 'exec ...'` so the provider key from your rc files reaches the
                               # child (the key is never read, printed or logged by the driver); False: direct exec
            env={"NO_COLOR": "1"})
s.wait(r"Enter send", 20)    # regex on the visible screen; True/False
s.type("hello"); key(s, "enter")   # key names: enter esc tab backtab up down left right home end pgup pgdn bksp
                                   # alt-bksp alt-enter shift-enter(CSI u) alt-b alt-f ctrl-a..ctrl-z del
s.paste("text\nwith newline")      # bracketed paste
s.click(x, y, btn="right")         # SGR mouse, 1-based cells; s.drag(x1,y1,x2,y2,mods=4) (4 = Shift); s.mouse("wheelup", x, y)
s.resize(24, 50)                   # TIOCSWINSZ (kernel raises SIGWINCH)
s.text(); s.lines(); s.attrs(row)  # screen text; per-cell (char, fg, bg, bold, reverse, italic, underline, strike)
s.modes                            # final DECSET state from the raw stream: 1049 alt-screen, 1000/1002/1006 mouse,
                                   # 2004 paste, 25 cursor visible
s.scrollback_text(); s.new_raw()   # lines scrolled off the top; raw bytes since last call
s.dump("out.txt", "title")         # screen + cursor + modes appended as numbered text
s.alive(); s.exit_status; s.close()
```

Notes and gotchas:

- The TUI draws a software cursor (a coloured cell) and hides the hardware cursor, so use `attrs()`, not
  `screen.cursor`, to find the caret.
- `pump(t)` drains output for the full `t` seconds. Check the screen after a wait, not right after a send.
- `termios.tcgetattr(s.fd)` on the master returns the slave's line discipline: ICANON/ECHO/ISIG are off while the
  app runs and must be back on after exit (see the "exit paths" checks in `qa_smoke.py`).
- Clipboard checks: save `pbpaste` first and restore it afterwards (`pbcopy`).
- Copy-on-select can be verified with `ps` (`pgrep -x pbcopy`) plus `pbpaste`; OSC 52 shows up in `s.raw`.
- Shift+drag and Option+drag are handled by the host terminal, so a pty cannot test them.
- Real-provider scenarios cost money: keep prompts short and use the cheapest model.

Full findings from the first run are in `docs/research/tty-qa-report.md`.
