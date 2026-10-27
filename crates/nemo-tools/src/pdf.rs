//! Text of a PDF for Read (port of the built-in fallback of pdf.js; the `pdftotext` spawn is not
//! ported: no external program runs on an untrusted file).
//! shortcut: only literal strings (Tj / TJ) in plain or Flate streams are read, so PDFs with
//! embedded CID fonts, scans or encryption come out empty and page ranges are ignored (the whole
//! file is shown). Upgrade to a vetted pure-Rust parser with decompression limits when such files
//! matter (lopdf 0.46 was considered: its default features pull rayon+chrono and I did not verify
//! its limits on hostile streams). Decoding is bounded: streams, bytes per stream, total text.

use std::io::Read;
use std::sync::LazyLock;

use flate2::read::ZlibDecoder;
use regex::Regex;

const MAX_STREAMS: usize = 2000;
const MAX_INFLATED: u64 = 8 * 1024 * 1024;
const MAX_TEXT: usize = 2 * 1024 * 1024;

static PAGES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+)(?:-(\d+))?$").unwrap());
static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"BT[\s\S]*?ET").unwrap());
static OPS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\[((?:\\.|[^\]\\])*)\]\s*TJ|\(((?:\\.|[^()\\])*)\)\s*(?:Tj|'|")|(T\*|Td|TD|Tm)\b"#,
    )
    .unwrap()
});
static PARTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(((?:\\.|[^()\\])*)\)|(-?\d+(?:\.\d+)?)").unwrap());
static ESC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\([nrtbf()\\]|[0-7]{1,3}|\r?\n)").unwrap());
static SKIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/Subtype\s*/Image|/FontFile|/Type\s*/XRef|/ObjStm").unwrap());

fn unescape(s: &str) -> String {
    ESC.replace_all(s, |c: &regex::Captures| {
        match &c[1] {
            "n" => "\n".to_string(),
            "r" => "\r".to_string(),
            "t" => "\t".to_string(),
            "b" | "f" => String::new(),
            "(" | ")" | "\\" => c[1].to_string(),
            o if o.starts_with(|d: char| d.is_ascii_digit()) => u32::from_str_radix(o, 8)
                .ok()
                .and_then(char::from_u32)
                .map_or(String::new(), String::from),
            _ => String::new(), // escaped line break
        }
    })
    .into_owned()
}

/// Text-showing operators of one decoded content stream, one line per Td/T*/ET.
fn text_of_content(content: &str) -> String {
    let mut out: Vec<String> = vec![];
    for block in BLOCK.find_iter(content) {
        let mut line = String::new();
        for m in OPS.captures_iter(block.as_str()) {
            if let Some(arr) = m.get(1) {
                for p in PARTS.captures_iter(arr.as_str()) {
                    if let Some(s) = p.get(1) {
                        line += &unescape(s.as_str());
                    } else if p[2].parse::<f64>().is_ok_and(|n| n < -200.0) {
                        line.push(' '); // a wide kerning gap is a space
                    }
                }
            } else if let Some(s) = m.get(2) {
                line += &unescape(s.as_str());
            } else if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out.join("\n")
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn extract_text(buf: &[u8]) -> String {
    let mut texts = vec![];
    let (mut pos, mut streams, mut total) = (0, 0, 0usize);
    while let Some(at) = find(buf, b"stream", pos) {
        pos = at + 6;
        if at >= 3 && &buf[at - 3..at] == b"end" {
            continue;
        }
        // bounded look-back: an unbounded scan per `stream` token is quadratic on a crafted file
        let win = at.saturating_sub(4096);
        let dict_start = buf[win..at]
            .windows(2)
            .rposition(|w| w == b"<<")
            .map_or(win, |i| win + i);
        let dict = latin1(&buf[dict_start..at]);
        let mut start = at + 6;
        if buf.get(start) == Some(&b'\r') {
            start += 1;
        }
        if buf.get(start) == Some(&b'\n') {
            start += 1;
        }
        let Some(end) = find(buf, b"endstream", start) else {
            break;
        };
        pos = end + 9;
        if SKIP.is_match(&dict) {
            continue;
        }
        streams += 1;
        if streams > MAX_STREAMS {
            break;
        }
        let raw = &buf[start..end];
        let body = if dict.contains("/FlateDecode") {
            let mut out = vec![];
            if ZlibDecoder::new(raw)
                .take(MAX_INFLATED)
                .read_to_end(&mut out)
                .is_err()
                && out.is_empty()
            {
                continue;
            }
            out
        } else {
            raw.to_vec()
        };
        let text = text_of_content(&latin1(&body)).replace('\0', "");
        if !text.trim().is_empty() {
            total += text.len();
            texts.push(text);
            if total > MAX_TEXT {
                break;
            }
        }
    }
    texts.join("\n\n")
}

/// Text of the PDF, or the error shown to the model. `pages` is validated like Node but cannot
/// be honoured.
pub(crate) fn extract(buf: &[u8], pages: Option<&str>) -> Result<String, String> {
    let mut note = "";
    if let Some(p) = pages.map(str::trim).filter(|p| !p.is_empty()) {
        let Some(m) = PAGES.captures(p) else {
            return Err(format!(
                "\"pages\" must look like \"3\" or \"2-5\", got \"{p}\"."
            ));
        };
        let first: u64 = m[1].parse().unwrap_or(0);
        let last: u64 = m.get(2).map_or(first, |l| l.as_str().parse().unwrap_or(0));
        if first < 1 || last < first {
            return Err(format!("\"pages\" {p} is not a valid range."));
        }
        note = " Page ranges are not supported by the built-in reader, so the whole file is shown.";
    }
    let text = extract_text(buf);
    if text.trim().is_empty() {
        return Err("no text could be extracted with the built-in PDF reader (scanned, font-encoded or encrypted PDFs are not supported).".into());
    }
    Ok(format!(
        "[text from the built-in PDF reader, which can miss text.{note}]\n\n{text}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    const PAGE: &str =
        "BT /F1 12 Tf 72 700 Td (Hello \\(PDF\\) world) Tj 0 -14 Td [(Second) -300 (line)] TJ ET";

    fn pdf(content: &str, flate: bool) -> Vec<u8> {
        let body = if flate {
            let mut e = ZlibEncoder::new(vec![], Compression::default());
            e.write_all(content.as_bytes()).unwrap();
            e.finish().unwrap()
        } else {
            content.as_bytes().to_vec()
        };
        let dict = format!(
            "<< /Length {}{} >>",
            body.len(),
            if flate { " /Filter /FlateDecode" } else { "" }
        );
        let mut v = format!("%PDF-1.4\n1 0 obj\n{dict}\nstream\n").into_bytes();
        v.extend(body);
        v.extend(b"\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n");
        v
    }

    #[test]
    fn reads_plain_and_flate_streams() {
        for flate in [false, true] {
            let t = extract(&pdf(PAGE, flate), None).unwrap();
            assert!(t.contains("Hello (PDF) world"), "{t}");
            assert!(t.contains("Second line"), "wide TJ gap is a space: {t}");
        }
    }

    #[test]
    fn page_ranges_validated_and_noted() {
        assert!(
            extract(&[], Some("abc"))
                .unwrap_err()
                .contains("must look like")
        );
        assert!(
            extract(&[], Some("5-2"))
                .unwrap_err()
                .contains("not a valid range")
        );
        assert!(
            extract(&[], Some("0"))
                .unwrap_err()
                .contains("not a valid range")
        );
        assert!(
            extract(&pdf(PAGE, false), Some("2-3"))
                .unwrap()
                .contains("whole file is shown")
        );
    }

    #[test]
    fn no_text_and_inflate_bomb_are_bounded() {
        assert!(
            extract(&pdf("q Q", false), None)
                .unwrap_err()
                .contains("no text")
        );
        assert!(extract(b"", None).is_err());
        let mut bomb = String::from("BT (");
        bomb.push_str(&"a".repeat(20 * 1024 * 1024));
        bomb.push_str(") Tj ET");
        let t = extract(&pdf(&bomb, true), None);
        // the stream is cut at the inflate cap: the unterminated string yields no text, no OOM
        assert!(t.is_err() || t.unwrap().len() < 9 * 1024 * 1024);
    }

    #[test]
    fn dict_scan_is_bounded_per_stream_token() {
        // 16 MB with no `<<`, then 2000 tiny streams: an unbounded look-back scans ~32 GB
        let mut f = vec![b'a'; 16 << 20];
        for _ in 0..3000 {
            f.extend(b"\nstream\nendstream\n");
        }
        let t = std::time::Instant::now();
        let _ = extract_text(&f);
        assert!(t.elapsed().as_secs() < 3, "{:?}", t.elapsed());
    }
}
