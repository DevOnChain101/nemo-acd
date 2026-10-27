#!/usr/bin/env node
// Fails (exit 1) when any `uses:` in the given workflow/action files is not pinned to a 40-hex commit SHA.
// Local actions (./path) are exempt; docker:// refs and tags/branches are not. Run by release.yml, not by ordinary CI.
//   node scripts/check-action-pins.mjs [file...]   (default: every .github/workflows/*.y?ml)
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const files = process.argv.length > 2 ? process.argv.slice(2)
  : readdirSync(".github/workflows").filter((f) => /\.ya?ml$/.test(f)).map((f) => join(".github/workflows", f));
const bad = [];
for (const f of files) {
  readFileSync(f, "utf8").split("\n").forEach((line, i) => {
    const m = line.match(/^\s*(?:-\s+)?uses:\s*["']?([^\s"'#]+)/);
    if (!m || m[1].startsWith("./")) return;
    if (!/^[^@\s]+@[0-9a-f]{40}$/.test(m[1])) bad.push(`${f}:${i + 1}: ${m[1]}`);
  });
}
if (bad.length) {
  console.error(`check-action-pins: ${bad.length} action(s) not pinned to a full commit SHA:\n  ${bad.join("\n  ")}\nSee docs/RELEASE.md "Action pinning".`);
  process.exit(1);
}
console.log(`check-action-pins: all actions pinned (${files.length} files)`);
