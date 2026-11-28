//! Typed clipboard port for Curie Clinic Desktop with OS clipboard integration
//! and an in-process bounded ring-buffer fallback (zero extra crates).

use std::collections::VecDeque;

use bevy::prelude::Resource;
use thiserror::Error;

/// Default maximum number of entries retained in the in-process clipboard ring buffer.
pub const DEFAULT_CLIPBOARD_HISTORY_LIMIT: usize = 16;

/// Typed clipboard error preserving failure context.
#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("clipboard helper '{tool}' failed: {source}")]
    Io {
        tool: &'static str,
        #[source]
        source: std::io::Error,
    },
}

/// Single-owner typed clipboard contract used by keyboard shortcuts and buttons.
pub trait ClipboardPort: Send + Sync {
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
    fn read_text(&mut self) -> Result<String, ClipboardError>;
    fn history(&self) -> Vec<String>;
}

/// Deterministic in-process bounded ring buffer clipboard (used in headless smoke & tests,
/// and as the resilient backing mirror for [`SystemClipboardAdapter`]).
#[derive(Debug, Clone)]
pub struct InMemoryClipboard {
    entries: VecDeque<String>,
    capacity: usize,
}

impl Default for InMemoryClipboard {
    fn default() -> Self {
        Self::new(DEFAULT_CLIPBOARD_HISTORY_LIMIT)
    }
}

impl InMemoryClipboard {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    #[must_use]
    pub fn latest(&self) -> Option<&str> {
        self.entries.back().map(String::as_str)
    }

    pub fn push(&mut self, text: String) {
        if self.entries.len() >= self.capacity {
            let _ = self.entries.pop_front();
        }
        self.entries.push_back(text);
    }
}

impl ClipboardPort for InMemoryClipboard {
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.push(text.to_owned());
        Ok(())
    }

    fn read_text(&mut self) -> Result<String, ClipboardError> {
        self.latest()
            .map(str::to_owned)
            .ok_or_else(|| ClipboardError::Unavailable {
                reason: "clipboard buffer is empty".to_owned(),
            })
    }

    fn history(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }
}

/// Native desktop clipboard adapter that mirrors into an in-process ring buffer and
/// synchronizes with the host OS clipboard tool when available on Linux/macOS/Windows.
#[derive(Debug, Default)]
pub struct SystemClipboardAdapter {
    fallback: InMemoryClipboard,
}

impl SystemClipboardAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self {
            fallback: InMemoryClipboard::default(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn try_os_write(text: &str) -> bool {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let candidates: &[(&str, &[&str])] = {
            #[cfg(target_os = "macos")]
            {
                &[("pbcopy", &[])]
            }
            #[cfg(target_os = "windows")]
            {
                &[("clip", &[])]
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                &[
                    ("wl-copy", &[]),
                    ("xclip", &["-selection", "clipboard"]),
                    ("xsel", &["--clipboard", "--input"]),
                ]
            }
        };

        for (bin, args) in candidates {
            let Ok(mut child) = Command::new(bin)
                .args(*args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|status| status.success()) {
                return true;
            }
        }
        false
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn try_os_read() -> Option<String> {
        use std::process::Command;

        let candidates: &[(&str, &[&str])] = {
            #[cfg(target_os = "macos")]
            {
                &[("pbpaste", &[])]
            }
            #[cfg(target_os = "windows")]
            {
                &[(
                    "powershell",
                    &["-NoProfile", "-Command", "Get-Clipboard -Raw"],
                )]
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                &[
                    ("wl-paste", &["--no-newline"]),
                    ("xclip", &["-selection", "clipboard", "-o"]),
                    ("xsel", &["--clipboard", "--output"]),
                ]
            }
        };

        for (bin, args) in candidates {
            let Ok(output) = Command::new(bin).args(*args).output() else {
                continue;
            };
            if output.status.success()
                && let Ok(text) = String::from_utf8(output.stdout)
                && !text.is_empty()
            {
                return Some(text);
            }
        }
        None
    }
}

impl ClipboardPort for SystemClipboardAdapter {
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.fallback.write_text(text)?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = Self::try_os_write(text);
        }
        Ok(())
    }

    fn read_text(&mut self) -> Result<String, ClipboardError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(os_text) = Self::try_os_read() {
                return Ok(os_text);
            }
        }
        self.fallback.read_text()
    }

    fn history(&self) -> Vec<String> {
        self.fallback.history()
    }
}

/// Bevy [`Resource`] wrapping the active [`ClipboardPort`] owner.
#[derive(Resource)]
pub struct DesktopClipboard {
    port: Box<dyn ClipboardPort>,
}

impl Default for DesktopClipboard {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl DesktopClipboard {
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            port: Box::new(InMemoryClipboard::default()),
        }
    }

    #[must_use]
    pub fn system() -> Self {
        Self {
            port: Box::new(SystemClipboardAdapter::new()),
        }
    }

    #[must_use]
    pub fn with_port(port: impl ClipboardPort + 'static) -> Self {
        Self {
            port: Box::new(port),
        }
    }

    pub fn copy_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.port.write_text(text)
    }

    pub fn paste_text(&mut self) -> Result<String, ClipboardError> {
        self.port.read_text()
    }

    #[must_use]
    pub fn history(&self) -> Vec<String> {
        self.port.history()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_clipboard_ring_buffer_caps_capacity_and_roundtrips() {
        let mut clip = InMemoryClipboard::new(2);
        assert!(clip.read_text().is_err());

        clip.write_text("first").expect("write 1");
        clip.write_text("second").expect("write 2");
        clip.write_text("third").expect("write 3");

        assert_eq!(clip.read_text().expect("read latest"), "third");
        assert_eq!(
            clip.history(),
            vec!["second".to_owned(), "third".to_owned()]
        );
    }
}
