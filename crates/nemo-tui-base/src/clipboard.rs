//! Clipboard copy (the `copyToClipboard` / `clipboard.js` part of the Node port).
//!
//! A [`ClipboardSink`] is one way to get text onto the clipboard; [`copy_to_clipboard`] tries
//! sinks in order. [`default_chain`] picks the OS tool first (clip.exe, pbcopy, wl-copy, xclip)
//! and OSC 52 last, which asks the terminal to do it and also works over ssh.
//!
//! Differences from Node: Node only uses clip.exe (Windows) or OSC 52; macOS/Linux tools are
//! new. Node's `copyToClipboard` always returns `true`; here the result says whether any sink
//! accepted the text (OSC 52 on a non-TTY writes nothing and reports failure).

use crate::Platform;
use nemo_config::Env;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub trait ClipboardSink {
    fn copy(&self, text: &str) -> io::Result<()>;
}

/// Best effort, nothing here can fail the app. Returns whether a sink accepted the text.
pub fn copy_to_clipboard(text: &str, sinks: &[&dyn ClipboardSink]) -> bool {
    sinks.iter().any(|s| s.copy(text).is_ok())
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// OSC 52 "set clipboard": `ESC ] 52 ; c ; <base64> BEL`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Writes OSC 52 to a terminal; refuses when `is_tty` is false (output is piped).
pub struct Osc52<W: Write> {
    out: Mutex<W>,
    is_tty: bool,
}

impl<W: Write> Osc52<W> {
    pub fn new(out: W, is_tty: bool) -> Self {
        Self {
            out: Mutex::new(out),
            is_tty,
        }
    }
}

impl Osc52<io::Stdout> {
    pub fn stdout() -> Self {
        Self::new(io::stdout(), io::stdout().is_terminal())
    }
}

impl<W: Write> ClipboardSink for Osc52<W> {
    fn copy(&self, text: &str) -> io::Result<()> {
        if !self.is_tty {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "output is not a terminal",
            ));
        }
        let mut out = self.out.lock().unwrap_or_else(|p| p.into_inner());
        out.write_all(osc52(text).as_bytes())?;
        out.flush()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// What clip.exe reads: UTF-16LE with a BOM (UTF-8 garbles non-ASCII text).
    Utf16LeBom,
}

/// An OS clipboard tool fed on stdin.
#[derive(Clone, Debug)]
pub struct PipeClipboard {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub encoding: Encoding,
}

impl PipeClipboard {
    /// `%SystemRoot%\System32\clip.exe` by absolute path (no PATH lookup).
    pub fn clip_exe() -> Self {
        Self {
            program: nemo_platform::system32("clip.exe"),
            args: vec![],
            encoding: Encoding::Utf16LeBom,
        }
    }

    pub fn pbcopy() -> Self {
        Self {
            program: "/usr/bin/pbcopy".into(),
            args: vec![],
            encoding: Encoding::Utf8,
        }
    }

    pub fn wl_copy() -> Self {
        Self {
            program: "wl-copy".into(),
            args: vec![],
            encoding: Encoding::Utf8,
        }
    }

    pub fn xclip() -> Self {
        Self {
            program: "xclip".into(),
            args: vec!["-selection".into(), "clipboard".into()],
            encoding: Encoding::Utf8,
        }
    }

    pub fn payload(&self, text: &str) -> Vec<u8> {
        match self.encoding {
            Encoding::Utf8 => text.as_bytes().to_vec(),
            Encoding::Utf16LeBom => {
                let mut v = vec![0xff, 0xfe];
                v.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
                v
            }
        }
    }
}

impl ClipboardSink for PipeClipboard {
    /// Spawns the tool, hands it the text and returns without waiting for it (a thread reaps it).
    fn copy(&self, text: &str) -> io::Result<()> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let data = self.payload(text);
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data); // closes stdin on drop
            let _ = child.wait();
        });
        Ok(())
    }
}

/// OS tool(s) for `platform`, then OSC 52 on stdout. Linux uses wl-copy under Wayland
/// (`WAYLAND_DISPLAY`) and xclip under X11 (`DISPLAY`), both read from the injected `env`.
pub fn default_chain(platform: Platform, env: &Env) -> Vec<Box<dyn ClipboardSink>> {
    let mut chain: Vec<Box<dyn ClipboardSink>> = Vec::new();
    match platform {
        Platform::Windows => chain.push(Box::new(PipeClipboard::clip_exe())),
        Platform::MacOs => chain.push(Box::new(PipeClipboard::pbcopy())),
        Platform::Linux => {
            if env.get_nonempty("WAYLAND_DISPLAY").is_some() {
                chain.push(Box::new(PipeClipboard::wl_copy()));
            }
            if env.get_nonempty("DISPLAY").is_some() {
                chain.push(Box::new(PipeClipboard::xclip()));
            }
        }
        Platform::Other => {}
    }
    chain.push(Box::new(Osc52::stdout()));
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn utf16_payload_has_bom() {
        assert_eq!(
            PipeClipboard::clip_exe().payload("aé"),
            [0xff, 0xfe, b'a', 0, 0xe9, 0]
        );
    }

    #[test]
    fn linux_tool_follows_injected_env() {
        let n = |pairs: &[(&str, &str)]| {
            default_chain(Platform::Linux, &Env::from_pairs(pairs.iter().copied())).len()
        };
        assert_eq!(n(&[]), 1);
        assert_eq!(n(&[("WAYLAND_DISPLAY", "wayland-0")]), 2);
        assert_eq!(n(&[("WAYLAND_DISPLAY", "w"), ("DISPLAY", ":0")]), 3);
    }

    #[cfg(windows)]
    #[test]
    fn clip_exe_is_absolute_system32() {
        assert!(
            PipeClipboard::clip_exe()
                .program
                .ends_with("System32\\clip.exe")
        );
    }
}
