//! Bounded, ephemeral console records. Raw records never leave pipe workers.
use serde_json::Value;
use std::sync::{Arc, Mutex};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const RECORD_LIMIT: usize = 65536;
pub(crate) type Queue = Arc<Mutex<Tail>>;

#[derive(Default)]
pub(crate) struct Tail {
    pub lines: Vec<String>,
    pub truncated: bool,
}
impl Tail {
    pub fn push(&mut self, mut line: String) {
        if line.len() > 1024 {
            let mut end = 1024;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            line.truncate(end);
            self.truncated = true;
        }
        if line.is_empty() {
            return;
        }
        self.lines.push(line);
        while self.lines.len() > 200 || self.lines.iter().map(String::len).sum::<usize>() > 65536 {
            self.lines.remove(0);
            self.truncated = true;
        }
    }
}

#[derive(Clone)]
pub(crate) struct Sink {
    pub queue: Queue,
    pub password: Arc<Zeroizing<String>>,
}
impl Sink {
    pub fn record(&self, raw: &[u8]) {
        let decoded = Zeroizing::new(String::from_utf8_lossy(raw).into_owned());
        // A control sequence can consume part of a literal password. Remove
        // literal credentials first so normalization cannot expose a suffix.
        let literal_safe = Zeroizing::new(decoded.replace(self.password.as_str(), "[redacted]"));
        // Terminal wrappers must not hide JSON from the decoder: escaped JSON
        // strings may contain a credential that literal replacement cannot see.
        let normalized = Zeroizing::new(strip_controls(&literal_safe));
        let mut value = serde_json::from_str::<Value>(&normalized).ok();
        if value.is_none() && normalized.contains("\\u") {
            // Do not forward recoverable Unicode-escaped data when malformed
            // JSON prevented decoding. Ordinary text diagnostics remain useful.
            let mut queue = self.queue.lock().unwrap();
            queue.truncated = true;
            queue.push("Diagnostic record omitted: invalid escaped data".into());
            return;
        }
        let text = Zeroizing::new(match value.as_ref() {
            Some(value) => format_record(value),
            None => normalized.to_string(),
        });
        // Decode JSON strings first, then redact BEFORE shortening or stripping
        // controls (and again afterwards, in case controls separated a secret).
        let redacted = Zeroizing::new(text.replace(self.password.as_str(), "[redacted]"));
        let clean = Zeroizing::new(strip_controls(&redacted));
        let safe = clean.replace(self.password.as_str(), "[redacted]");
        if let Some(value) = value.as_mut() {
            wipe(value);
        }
        self.queue.lock().unwrap().push(safe);
    }
    pub fn dropped(&self) {
        self.queue.lock().unwrap().truncated = true;
    }
}

fn format_record(value: &Value) -> String {
    match value["message_type"].as_str() {
        Some("status") => {
            let files = Zeroizing::new(
                value["current_files"]
                    .as_array()
                    .map(|files| {
                        files
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default(),
            );
            format!(
                "Status: {} files, {} bytes; {}",
                value["files_done"]
                    .as_u64()
                    .or(value["files_restored"].as_u64())
                    .unwrap_or(0),
                value["bytes_done"]
                    .as_u64()
                    .or(value["bytes_restored"].as_u64())
                    .unwrap_or(0),
                files.as_str()
            )
        }
        Some("summary") => format!(
            "Summary: {} files, {} bytes; snapshot {}",
            value["total_files_processed"]
                .as_u64()
                .or(value["files_restored"].as_u64())
                .unwrap_or(0),
            value["total_bytes_processed"]
                .as_u64()
                .or(value["bytes_restored"].as_u64())
                .unwrap_or(0),
            value["snapshot_id"].as_str().unwrap_or("")
        ),
        Some("verbose_status") => format!(
            "{}: {}",
            value["action"].as_str().unwrap_or("File"),
            value["item"].as_str().unwrap_or("")
        ),
        _ => {
            // Diagnostics differ by restic version. Render decoded string values,
            // not escaped JSON (which could hide an encoded credential).
            fn strings(value: &Value, out: &mut String) {
                match value {
                    Value::String(s) => {
                        out.push_str(s);
                        out.push(' ');
                    }
                    Value::Array(a) => {
                        for v in a {
                            strings(v, out);
                        }
                    }
                    Value::Object(o) => {
                        for v in o.values() {
                            strings(v, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut out = String::new();
            strings(value, &mut out);
            out
        }
    }
}
pub(super) fn wipe(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(a) => {
            for v in a {
                wipe(v);
            }
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                wipe(v);
            }
        }
        _ => {}
    }
}
fn strip_controls(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.peek() == Some(&'\\')) {
                            if c == '\u{1b}' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out
}

pub(crate) struct Records {
    bytes: Zeroizing<Vec<u8>>,
    oversized: bool,
    sink: Sink,
}
impl Records {
    pub fn new(sink: Sink) -> Self {
        Self {
            bytes: Zeroizing::new(Vec::new()),
            oversized: false,
            sink,
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.finish();
            } else if !self.oversized {
                if self.bytes.len() == RECORD_LIMIT {
                    self.bytes.zeroize();
                    self.bytes.clear();
                    self.oversized = true;
                    self.sink.dropped();
                } else {
                    self.bytes.push(byte);
                }
            }
        }
    }
    pub fn finish(&mut self) {
        if !self.oversized && !self.bytes.is_empty() {
            self.sink.record(&self.bytes);
        }
        self.bytes.zeroize();
        self.bytes.clear();
        self.oversized = false;
    }
}
