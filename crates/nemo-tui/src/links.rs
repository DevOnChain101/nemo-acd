//! ctrl+click on a url or a path in the chat (port of `links.js`). Mouse reporting is on, so the
//! terminal does not open links itself: the click cell is looked up in the text of the last
//! drawn frame and the target is handed to the platform opener.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nemo_config::Env;
use nemo_tui_base::Platform;
use nemo_tui_base::text::{char_width, strip_ansi};

fn forbidden_url(c: char) -> bool {
    c.is_whitespace() || "<>\"'`()[]│┃".contains(c)
}

fn forbidden_path(c: char) -> bool {
    c.is_whitespace() || "<>\"'`|*?│┃".contains(c)
}

/// Terminal cell where every char of `text` starts, plus the total width.
fn cells(chars: &[char]) -> (Vec<usize>, usize) {
    let mut at = Vec::with_capacity(chars.len());
    let mut cell = 0;
    for &c in chars {
        at.push(cell);
        cell += char_width(c);
    }
    (at, cell)
}

fn hit(chars: &[char], from: usize, to: usize, col: usize) -> Option<String> {
    let (at, total) = cells(chars);
    let end = at.get(to).copied().unwrap_or(total);
    (col >= at[from] && col < end).then(|| chars[from..to].iter().collect())
}

/// The http(s) url under terminal cell `col` (0-based) of a drawn line.
pub fn url_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = strip_ansi(line).chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let mut to = i;
            while to < chars.len() && !forbidden_url(chars[to]) {
                to += 1;
            }
            while to > i && ".,;:!?".contains(chars[to - 1]) {
                to -= 1;
            }
            if let Some(u) = hit(&chars, i, to, col) {
                return Some(u);
            }
            i = to.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

fn path_start(c: &[char], i: usize) -> bool {
    let sep = |k: usize| matches!(c.get(k), Some('/' | '\\'));
    let body = |k: usize| c.get(k).is_some_and(|x| !forbidden_path(*x));
    if c[i].is_ascii_alphabetic() && c.get(i + 1) == Some(&':') && sep(i + 2) {
        return true;
    }
    if c[i] == '~' && sep(i + 1) && body(i + 2) {
        return true;
    }
    // "/dir/..." needs two segments so that "and/or" and a lone "/" are not paths
    if c[i] == '/' {
        let mut k = i + 1;
        while body(k) && c[k] != '/' {
            k += 1;
        }
        return k > i + 1 && c.get(k) == Some(&'/');
    }
    false
}

/// The absolute / `~` path under terminal cell `col`. Paths with spaces cannot be told from the
/// words around them and are not matched.
pub fn path_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = strip_ansi(line).chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || chars[i - 1].is_whitespace() || "(\"'`=".contains(chars[i - 1]);
        if boundary && path_start(&chars, i) {
            let mut to = i;
            while to < chars.len() && !forbidden_path(chars[to]) {
                to += 1;
            }
            while to > i && ".,;:!?)]".contains(chars[to - 1]) {
                to -= 1;
            }
            if let Some(p) = hit(&chars, i, to, col) {
                return Some(p);
            }
            i = to.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathTarget {
    pub path: PathBuf,
    pub dir: bool,
}

/// What a path from the screen points at right now. `file.rs:12:3` loses its line numbers, `~`
/// is `home`, relative paths are refused. `None` when it does not exist.
pub fn existing_path(raw: &str, home: Option<&Path>) -> Option<PathTarget> {
    let mut p = raw.to_string();
    if let Some(rest) = p.strip_prefix('~')
        && rest.starts_with(['/', '\\'])
    {
        p = format!("{}{rest}", home?.display());
    }
    // "name.ext:12" / "name.ext:12:3"
    let mut cut = p.len();
    for _ in 0..2 {
        let (head, tail) = p[..cut].rsplit_once(':').unwrap_or((&p[..cut], "x"));
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) && head.contains('.') {
            cut = head.len();
        }
    }
    let path = PathBuf::from(&p[..cut]);
    if !path.is_absolute() {
        return None;
    }
    let meta = std::fs::metadata(&path).ok()?;
    Some(PathTarget {
        path,
        dir: meta.is_dir(),
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Url(String),
    Folder(PathBuf),
    /// Shown selected in its folder; never run or opened with its program.
    RevealFile(PathBuf),
}

impl OpenTarget {
    pub fn from_path(t: &PathTarget) -> Self {
        if t.dir {
            Self::Folder(t.path.clone())
        } else {
            Self::RevealFile(t.path.clone())
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Url(u) => u.clone(),
            Self::Folder(p) | Self::RevealFile(p) => p.display().to_string(),
        }
    }
}

pub trait Opener {
    /// Starts the opener; a failure to start is an error (never swallowed).
    fn open(&self, target: &OpenTarget) -> io::Result<()>;
}

/// Extensions the desktop would run or install when opened.
fn launchable(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase());
    matches!(
        ext.as_deref(),
        Some("app" | "command" | "workflow" | "terminal" | "tool" | "pkg" | "desktop")
    )
}

/// `(program, args)` per platform. No shell is involved, so an address can never run as a
/// command. Windows tools are spawned by absolute path under `SystemRoot`.
pub fn command_for(
    platform: Platform,
    env: &Env,
    target: &OpenTarget,
) -> io::Result<(PathBuf, Vec<String>)> {
    if let OpenTarget::Url(u) = target {
        let ok = ["http://", "https://", "mailto:"]
            .iter()
            .any(|p| u.starts_with(p))
            && !u.chars().any(|c| c.is_whitespace() || c.is_control());
        if !ok {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only http, https and mailto addresses are opened",
            ));
        }
    }
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let root = PathBuf::from(env.get_nonempty("SystemRoot").unwrap_or(r"C:\Windows"));
    Ok(match (platform, target) {
        (Platform::Windows, OpenTarget::Url(u)) => (
            root.join("System32").join("rundll32.exe"),
            vec!["url.dll,FileProtocolHandler".into(), u.clone()],
        ),
        // Never ShellExecute a path: `/select,` only shows it in its folder.
        (Platform::Windows, OpenTarget::Folder(p) | OpenTarget::RevealFile(p)) => {
            (root.join("explorer.exe"), vec![format!("/select,{}", s(p))])
        }
        (Platform::MacOs, OpenTarget::Url(u)) => ("/usr/bin/open".into(), vec![u.clone()]),
        // `open <dir>` would launch a directory named x.app: always reveal.
        (Platform::MacOs, OpenTarget::Folder(p) | OpenTarget::RevealFile(p)) => {
            ("/usr/bin/open".into(), vec!["-R".into(), s(p)])
        }
        (_, OpenTarget::Url(u)) => ("xdg-open".into(), vec![u.clone()]),
        (_, OpenTarget::Folder(p)) if !launchable(p) => ("xdg-open".into(), vec![s(p)]),
        (_, OpenTarget::Folder(p) | OpenTarget::RevealFile(p)) => (
            "xdg-open".into(),
            vec![s(p.parent().unwrap_or(Path::new("/")))],
        ),
    })
}

pub struct SystemOpener {
    pub platform: Platform,
    pub env: Env,
}

impl Opener for SystemOpener {
    fn open(&self, target: &OpenTarget) -> io::Result<()> {
        let (program, args) = command_for(self.platform, &self.env, target)?;
        spawn_detached(&program, &args)
    }
}

/// Spawn errors carry the program path. The child is reaped on a thread so it does not stay a
/// zombie; its exit status is not tracked (explorer.exe exits 1 on success).
pub fn spawn_detached(program: &Path, args: &[String]) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", program.display())))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_url_under_cell_and_trims_punctuation() {
        let l = "see https://example.com/a?b=1. now";
        assert_eq!(url_at(l, 4).as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(
            url_at(l, 29).as_deref(),
            None,
            "the full stop is not the link"
        );
        assert_eq!(url_at(l, 0), None);
        // wide characters before the link shift cells, not chars
        assert_eq!(url_at("日本 http://a.b", 6).as_deref(), Some("http://a.b"));
        assert_eq!(
            url_at("\x1b[31mhttp://x.y\x1b[0m", 1).as_deref(),
            Some("http://x.y")
        );
    }

    #[test]
    fn finds_paths() {
        assert_eq!(
            path_at("wrote /tmp/a/b.txt:12 ok", 8).as_deref(),
            Some("/tmp/a/b.txt:12")
        );
        assert_eq!(path_at("and/or", 1), None);
        assert_eq!(path_at("at (~/proj/x)", 5).as_deref(), Some("~/proj/x"));
        assert_eq!(
            path_at(r"open C:\Users\me\f.txt now", 8).as_deref(),
            Some(r"C:\Users\me\f.txt")
        );
        assert_eq!(path_at("/only", 1), None);
    }

    #[test]
    fn existing_path_rules() {
        let td = tempfile::tempdir().unwrap();
        let d = td.path().to_path_buf();
        let f = d.join("probe.rs");
        std::fs::write(&f, "x").unwrap();
        let t = existing_path(&format!("{}:12:3", f.display()), None).unwrap();
        assert!(!t.dir && t.path == f);
        assert!(existing_path(&d.display().to_string(), None).unwrap().dir);
        assert_eq!(existing_path("relative/p.rs", None), None);
        assert_eq!(existing_path("~/x/y", None), None, "no home known");
    }

    #[test]
    fn opener_commands_per_platform() {
        let env = Env::from_pairs([("SystemRoot", r"D:\Win")]);
        let u = OpenTarget::Url("https://x.y/z".into());
        let (p, a) = command_for(Platform::Windows, &env, &u).unwrap();
        assert_eq!(
            p,
            PathBuf::from(r"D:\Win")
                .join("System32")
                .join("rundll32.exe")
        );
        assert_eq!(a, ["url.dll,FileProtocolHandler", "https://x.y/z"]);
        let f = OpenTarget::RevealFile("/a/b.txt".into());
        assert_eq!(
            command_for(Platform::MacOs, &env, &f).unwrap().1,
            ["-R", "/a/b.txt"]
        );
        assert_eq!(command_for(Platform::Linux, &env, &f).unwrap().1, ["/a"]);
        let (p, a) = command_for(Platform::Windows, &env, &f).unwrap();
        assert_eq!(p, PathBuf::from(r"D:\Win").join("explorer.exe"));
        assert_eq!(a, ["/select,/a/b.txt"]);
        for bad in [
            "file:///etc/passwd",
            "javascript:x",
            "https://a b",
            "https://a\x1b]52;c;x",
        ] {
            let bad = OpenTarget::Url(bad.into());
            assert!(command_for(Platform::Linux, &env, &bad).is_err());
        }
        let m = OpenTarget::Url("mailto:a@b.c".into());
        assert!(command_for(Platform::MacOs, &env, &m).is_ok());
    }

    #[test]
    fn paths_are_revealed_never_launched() {
        let env = Env::from_pairs([("SystemRoot", r"D:\Win")]);
        for name in [
            "/a/x.app",
            "/a/y.COMMAND",
            "/a/d",
            "/a/z.pkg",
            "/a/w.workflow",
        ] {
            let d = OpenTarget::Folder(name.into());
            let (p, a) = command_for(Platform::MacOs, &env, &d).unwrap();
            assert_eq!(
                (p, a),
                (
                    "/usr/bin/open".into(),
                    vec!["-R".to_string(), name.to_string()]
                )
            );
            let (_, a) = command_for(Platform::Windows, &env, &d).unwrap();
            assert_eq!(a, [format!("/select,{name}")]);
        }
        let app = OpenTarget::Folder("/a/x.app".into());
        assert_eq!(command_for(Platform::Linux, &env, &app).unwrap().1, ["/a"]);
        let t = OpenTarget::Folder("/a/t.Desktop".into());
        assert_eq!(command_for(Platform::Linux, &env, &t).unwrap().1, ["/a"]);
        let plain = OpenTarget::Folder("/a/d".into());
        assert_eq!(
            command_for(Platform::Linux, &env, &plain).unwrap().1,
            ["/a/d"]
        );
    }

    #[test]
    fn spawn_failure_is_reported() {
        let e = spawn_detached(Path::new("/nonexistent/nemo-opener"), &[]).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(e.to_string().contains("nemo-opener"), "{e}");
    }
}
