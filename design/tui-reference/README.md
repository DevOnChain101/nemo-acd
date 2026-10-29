# TUI reference mockups

The owner-approved visual reference for the Synthesis terminal UI (supplied 2026-10-30). The inventory of
what each shows, how the two merge, the panel-id mapping and the acceptance criteria are in
`docs/specs/tui-design.md` section 14 ("Owner reference mockups (authoritative)").

| File | Shows |
|---|---|
| `mockup-A-chat.png` | Chat-centred: Workspace tree, Session, Tools & Skills, Activity on the left; chat with rich tool cards (Reading file, Searching, Running, Applying patch, Verifying), tab strip chat/diff/terminal/browser/tests/files and prompt in the centre; capability topology and usage on the right. |
| `mockup-B-dashboard.png` | Dashboard-centred: Workspace list, Session, Tools & Skills, Recent activity, Session stats on the left; Agent Status and Context cards, tab strip TERMINAL/agent/workspace/diff, project tree + diff + terminal split, prompt with model chip and slash-command bar in the centre; topology hub, Usage & Metrics, System Health on the right. |

Notes
- Both images still show the old second title after "Synthesis" in the header. The owner removed it: the title is
  "Synthesis" only. Everything else is the target, within terminal limits (no images, shadows or glow).
- Palette: `tools/sample-mockup-palette.py` (standard library only) samples the dominant colours; the default theme
  `synthesis` is built from them (spec section 14.7).
