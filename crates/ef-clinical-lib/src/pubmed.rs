//! PubMed / NCBI status refresh client contract and shared 100 ms deterministic rate limiter.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hash::sha256_hex;

/// Mandatory minimum spacing between concurrent NCBI/PubMed request starts (`>= 100 ms`).
pub const PUBMED_MIN_SPACING_MS: u64 = 100;

/// Shared deterministic rate limiter enforcing `>= 100 ms` start spacing across all concurrent
/// NCBI/PubMed callers.
#[derive(Debug, Clone)]
pub struct SharedPubMedRateLimiter {
    min_spacing_ms: u64,
    inner: Arc<Mutex<RateLimiterState>>,
}

#[derive(Debug, Default)]
struct RateLimiterState {
    last_start_ms: Option<u64>,
    granted_starts_ms: Vec<u64>,
}

impl Default for SharedPubMedRateLimiter {
    fn default() -> Self {
        Self::new(PUBMED_MIN_SPACING_MS)
    }
}

impl SharedPubMedRateLimiter {
    #[must_use]
    pub fn new(min_spacing_ms: u64) -> Self {
        Self {
            min_spacing_ms: min_spacing_ms.max(PUBMED_MIN_SPACING_MS),
            inner: Arc::new(Mutex::new(RateLimiterState::default())),
        }
    }

    /// Reserves the next eligible request start timestamp (`>= requested_at_ms` and
    /// `>= prev_start + min_spacing_ms`).
    pub fn acquire_start_ms(&self, requested_at_ms: u64) -> Result<u64, PubMedError> {
        let mut guard = self.inner.lock().map_err(|err| PubMedError::Internal {
            detail: format!("rate limiter lock poisoned: {err}"),
        })?;
        let granted = match guard.last_start_ms {
            None => requested_at_ms,
            Some(prev) => requested_at_ms.max(prev.saturating_add(self.min_spacing_ms)),
        };
        guard.last_start_ms = Some(granted);
        guard.granted_starts_ms.push(granted);
        Ok(granted)
    }

    /// Returns all granted request start timestamps in order for concurrency spacing verification.
    #[must_use]
    pub fn granted_starts_ms(&self) -> Vec<u64> {
        self.inner
            .lock()
            .map(|guard| guard.granted_starts_ms.clone())
            .unwrap_or_default()
    }
}

/// Publication status classified verbatim from NCBI NLM records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubMedStatus {
    Current,
    Corrected,
    Retracted,
}

/// Raw NCBI record payload returned by a PubMed status fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubMedRecordPayload {
    pub pmid: Option<String>,
    pub book_accession: Option<String>,
    pub publication_types: Vec<String>,
    pub verbatim_notice: Option<String>,
    pub raw_xml: String,
    pub deleted: bool,
}

/// Simulated or live transport response from NCBI E-utilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubMedTransportOutcome {
    Payload(PubMedRecordPayload),
    ThrottledOrServerError {
        http_status: u16,
        retry_after_ms: u64,
    },
    /// Auth refusal (e.g., 401/403 invalid API key): MUST fail night immediately without retrying
    /// and NEVER expose the key.
    AuthRefused {
        http_status: u16,
    },
}

/// Decision produced when refreshing a PubMed citation status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubMedRefreshDecision {
    UnchangedReconfirmed {
        pmid: String,
        content_sha256: String,
    },
    Updated {
        pmid: String,
        status: PubMedStatus,
        verbatim_notice: Option<String>,
        content_sha256: String,
    },
    BookResolved {
        book_accession: String,
        pmid: String,
        content_sha256: String,
    },
    DeletedEmpty,
    /// Records lacking `publication_types` classify explicitly as unreadable — never empty!
    UnreadableMissingPublicationType {
        pmid: String,
    },
    RetryWithBackoff {
        http_status: u16,
        backoff_ms: u64,
    },
}

/// Errors returned by PubMed status refresh.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PubMedError {
    /// Immediate fatal failure on NCBI auth refusal (zero retries, zero key leakage).
    #[error("NCBI authorization refused (HTTP {http_status}); halting night without retry")]
    AuthorizationRefused { http_status: u16 },
    #[error("PubMed refresh internal error: {detail}")]
    Internal { detail: String },
}

/// Verbatim stored citation hash and status tracked by the reference library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPubMedEntry {
    pub pmid: String,
    pub status: PubMedStatus,
    pub content_sha256: String,
    pub verbatim_notice: Option<String>,
    pub replacements: u64,
}

/// PubMed status refresher enforcing verbatim hash matching, book PMID resolution,
/// unreadable publication-type honesty, 100 ms shared pacing, and fail-fast auth refusal.
#[derive(Debug, Default)]
pub struct PubMedRefresher {
    limiter: SharedPubMedRateLimiter,
    entries: BTreeMap<String, StoredPubMedEntry>,
    book_to_pmid: BTreeMap<String, String>,
}

impl PubMedRefresher {
    #[must_use]
    pub fn new(limiter: SharedPubMedRateLimiter) -> Self {
        Self {
            limiter,
            entries: BTreeMap::new(),
            book_to_pmid: BTreeMap::new(),
        }
    }

    pub fn register_book_pmid(
        &mut self,
        book_accession: impl Into<String>,
        pmid: impl Into<String>,
    ) {
        self.book_to_pmid.insert(book_accession.into(), pmid.into());
    }

    pub fn seed_entry(&mut self, entry: StoredPubMedEntry) {
        self.entries.insert(entry.pmid.clone(), entry);
    }

    #[must_use]
    pub fn entry(&self, pmid: &str) -> Option<&StoredPubMedEntry> {
        self.entries.get(pmid)
    }

    #[must_use]
    pub fn limiter(&self) -> &SharedPubMedRateLimiter {
        &self.limiter
    }

    /// Processes a single rate-limited PubMed fetch response at `requested_at_ms`.
    pub fn refresh_record(
        &mut self,
        requested_at_ms: u64,
        outcome: PubMedTransportOutcome,
    ) -> Result<(u64, PubMedRefreshDecision), PubMedError> {
        let scheduled_ms = self.limiter.acquire_start_ms(requested_at_ms)?;

        let payload = match outcome {
            PubMedTransportOutcome::AuthRefused { http_status } => {
                // Fail the night immediately without retrying or exposing keys.
                return Err(PubMedError::AuthorizationRefused { http_status });
            }
            PubMedTransportOutcome::ThrottledOrServerError {
                http_status,
                retry_after_ms,
            } => {
                return Ok((
                    scheduled_ms,
                    PubMedRefreshDecision::RetryWithBackoff {
                        http_status,
                        backoff_ms: retry_after_ms.max(PUBMED_MIN_SPACING_MS),
                    },
                ));
            }
            PubMedTransportOutcome::Payload(p) => p,
        };

        if payload.deleted {
            return Ok((scheduled_ms, PubMedRefreshDecision::DeletedEmpty));
        }

        let resolved_pmid = match (&payload.pmid, &payload.book_accession) {
            (Some(pmid), _) if !pmid.trim().is_empty() => pmid.clone(),
            (_, Some(book)) => {
                if let Some(mapped) = self.book_to_pmid.get(book) {
                    mapped.clone()
                } else {
                    return Ok((
                        scheduled_ms,
                        PubMedRefreshDecision::UnreadableMissingPublicationType {
                            pmid: book.clone(),
                        },
                    ));
                }
            }
            _ => {
                return Ok((
                    scheduled_ms,
                    PubMedRefreshDecision::UnreadableMissingPublicationType {
                        pmid: "unknown".to_owned(),
                    },
                ));
            }
        };

        if payload.publication_types.is_empty() {
            return Ok((
                scheduled_ms,
                PubMedRefreshDecision::UnreadableMissingPublicationType {
                    pmid: resolved_pmid,
                },
            ));
        }

        let content_sha256 = sha256_hex(payload.raw_xml.as_bytes());
        let status = classify_pubmed_status(&payload.publication_types);

        if let Some(book_acc) = payload.book_accession {
            self.entries.insert(
                resolved_pmid.clone(),
                StoredPubMedEntry {
                    pmid: resolved_pmid.clone(),
                    status,
                    content_sha256: content_sha256.clone(),
                    verbatim_notice: payload.verbatim_notice,
                    replacements: 0,
                },
            );
            return Ok((
                scheduled_ms,
                PubMedRefreshDecision::BookResolved {
                    book_accession: book_acc,
                    pmid: resolved_pmid,
                    content_sha256,
                },
            ));
        }

        if let Some(existing) = self.entries.get(&resolved_pmid)
            && existing.content_sha256 == content_sha256
        {
            return Ok((
                scheduled_ms,
                PubMedRefreshDecision::UnchangedReconfirmed {
                    pmid: resolved_pmid,
                    content_sha256,
                },
            ));
        }

        let prev_replacements = self
            .entries
            .get(&resolved_pmid)
            .map_or(0, |e| e.replacements.saturating_add(1));
        self.entries.insert(
            resolved_pmid.clone(),
            StoredPubMedEntry {
                pmid: resolved_pmid.clone(),
                status,
                content_sha256: content_sha256.clone(),
                verbatim_notice: payload.verbatim_notice.clone(),
                replacements: prev_replacements,
            },
        );

        Ok((
            scheduled_ms,
            PubMedRefreshDecision::Updated {
                pmid: resolved_pmid,
                status,
                verbatim_notice: payload.verbatim_notice,
                content_sha256,
            },
        ))
    }
}

fn classify_pubmed_status(publication_types: &[String]) -> PubMedStatus {
    for pt in publication_types {
        let lower = pt.to_ascii_lowercase();
        if lower.contains("retracted") || lower.contains("retraction") {
            return PubMedStatus::Retracted;
        }
        if lower.contains("erratum") || lower.contains("correction") {
            return PubMedStatus::Corrected;
        }
    }
    PubMedStatus::Current
}
