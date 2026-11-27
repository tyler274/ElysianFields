use std::fmt;

use bevy::prelude::Resource;

/// Live-region politeness matching `accesskit::Live` and ARIA `aria-live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnouncementPriority {
    Polite,
    Assertive,
}

impl AnnouncementPriority {
    #[must_use]
    pub const fn to_accesskit(self) -> accesskit::Live {
        match self {
            Self::Polite => accesskit::Live::Polite,
            Self::Assertive => accesskit::Live::Assertive,
        }
    }
}

/// Bounded coalescing window for streaming polite announcements.
///
/// Prevents high-rate token streams (e.g., 40 tokens/sec) from flooding
/// VoiceOver, TalkBack, NVDA, or Orca with per-token interruptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoalescingPolicy {
    min_interval_ms: u64,
    max_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoalescingPolicyError {
    MinTooShort {
        min_interval_ms: u64,
        floor_ms: u64,
    },
    MaxTooLong {
        max_interval_ms: u64,
        ceiling_ms: u64,
    },
    InvertedWindow {
        min_interval_ms: u64,
        max_interval_ms: u64,
    },
}

impl fmt::Display for CoalescingPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MinTooShort {
                min_interval_ms,
                floor_ms,
            } => write!(
                f,
                "min_interval_ms ({min_interval_ms}ms) must be at least {floor_ms}ms to avoid screen-reader flooding"
            ),
            Self::MaxTooLong {
                max_interval_ms,
                ceiling_ms,
            } => write!(
                f,
                "max_interval_ms ({max_interval_ms}ms) must not exceed {ceiling_ms}ms so streaming updates stay timely"
            ),
            Self::InvertedWindow {
                min_interval_ms,
                max_interval_ms,
            } => write!(
                f,
                "min_interval_ms ({min_interval_ms}ms) must be <= max_interval_ms ({max_interval_ms}ms)"
            ),
        }
    }
}

impl std::error::Error for CoalescingPolicyError {}

impl CoalescingPolicy {
    pub const MIN_FLOOR_MS: u64 = 750;
    pub const MAX_CEILING_MS: u64 = 1500;

    /// Constructs a validated coalescing policy within the 750ms..=1500ms budget.
    pub const fn new(
        min_interval_ms: u64,
        max_interval_ms: u64,
    ) -> Result<Self, CoalescingPolicyError> {
        if min_interval_ms < Self::MIN_FLOOR_MS {
            return Err(CoalescingPolicyError::MinTooShort {
                min_interval_ms,
                floor_ms: Self::MIN_FLOOR_MS,
            });
        }
        if max_interval_ms > Self::MAX_CEILING_MS {
            return Err(CoalescingPolicyError::MaxTooLong {
                max_interval_ms,
                ceiling_ms: Self::MAX_CEILING_MS,
            });
        }
        if min_interval_ms > max_interval_ms {
            return Err(CoalescingPolicyError::InvertedWindow {
                min_interval_ms,
                max_interval_ms,
            });
        }
        Ok(Self {
            min_interval_ms,
            max_interval_ms,
        })
    }

    #[must_use]
    pub const fn min_interval_ms(self) -> u64 {
        self.min_interval_ms
    }

    #[must_use]
    pub const fn max_interval_ms(self) -> u64 {
        self.max_interval_ms
    }
}

impl Default for CoalescingPolicy {
    fn default() -> Self {
        Self {
            min_interval_ms: 750,
            max_interval_ms: 1500,
        }
    }
}

/// Emitted live-region announcement ready for AccessKit and the browser bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveAnnouncement {
    pub sequence: u64,
    pub priority: AnnouncementPriority,
    pub emitted_at_ms: u64,
    pub text: String,
    pub coalesced_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PoliteBufferState {
    Empty,
    Accumulating {
        started_at_ms: u64,
        text: String,
        token_count: usize,
    },
}

/// Single-owner throttled live announcement queue for streaming clinical prose
/// and immediate validation/modal alerts.
#[derive(Debug, Clone, Resource)]
pub struct LiveAnnouncementQueue {
    policy: CoalescingPolicy,
    next_sequence: u64,
    last_polite_emit_ms: Option<u64>,
    polite_buffer: PoliteBufferState,
    emitted: Vec<LiveAnnouncement>,
}

impl Default for LiveAnnouncementQueue {
    fn default() -> Self {
        Self::new(CoalescingPolicy::default())
    }
}

impl LiveAnnouncementQueue {
    #[must_use]
    pub const fn new(policy: CoalescingPolicy) -> Self {
        Self {
            policy,
            next_sequence: 1,
            last_polite_emit_ms: None,
            polite_buffer: PoliteBufferState::Empty,
            emitted: Vec::new(),
        }
    }

    #[must_use]
    pub const fn policy(&self) -> CoalescingPolicy {
        self.policy
    }

    /// Buffers an incoming streaming token and flushes only on a qualified
    /// sentence boundary (>= `min_interval_ms`) or max coalescing window (`max_interval_ms`).
    pub fn push_stream_token(&mut self, token: &str, now_ms: u64) -> Option<&LiveAnnouncement> {
        if token.is_empty() {
            return None;
        }

        match &mut self.polite_buffer {
            PoliteBufferState::Empty => {
                self.polite_buffer = PoliteBufferState::Accumulating {
                    started_at_ms: now_ms,
                    text: token.to_owned(),
                    token_count: 1,
                };
            }
            PoliteBufferState::Accumulating {
                text, token_count, ..
            } => {
                text.push_str(token);
                *token_count += 1;
            }
        }

        if self.should_flush_polite(now_ms) {
            return self.flush_polite_now(now_ms);
        }
        None
    }

    /// Periodic clock advance to flush buffered streaming prose once `max_interval_ms` elapses.
    pub fn tick(&mut self, now_ms: u64) -> Option<&LiveAnnouncement> {
        if self.should_flush_polite(now_ms) {
            return self.flush_polite_now(now_ms);
        }
        None
    }

    /// Flushes any remaining polite streaming prose immediately when a turn completes.
    pub fn finish_stream(&mut self, now_ms: u64) -> Option<&LiveAnnouncement> {
        self.flush_polite_now(now_ms)
    }

    /// Immediately announces an urgent validation or state alert (`aria-live="assertive"`)
    /// without dropping or corrupting any pending streaming polite buffer.
    pub fn announce_assertive(
        &mut self,
        message: impl Into<String>,
        now_ms: u64,
    ) -> &LiveAnnouncement {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.emitted.push(LiveAnnouncement {
            sequence,
            priority: AnnouncementPriority::Assertive,
            emitted_at_ms: now_ms,
            text: message.into(),
            coalesced_tokens: 1,
        });
        // Safe: we just pushed an element onto `self.emitted`.
        &self.emitted[self.emitted.len() - 1]
    }

    /// Immediately emits a discrete polite status confirmation (for example,
    /// opening a citation or dismissing a modal) without merging into stream prose.
    pub fn announce_polite_event(
        &mut self,
        message: impl Into<String>,
        now_ms: u64,
    ) -> &LiveAnnouncement {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.last_polite_emit_ms = Some(now_ms);
        self.emitted.push(LiveAnnouncement {
            sequence,
            priority: AnnouncementPriority::Polite,
            emitted_at_ms: now_ms,
            text: message.into(),
            coalesced_tokens: 1,
        });
        &self.emitted[self.emitted.len() - 1]
    }

    #[must_use]
    pub fn emitted(&self) -> &[LiveAnnouncement] {
        &self.emitted
    }

    #[must_use]
    pub fn latest_text(&self, priority: AnnouncementPriority) -> Option<&str> {
        self.emitted
            .iter()
            .rev()
            .find(|item| item.priority == priority)
            .map(|item| item.text.as_str())
    }

    /// Counts announcements of a given priority emitted within `[start_ms, end_ms]`.
    #[must_use]
    pub fn count_in_window(
        &self,
        start_ms: u64,
        end_ms: u64,
        priority: AnnouncementPriority,
    ) -> usize {
        self.emitted
            .iter()
            .filter(|item| {
                item.priority == priority
                    && item.emitted_at_ms >= start_ms
                    && item.emitted_at_ms <= end_ms
            })
            .count()
    }

    fn should_flush_polite(&self, now_ms: u64) -> bool {
        let PoliteBufferState::Accumulating {
            started_at_ms,
            text,
            ..
        } = &self.polite_buffer
        else {
            return false;
        };

        let elapsed_since_start = now_ms.saturating_sub(*started_at_ms);
        if elapsed_since_start >= self.policy.max_interval_ms {
            return true;
        }

        let elapsed_since_last = self
            .last_polite_emit_ms
            .map_or(elapsed_since_start, |last| now_ms.saturating_sub(last));
        let ends_sentence = ends_on_sentence_boundary(text);

        ends_sentence && elapsed_since_last >= self.policy.min_interval_ms
    }

    fn flush_polite_now(&mut self, now_ms: u64) -> Option<&LiveAnnouncement> {
        let prev = std::mem::replace(&mut self.polite_buffer, PoliteBufferState::Empty);
        let PoliteBufferState::Accumulating {
            text, token_count, ..
        } = prev
        else {
            return None;
        };

        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }

        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.last_polite_emit_ms = Some(now_ms);
        self.emitted.push(LiveAnnouncement {
            sequence,
            priority: AnnouncementPriority::Polite,
            emitted_at_ms: now_ms,
            text: trimmed.to_owned(),
            coalesced_tokens: token_count,
        });
        Some(&self.emitted[self.emitted.len() - 1])
    }
}

fn ends_on_sentence_boundary(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || text.ends_with('\n')
}
