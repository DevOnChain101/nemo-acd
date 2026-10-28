"""Tiny pty + VT driver for black-box TUI QA. Python 3.9+, needs `pyte` (install in a venv, never globally).

Never prints the environment; the provider key reaches the child only through `zsh -ic` (the user's rc files).
"""
import fcntl, os, pty, re, select, signal, struct, subprocess, sys, termios, time
import pyte

MODES = {  # DECSET/DECRST modes whose final state we track from the raw byte stream
    "alt_screen": (1049, 1047, 47), "mouse_press": (1000,), "mouse_drag": (1002,), "mouse_any": (1003,),
    "mouse_sgr": (1006,), "paste": (2004,), "cursor_hidden": (25,), "focus": (1004,), "kitty_kbd": (),
}
_DEC = re.compile(rb"\x1b\[\?([0-9;]+)([hl])")


class Session:
    def __init__(self, argv, cwd, rows=40, cols=120, env=None, shell=True, scrollback=2000):
        self.rows, self.cols = rows, cols
        self.screen = pyte.HistoryScreen(cols, rows, history=scrollback, ratio=1.0)
        self.stream = pyte.ByteStream(self.screen)
        self.raw = bytearray()
        self.modes = {}  # decset number -> bool
        self.mark = 0
        e = dict(os.environ)
        e.update({"TERM": "xterm-256color", "COLORTERM": "truecolor"}); e.pop("NO_COLOR", None)
        e.update(env or {})
        cmd = argv if not shell else ["zsh", "-ic", "exec " + " ".join(_q(a) for a in argv)]
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execvpe(cmd[0], cmd, e)
        self._winsz(rows, cols)
        self.exit_status = None

    def _winsz(self, rows, cols):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def resize(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.screen.resize(rows, cols)
        self._winsz(rows, cols)  # the kernel raises SIGWINCH for the foreground group itself

    def pump(self, t=0.05):
        end = time.time() + t
        while True:
            r, _, _ = select.select([self.fd], [], [], max(0, end - time.time()))
            if not r: return
            try: d = os.read(self.fd, 65536)
            except OSError: self._reap(); return
            if not d: self._reap(); return
            self.raw += d
            for m in _DEC.finditer(d):
                for n in m.group(1).split(b";"):
                    self.modes[int(n)] = m.group(2) == b"h"
            self.stream.feed(d)

    def _reap(self):
        if self.exit_status is None:
            try:
                p, st = os.waitpid(self.pid, os.WNOHANG)
                if p: self.exit_status = os.waitstatus_to_exitcode(st) if hasattr(os, "waitstatus_to_exitcode") else st
            except ChildProcessError: self.exit_status = -1

    def alive(self):
        self._reap(); return self.exit_status is None

    # ---- input
    def send(self, data, settle=0.15):
        if isinstance(data, str): data = data.encode()
        os.write(self.fd, data); self.pump(settle)

    def type(self, s, delay=0.01):
        for ch in s:
            os.write(self.fd, ch.encode()); self.pump(delay)
        self.pump(0.1)

    def paste(self, s):
        self.send(b"\x1b[200~" + s.encode() + b"\x1b[201~", 0.4)

    def mouse(self, kind, x, y, mods=0):
        """kind: left|middle|right|release|drag|wheelup|wheeldown; x,y 1-based cell."""
        b = {"left": 0, "middle": 1, "right": 2, "drag": 32, "wheelup": 64, "wheeldown": 65}.get(kind, 0) | mods
        end = "m" if kind == "release" else "M"
        self.send(f"\x1b[<{b};{x};{y}{end}".encode(), 0.05)

    def click(self, x, y, btn="left", mods=0):
        self.mouse(btn, x, y, mods); self.mouse("release", x, y, mods)

    def drag(self, x1, y1, x2, y2, mods=0, steps=4):
        self.mouse("left", x1, y1, mods)
        for i in range(1, steps + 1):
            self.mouse("drag", x1 + (x2 - x1) * i // steps, y1 + (y2 - y1) * i // steps, mods)
        self.mouse("release", x2, y2, mods)

    # ---- output
    def lines(self, rstrip=True):
        return [l.rstrip() if rstrip else l for l in self.screen.display]

    def text(self):
        ls = self.lines()
        while ls and not ls[-1]: ls.pop()
        return "\n".join(ls)

    def new_raw(self):
        d = bytes(self.raw[self.mark:]); self.mark = len(self.raw); return d

    def wait(self, pat, timeout=30, quiet=0.0):
        """Wait until regex/substr appears on the visible screen. Returns True/False."""
        rx = re.compile(pat) if not isinstance(pat, re.Pattern) else pat
        end = time.time() + timeout
        while time.time() < end:
            self.pump(0.1)
            if rx.search(self.text()): return True
            if not self.alive(): self.pump(0.1); return bool(rx.search(self.text()))
        return False

    def wait_idle(self, quiet=1.5, timeout=60):
        """Wait until no output for `quiet` seconds."""
        end = time.time() + timeout; last = len(self.raw); t0 = time.time()
        while time.time() < end:
            self.pump(0.1)
            if len(self.raw) != last: last, t0 = len(self.raw), time.time()
            elif time.time() - t0 >= quiet: return True
        return False

    def attrs(self, y, x0=0, x1=None):
        """Per-cell (char, fg, bg, bold, reverse, italic, underline, strike) for row y (0-based)."""
        row = self.screen.buffer[y]; out = []
        for x in range(x0, x1 if x1 is not None else self.cols):
            c = row[x]; out.append((c.data, c.fg, c.bg, c.bold, c.reverse, c.italics, c.underscore, c.strikethrough))
        return out

    def dump(self, path, title=""):
        c = self.screen.cursor
        with open(path, "a") as f:
            f.write(f"===== {title} [{self.cols}x{self.rows}] cursor=({c.x},{c.y}) hidden={c.hidden} modes={ {k:v for k,v in sorted(self.modes.items())} }\n")
            for i, l in enumerate(self.lines()): f.write(f"{i:02d}|{l}\n")
            f.write("\n")

    def scrollback_text(self):
        return "\n".join("".join(r[x].data for x in range(self.cols)).rstrip() for r in self.screen.history.top)

    def close(self, sig=signal.SIGTERM):
        if self.alive():
            try: os.killpg(os.getpgid(self.pid), sig)
            except OSError: pass
            time.sleep(0.3); self._reap()
            if self.exit_status is None:
                try: os.kill(self.pid, signal.SIGKILL)
                except OSError: pass
        try: os.close(self.fd)
        except OSError: pass


def _q(a):
    return "'" + a.replace("'", "'\\''") + "'"


KEYS = {"enter": b"\r", "esc": b"\x1b", "tab": b"\t", "backtab": b"\x1b[Z", "up": b"\x1b[A", "down": b"\x1b[B",
        "right": b"\x1b[C", "left": b"\x1b[D", "home": b"\x1b[H", "end": b"\x1b[F", "pgup": b"\x1b[5~", "pgdn": b"\x1b[6~",
        "bksp": b"\x7f", "alt-bksp": b"\x1b\x7f", "alt-enter": b"\x1b\r", "shift-enter": b"\x1b[13;2u", "del": b"\x1b[3~",
        "alt-b": b"\x1bb", "alt-f": b"\x1bf", "alt-left": b"\x1b[1;3D", "alt-right": b"\x1b[1;3C",
        **{f"ctrl-{c}": bytes([ord(c) - 96]) for c in "abcdefghijklmnopqrstuvwxyz"}}


def key(s, name, settle=0.15):
    s.send(KEYS[name], settle)


def make_repo(path, kind="rust"):
    """Throwaway git repo with a tiny project. kind: rust | node | empty"""
    import shutil
    shutil.rmtree(path, ignore_errors=True); os.makedirs(path)
    def w(p, t):
        os.makedirs(os.path.dirname(os.path.join(path, p)) or path, exist_ok=True)
        open(os.path.join(path, p), "w").write(t)
    w("README.md", "# demo\n\nTiny QA project.\n")
    w("notes.txt", "line one\nline two\nline three\n")
    if kind == "rust":
        w("Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n')
        w("src/lib.rs", "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn adds() {\n        assert_eq!(add(1, 2), 3);\n    }\n}\n")
    elif kind == "node":
        w("package.json", '{"name":"demo","version":"1.0.0","scripts":{"test":"node --test"}}\n')
        w("index.js", "exports.add = (a, b) => a + b;\n")
        w("index.test.js", "const t=require('node:test');const a=require('node:assert');const {add}=require('./index');\nt('add',()=>a.strictEqual(add(1,2),3));\n")
    w(".gitignore", "target/\nnode_modules/\n")
    env = dict(os.environ, GIT_AUTHOR_NAME="qa", GIT_AUTHOR_EMAIL="qa@x", GIT_COMMITTER_NAME="qa", GIT_COMMITTER_EMAIL="qa@x")
    for c in (["git", "init", "-q", "-b", "main"], ["git", "add", "-A"], ["git", "commit", "-qm", "init"]):
        subprocess.run(c, cwd=path, env=env, check=True, stdout=subprocess.DEVNULL)
    return path
