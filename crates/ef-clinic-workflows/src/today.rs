//! Today urgency/standing queue, incomplete-source honesty, and channel health banners (`TodaySaysSo`).

use std::collections::BTreeMap;

use ef_core::{PhysicianId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Action urgency ordering on Today: `Now` always sorts ahead of `Today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodayUrgency {
    Now,
    Today,
}

/// Category of an actionable item on the Today queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodayItemKind {
    UrgentClinicalConcern,
    SameDayClinicalConcern,
    PreparedVisitBriefing,
    UnlinkedPatientMessage,
    UnreadPatientMessage,
}

/// Physician standing recorded against a concrete clinical `version_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PhysicianItemStanding {
    Unseen,
    Seen { version_id: u64 },
    Done { version_id: u64 },
}

impl PhysicianItemStanding {
    /// Returns true if this item has been marked seen at or after `current_version_id`.
    #[must_use]
    pub const fn is_seen_for(self, current_version_id: u64) -> bool {
        match self {
            Self::Unseen => false,
            Self::Seen { version_id } | Self::Done { version_id } => {
                version_id >= current_version_id
            }
        }
    }

    /// Returns true if this item has been marked done at or after `current_version_id`.
    ///
    /// When newer clinical data advances `current_version_id`, the item resurfaces honestly.
    #[must_use]
    pub const fn is_done_for(self, current_version_id: u64) -> bool {
        match self {
            Self::Done { version_id } => version_id >= current_version_id,
            Self::Unseen | Self::Seen { .. } => false,
        }
    }
}

/// Raw clinical item candidate fed into the Today queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodayQueueItem {
    pub item_id: String,
    pub workspace_id: Option<WorkspaceId>,
    pub urgency: TodayUrgency,
    pub waiting_since_epoch_secs: u64,
    pub kind: TodayItemKind,
    pub version_id: u64,
    pub summary: String,
    pub can_mark_done: bool,
}

/// Rendered item on Today with evaluated physician standing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedTodayItem {
    pub item: TodayQueueItem,
    pub seen: bool,
    pub done: bool,
}

/// Scheduled patient visit entry shown on Today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledVisitEntry {
    pub workspace_id: WorkspaceId,
    pub display_name: String,
    pub scheduled_at_epoch_secs: u64,
    pub briefing_ready: bool,
}

/// Raw health status of a practice communication channel (Gmail/Spruce).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChannelHealthStatus {
    Connected,
    TempError { detail: String },
    Revoked { detail: String },
    Disconnected,
}

/// Today banner derived from [`ChannelHealthStatus`] enforcing the `TodaySaysSo` invariant:
/// - `TempError` -> temporary trouble notice WITHOUT asking to reconnect.
/// - `Revoked` -> explicit reconnect prompt.
/// - `Connected` -> no error banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TodayChannelBanner {
    None,
    TemporaryTrouble {
        channel_id: String,
        detail: String,
        asks_to_reconnect: bool,
    },
    ReconnectRequired {
        channel_id: String,
        detail: String,
    },
    Disconnected {
        channel_id: String,
    },
}

/// Single owner deriving a Today channel health banner (`TodaySaysSo`).
#[must_use]
pub fn derive_channel_banner(channel_id: &str, status: &ChannelHealthStatus) -> TodayChannelBanner {
    match status {
        ChannelHealthStatus::Connected => TodayChannelBanner::None,
        ChannelHealthStatus::TempError { detail } => TodayChannelBanner::TemporaryTrouble {
            channel_id: channel_id.to_owned(),
            detail: detail.clone(),
            asks_to_reconnect: false,
        },
        ChannelHealthStatus::Revoked { detail } => TodayChannelBanner::ReconnectRequired {
            channel_id: channel_id.to_owned(),
            detail: detail.clone(),
        },
        ChannelHealthStatus::Disconnected => TodayChannelBanner::Disconnected {
            channel_id: channel_id.to_owned(),
        },
    }
}

/// Explicit record of a producer or access check that failed while building Today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodayFailedPart {
    pub producer: String,
    pub reason: String,
}

/// Complete Today view state separating honest `Quiet` from `Actionable` / `Incomplete`.
///
/// A partial producer failure NEVER renders as `Quiet`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum TodayViewState {
    Quiet {
        also_reviewed: usize,
        scheduled_visits: Vec<ScheduledVisitEntry>,
        channel_banners: Vec<TodayChannelBanner>,
    },
    Actionable {
        items: Vec<RenderedTodayItem>,
        scheduled_visits: Vec<ScheduledVisitEntry>,
        also_reviewed: usize,
        failed_parts: Vec<TodayFailedPart>,
        may_be_missing_someone: bool,
        channel_banners: Vec<TodayChannelBanner>,
    },
}

/// Stateful per-physician Today standing tracker and view assembler.
#[derive(Debug, Clone, Default)]
pub struct TodayQueueEngine {
    standings: BTreeMap<(PhysicianId, String), PhysicianItemStanding>,
    channel_health: BTreeMap<String, ChannelHealthStatus>,
}

impl TodayQueueEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates a practice channel's health status.
    pub fn set_channel_health(
        &mut self,
        channel_id: impl Into<String>,
        status: ChannelHealthStatus,
    ) {
        self.channel_health.insert(channel_id.into(), status);
    }

    /// Marks an item seen by `physician_id` at `version_id`.
    pub fn mark_seen(&mut self, physician_id: &PhysicianId, item_id: &str, version_id: u64) {
        let key = (physician_id.clone(), item_id.to_owned());
        let next = match self.standings.get(&key).copied() {
            Some(PhysicianItemStanding::Done { version_id: done_v }) if done_v >= version_id => {
                PhysicianItemStanding::Done { version_id: done_v }
            }
            _ => PhysicianItemStanding::Seen { version_id },
        };
        self.standings.insert(key, next);
    }

    /// Marks an item done by `physician_id` at `version_id` (only when `can_mark_done` is true).
    pub fn mark_done(&mut self, physician_id: &PhysicianId, item: &TodayQueueItem) -> bool {
        if !item.can_mark_done {
            return false;
        }
        let key = (physician_id.clone(), item.item_id.clone());
        self.standings.insert(
            key,
            PhysicianItemStanding::Done {
                version_id: item.version_id,
            },
        );
        true
    }

    /// Assembles Today deterministically:
    /// - Urgent (`Now`) items always sort ahead of `Today`, then oldest `waiting_since` first.
    /// - Items marked `Done` for the current `version_id` are suppressed; newer `version_id` resurfaces them.
    /// - Any `failed_parts` sets `may_be_missing_someone = true` and prevents rendering `Quiet`.
    #[must_use]
    pub fn build_view(
        &self,
        physician_id: &PhysicianId,
        candidates: &[TodayQueueItem],
        mut scheduled_visits: Vec<ScheduledVisitEntry>,
        also_reviewed: usize,
        failed_parts: Vec<TodayFailedPart>,
    ) -> TodayViewState {
        scheduled_visits.sort_by_key(|v| v.scheduled_at_epoch_secs);

        let mut rendered: Vec<RenderedTodayItem> = Vec::new();
        for item in candidates {
            let standing = self
                .standings
                .get(&(physician_id.clone(), item.item_id.clone()))
                .copied()
                .unwrap_or(PhysicianItemStanding::Unseen);
            if standing.is_done_for(item.version_id) {
                continue;
            }
            rendered.push(RenderedTodayItem {
                item: item.clone(),
                seen: standing.is_seen_for(item.version_id),
                done: false,
            });
        }

        rendered.sort_by(|a, b| {
            a.item
                .urgency
                .cmp(&b.item.urgency)
                .then_with(|| {
                    a.item
                        .waiting_since_epoch_secs
                        .cmp(&b.item.waiting_since_epoch_secs)
                })
                .then_with(|| a.item.item_id.cmp(&b.item.item_id))
        });

        let channel_banners: Vec<TodayChannelBanner> = self
            .channel_health
            .iter()
            .filter_map(|(id, status)| {
                let banner = derive_channel_banner(id, status);
                match banner {
                    TodayChannelBanner::None => None,
                    other => Some(other),
                }
            })
            .collect();

        let may_be_missing_someone = !failed_parts.is_empty();
        if rendered.is_empty() && !may_be_missing_someone {
            TodayViewState::Quiet {
                also_reviewed,
                scheduled_visits,
                channel_banners,
            }
        } else {
            TodayViewState::Actionable {
                items: rendered,
                scheduled_visits,
                also_reviewed,
                failed_parts,
                may_be_missing_someone,
                channel_banners,
            }
        }
    }
}
