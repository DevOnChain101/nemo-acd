"""Smoke scenarios for the real TUI: startup, prompt editing, slash picker, unknown command, exit restore.

Run: <venv>/bin/python tests/tty-qa/qa_smoke.py   (env: NEMO_BIN, QA_OUT; add --live for one real-provider turn)
Screens are appended to $QA_OUT/smoke.txt. Exit code 1 if any check fails.
"""
import os, sys, tempfile, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ttyqa import Session, key, make_repo

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BIN = os.environ.get("NEMO_BIN", os.path.join(ROOT, "target", "debug", "nemo"))
OUT = os.environ.get("QA_OUT") or tempfile.mkdtemp(prefix="ttyqa-")
FAILS = []


def check(name, ok, s=None):
    print(("PASS " if ok else "FAIL ") + name)
    if not ok:
        FAILS.append(name)
        if s: s.dump(os.path.join(OUT, "smoke.txt"), "FAIL " + name)


def main():
    repo = make_repo(os.path.join(OUT, "repo"), "rust")
    # shell=True runs through `zsh -ic` so the provider key from your rc files reaches the child (never printed)
    s = Session([BIN, "--cwd", repo], repo, rows=40, cols=120, shell="--live" in sys.argv)
    check("first frame", s.wait("Enter send|ready", 20), s)
    s.pump(1)
    check("alt-screen + mouse + paste modes on", all(s.modes.get(m) for m in (1049, 1000, 1006, 2004)), s)
    s.type("hello wrld"); key(s, "left"); key(s, "left"); key(s, "left"); s.type("o")
    check("prompt editing inserts at cursor", "hello world" in s.text(), s)
    key(s, "ctrl-u"); key(s, "ctrl-k"); key(s, "ctrl-a"); key(s, "ctrl-k")
    s.type("/"); check("slash picker opens", s.wait("/help", 5), s)
    key(s, "ctrl-u"); s.type("/bogus"); key(s, "enter")
    check("unknown command message", s.wait("unknown command", 5), s)
    if "--live" in sys.argv:
        s.type("say hi in 3 words"); key(s, "enter")
        check("live turn finishes", s.wait("Done ·", 90), s)
    key(s, "ctrl-d"); s.pump(1.5)
    check("exit code 0", not s.alive() and s.exit_status == 0, s)
    check("terminal restored (alt-screen, mouse, paste off; cursor on)",
          not any(s.modes.get(m) for m in (1049, 1000, 1002, 1006, 2004)) and s.modes.get(25) is True, s)
    s.close()
    print(f"{len(FAILS)} failure(s); screens in {OUT}")
    sys.exit(1 if FAILS else 0)


main()
