//! Stable workspace identity (docs/specs/messaging.md §1, memory.md §4): the same repository
//! gets the same key in every session and process, unlike the per-session `workspace_id`.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{PathPolicy, canonicalize};

/// First 24 hex (96 bits) of SHA-256 over the folded realpath of `root` (`\` to `/` on Windows,
/// case-folded on Windows and macOS). Callers pass an existing directory; a missing one is hashed
/// as given. It is a NAME for grouping sessions, derived from a guessable path: never an
/// authentication token or a capability.
pub fn workspace_key(root: &Path, policy: PathPolicy) -> String {
    let real = canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut s = real.to_string_lossy().into_owned();
    if policy.windows_separators {
        s = s.replace('\\', "/");
    }
    if policy.case_insensitive {
        s = s.to_lowercase();
    }
    let d = Sha256::digest(s.as_bytes());
    d[..12].iter().map(|b| format!("{b:02x}")).collect()
}

/// Basename of `root` mapped to `[a-z0-9_-]` (`My-App` becomes `my-app`). Not unique.
pub fn workspace_label(root: &Path) -> String {
    let name = root
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().to_lowercase());
    let l: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if l.is_empty() { "workspace".into() } else { l }
}

/// Key of the repository that owns `root`: all git worktrees of one repository share it. It is the
/// [`workspace_key`] of the main worktree, found as `git rev-parse --git-common-dir` does (a `.git`
/// file points at `<main>/.git/worktrees/<id>`, whose `commondir` names `<main>/.git`) but without
/// spawning git. Outside git, or when the pointers are unreadable or do not agree both ways
/// (`.git` is a symlink, is over 4 KiB, does not name `<main>/.git/worktrees/<id>`, or that
/// directory's `gitdir` does not point back at `<root>/.git`), it equals `workspace_key(root)`.
/// A repository can still forge a consistent pair inside itself, so this key must not gate access
/// to anything until it is hardened further; it only groups sessions.
pub fn repo_key(root: &Path, policy: PathPolicy) -> String {
    let main = common_dir(root).map_or_else(
        || root.to_path_buf(),
        |c| {
            if c.file_name().is_some_and(|n| n == ".git") {
                c.parent().map_or(c.clone(), Path::to_path_buf)
            } else {
                c // bare repository
            }
        },
    );
    workspace_key(&main, policy)
}

/// At most 4 KiB of text; longer or unreadable is None.
fn read_small(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = String::new();
    fs::File::open(p)
        .ok()?
        .take(4097)
        .read_to_string(&mut buf)
        .ok()?;
    (buf.len() <= 4096).then_some(buf)
}

fn common_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let meta = fs::symlink_metadata(&dot_git).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    if meta.is_dir() {
        return canonicalize(dot_git).ok();
    }
    let text = read_small(&dot_git)?;
    let gitdir = root.join(text.lines().next()?.strip_prefix("gitdir:")?.trim());
    let gitdir = canonicalize(gitdir).ok()?;
    // <main>/.git/worktrees/<id>
    let common = gitdir
        .parent()
        .filter(|w| w.file_name().is_some_and(|n| n == "worktrees"))?;
    let common = common
        .parent()
        .filter(|g| g.file_name().is_some_and(|n| n == ".git"))?;
    // the worktree admin dir must point back at this checkout
    let back = canonicalize(read_small(&gitdir.join("gitdir"))?.trim()).ok()?;
    (back == canonicalize(&dot_git).ok()?).then_some(())?;
    let rel = read_small(&gitdir.join("commondir"))?;
    (canonicalize(gitdir.join(rel.trim())).ok()? == common).then(|| common.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSIX: PathPolicy = PathPolicy {
        case_insensitive: false,
        windows_separators: false,
    };
    const FOLD: PathPolicy = PathPolicy {
        case_insensitive: true,
        windows_separators: false,
    };

    #[test]
    fn key_is_24_hex_stable_and_follows_symlink_free_realpath() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("A");
        fs::create_dir(&a).unwrap();
        let k = workspace_key(&a, POSIX);
        assert!(
            k.len() == 24 && k.bytes().all(|b| b.is_ascii_hexdigit()),
            "{k}"
        );
        assert_eq!(k, workspace_key(&a.join("."), POSIX));
        let b = d.path().join("B");
        fs::create_dir(&b).unwrap();
        assert_ne!(k, workspace_key(&b, POSIX));
    }

    #[test]
    fn folding_policy_decides_whether_case_matters() {
        let d = tempfile::tempdir().unwrap();
        let (up, low) = (d.path().join("Repo"), d.path().join("repo"));
        // keys of paths differing only by case, without needing a case-insensitive disk
        assert_eq!(workspace_key(&up, FOLD), workspace_key(&low, FOLD));
        assert_ne!(workspace_key(&up, POSIX), workspace_key(&low, POSIX));
    }

    #[test]
    fn label_is_a_safe_basename() {
        assert_eq!(workspace_label(Path::new("/x/My-App")), "my-app");
        assert_eq!(workspace_label(Path::new("/x/My Repo.v2")), "my-repo-v2");
        assert_eq!(workspace_label(Path::new("/")), "workspace");
    }

    #[test]
    fn worktrees_share_the_repo_key_but_not_the_workspace_key() {
        let d = tempfile::tempdir().unwrap();
        let base = canonicalize(d.path()).unwrap();
        let (main, wt) = (base.join("main"), base.join("wt"));
        let gitdir = main.join(".git/worktrees/wt");
        fs::create_dir_all(&gitdir).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        fs::write(
            gitdir.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        assert_ne!(workspace_key(&main, POSIX), workspace_key(&wt, POSIX));
        assert_eq!(repo_key(&wt, POSIX), workspace_key(&main, POSIX));
        assert_eq!(repo_key(&main, POSIX), workspace_key(&main, POSIX));
        // not a repository: the repo key is the workspace key
        let plain = base.join("plain");
        fs::create_dir(&plain).unwrap();
        assert_eq!(repo_key(&plain, POSIX), workspace_key(&plain, POSIX));
    }

    /// A checkout whose `.git` claims membership of `main` must be rejected unless `main` agrees.
    #[test]
    fn hostile_git_pointers_fall_back_to_the_workspace_key() {
        let d = tempfile::tempdir().unwrap();
        let base = canonicalize(d.path()).unwrap();
        let (main, wt, evil) = (base.join("main"), base.join("wt"), base.join("evil"));
        let gitdir = main.join(".git/worktrees/other");
        fs::create_dir_all(&gitdir).unwrap();
        fs::create_dir_all(&wt).unwrap();
        fs::create_dir_all(&evil).unwrap();
        fs::write(gitdir.join("commondir"), "../..\n").unwrap();
        // main's admin dir points back at some other checkout, not at wt
        fs::write(
            gitdir.join("gitdir"),
            format!("{}\n", base.join("else/.git").display()),
        )
        .unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(repo_key(&wt, POSIX), workspace_key(&wt, POSIX));
        // gitdir not of the form <main>/.git/worktrees/<id>
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git").display()),
        )
        .unwrap();
        assert_eq!(repo_key(&wt, POSIX), workspace_key(&wt, POSIX));
        // oversized .git file
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n{}", gitdir.display(), "x".repeat(5000)),
        )
        .unwrap();
        assert_eq!(repo_key(&wt, POSIX), workspace_key(&wt, POSIX));
        // .git symlink to another repository's .git
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(main.join(".git"), evil.join(".git")).unwrap();
            assert_eq!(repo_key(&evil, POSIX), workspace_key(&evil, POSIX));
        }
    }
}
