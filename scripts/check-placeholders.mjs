#!/usr/bin/env node
// Release gate for the repository placeholder. The single source of the GitHub slug is packaging/REPO.
//   node scripts/check-placeholders.mjs [--expect owner/repo]   fail if the slug is malformed, still OWNER, or differs from --expect
//                                                               (case-insensitive), or if OWNER remains in README/INSTALL/templates (installers hold their own guard and are stamped by release.yml)
//   node scripts/check-placeholders.mjs --set owner/repo        write packaging/REPO and replace OWNER/Nemo-ACD in README.md and docs/INSTALL.md
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const { values: a } = parseArgs({ options: { expect: { type: "string" }, set: { type: "string" } } });
const SLUG = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const docs = ["README.md", "docs/INSTALL.md"];
const walk = (d) => readdirSync(d).flatMap((f) => (statSync(join(d, f)).isDirectory() ? walk(join(d, f)) : [join(d, f)]));
const fail = (m) => { console.error(`check-placeholders: ${m}`); process.exit(1); };

if (a.set) {
  if (!SLUG.test(a.set) || /\bOWNER\b/.test(a.set)) fail(`bad slug ${a.set}`);
  writeFileSync("packaging/REPO", a.set + "\n");
  for (const f of docs) writeFileSync(f, readFileSync(f, "utf8").replaceAll("OWNER/Nemo-ACD", a.set));
  console.log(`set ${a.set}`);
  process.exit(0);
}
const slug = readFileSync("packaging/REPO", "utf8").trim();
if (!SLUG.test(slug)) fail(`packaging/REPO is not owner/repo: ${JSON.stringify(slug)}`);
if (/\bOWNER\b/.test(slug)) fail(`packaging/REPO still holds the placeholder ${slug}; run: node scripts/check-placeholders.mjs --set <owner>/<repo>`);
if (a.expect && a.expect.toLowerCase() !== slug.toLowerCase()) fail(`packaging/REPO (${slug}) != ${a.expect}`);
const left = [...docs, ...walk("packaging/homebrew"), ...walk("packaging/winget")].filter((f) => /\bOWNER\b/.test(readFileSync(f, "utf8")));
if (left.length) fail(`placeholder OWNER remains in: ${left.join(", ")}`);
console.log(`check-placeholders: ok (${slug})`);
