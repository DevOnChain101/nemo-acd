#!/usr/bin/env node
// Fails (exit 1) when a tracked path cannot be checked out on Windows: characters <>:"|?* or control chars,
// a trailing dot/space, a reserved device name (CON, NUL, COM1 ... also with an extension), a path longer than
// 240 chars, or two paths that differ only by case (they collide on Windows and default macOS).
//   node scripts/check-portable-paths.mjs
import { execFileSync } from "node:child_process";

const files = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter(Boolean);
const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
const bad = [];
const seen = new Map();
for (const f of files) {
  const why = [];
  if (/[<>:"|?*\x00-\x1f]/.test(f)) why.push("invalid character");
  if (f.length > 240) why.push(`too long (${f.length} chars)`);
  for (const part of f.split("/")) {
    if (/[. ]$/.test(part)) why.push(`trailing dot/space in "${part}"`);
    if (RESERVED.test(part.split(".")[0])) why.push(`reserved name "${part}"`);
  }
  const key = f.toLowerCase();
  if (seen.has(key)) why.push(`differs only by case from ${seen.get(key)}`);
  else seen.set(key, f);
  if (why.length) bad.push(`  ${JSON.stringify(f)}: ${why.join("; ")}`);
}
if (bad.length) {
  console.error(`check-portable-paths: ${bad.length} tracked path(s) are not portable to Windows:\n${bad.join("\n")}`);
  process.exit(1);
}
console.log(`check-portable-paths: ${files.length} tracked paths ok`);
