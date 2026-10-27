//! Minimal Server-Sent Events parser (WHATWG rules). Hand-rolled instead of eventsource-stream:
//! ~60 lines, works on raw bytes so a multi-byte UTF-8 sequence split across network chunks is
//! only decoded once its line is complete, and no extra dependency or its MSRV/maintenance risk.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data: String,
    has_data: bool,
    event: Option<String>,
    started: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one network chunk; returns the events completed by it. An event still open at end of
    /// stream (no blank line) is dropped, as the spec requires.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.buf);
        let (mut out, mut start, mut i) = (Vec::new(), 0, 0);
        while i < buf.len() {
            let skip = match buf[i] {
                b'\n' => 1,
                // A trailing lone CR may be the first half of CRLF: wait for more bytes.
                b'\r' if i + 1 >= buf.len() => break,
                b'\r' => 1 + usize::from(buf[i + 1] == b'\n'),
                _ => {
                    i += 1;
                    continue;
                }
            };
            self.line(&buf[start..i], &mut out);
            i += skip;
            start = i;
        }
        self.buf = buf[start..].to_vec();
        out
    }

    fn line(&mut self, raw: &[u8], out: &mut Vec<SseEvent>) {
        let mut line = String::from_utf8_lossy(raw).into_owned();
        if !self.started {
            self.started = true;
            if let Some(rest) = line.strip_prefix('\u{feff}') {
                line = rest.to_string();
            }
        }
        if line.is_empty() {
            if self.has_data {
                out.push(SseEvent {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data),
                });
                self.has_data = false;
            }
            self.event = None;
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = line.split_once(':').unwrap_or((&line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_string()),
            _ => {} // id / retry unused
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(data: &str) -> SseEvent {
        SseEvent {
            event: None,
            data: data.into(),
        }
    }

    #[test]
    fn split_everywhere_gives_same_events() {
        let src =
            "event: x\r\ndata: a\ndata: b\n\n: comment\ndata:héllo\n\r\ndata: z\r\r\n".as_bytes();
        let whole = SseParser::new().push(src);
        assert_eq!(
            whole,
            vec![
                SseEvent {
                    event: Some("x".into()),
                    data: "a\nb".into()
                },
                ev("héllo"),
                ev("z"),
            ]
        );
        for cut in 0..src.len() {
            let mut p = SseParser::new();
            let mut got = p.push(&src[..cut]);
            got.extend(p.push(&src[cut..]));
            assert_eq!(got, whole, "cut at {cut}");
        }
    }

    #[test]
    fn unterminated_event_is_dropped() {
        assert!(SseParser::new().push(b"data: partial\n").is_empty());
    }
}
