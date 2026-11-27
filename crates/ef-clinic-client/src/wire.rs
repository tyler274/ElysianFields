//! HTTP/SSE wire serialization, `/elysianfields/api/...` subpath routing,
//! loopback override safety, and strict CSP/cache-header verification.

use std::collections::BTreeMap;

use ef_clinic_domain::auth::DeepLinkTarget;
use ef_clinic_domain::chat::{ChatStreamEvent, ExchangeRecord};
use ef_clinic_domain::workspace::{CitedSourcePayload, MembershipTier};
use ef_core::{
    CachePolicy, ChatId, LoopbackApiOverride, SourceKey, Subpath, SubpathError, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Canonical strict Content-Security-Policy mandated for Browser Wasm under `/elysianfields/`.
pub const STRICT_WASM_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; img-src 'self' data: blob:; font-src 'self'; frame-ancestors 'none'";

/// Typed subpath-mounted API endpoints for Curie Clinic under `/elysianfields/api/...`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClinicEndpoint {
    AuthSignIn,
    AuthVerify,
    AuthWhoAmI,
    AuthSignOut,
    PatientWorkspaces,
    PatientRoster {
        query: String,
    },
    PatientWorkspaceDetail {
        workspace_id: WorkspaceId,
    },
    ChatSnapshot {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
    },
    CitedSource {
        workspace_id: WorkspaceId,
        chat_id: ChatId,
        source_key: SourceKey,
    },
    Commands,
    WorkspaceEvents {
        since_event_id: u64,
    },
    ComposerDraft {
        workspace_id: WorkspaceId,
        chat_target: String,
    },
}

impl ClinicEndpoint {
    /// Resolves this endpoint relative to `/elysianfields/api/...` with optional loopback override.
    pub fn resolve_url(
        &self,
        loopback_override: Option<&LoopbackApiOverride>,
    ) -> Result<String, SubpathError> {
        let rel = match self {
            Self::AuthSignIn => "auth/sign-in".to_owned(),
            Self::AuthVerify => "auth/verify".to_owned(),
            Self::AuthWhoAmI => "auth/whoami".to_owned(),
            Self::AuthSignOut => "auth/sign-out".to_owned(),
            Self::PatientWorkspaces => "patient-workspaces".to_owned(),
            Self::PatientRoster { query } => {
                format!("patient-roster?query={}", percent_encode_query(query))
            }
            Self::PatientWorkspaceDetail { workspace_id } => {
                format!("patient-workspaces/{}", workspace_id.as_str())
            }
            Self::ChatSnapshot {
                workspace_id,
                chat_id,
            } => format!(
                "patient-workspaces/{}/chats/{}/snapshot",
                workspace_id.as_str(),
                chat_id.as_str()
            ),
            Self::CitedSource {
                workspace_id,
                chat_id,
                source_key,
            } => format!(
                "patient-workspaces/{}/chats/{}/sources/{}",
                workspace_id.as_str(),
                chat_id.as_str(),
                source_key.as_str()
            ),
            Self::Commands => "commands".to_owned(),
            Self::WorkspaceEvents { since_event_id } => {
                format!("patient-workspaces/events?since={since_event_id}")
            }
            Self::ComposerDraft {
                workspace_id,
                chat_target,
            } => format!(
                "physician-state/drafts/{}/{}",
                workspace_id.as_str(),
                chat_target
            ),
        };
        let mounted = Subpath::resolve_api(&rel)?;
        match loopback_override {
            Some(base) => Ok(format!("{}{mounted}", base.as_str())),
            None => Ok(mounted),
        }
    }

    /// Required cache policy for this API route (`ClinicalNoStore` on every auth/clinical route).
    #[must_use]
    pub const fn required_cache_policy(&self) -> CachePolicy {
        CachePolicy::ClinicalNoStore
    }
}

/// Request body for `POST /elysianfields/api/auth/sign-in`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignInRequest {
    pub email: String,
}

/// Request body for `POST /elysianfields/api/auth/verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifySignInRequest {
    pub email: String,
    pub token: String,
}

/// Wire payload for `GET /elysianfields/api/patient-workspaces/{workspace_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatientWorkspaceDetail {
    pub patient_workspace_id: WorkspaceId,
    pub display_name: String,
    pub membership: MembershipTier,
    pub time_zone: String,
    pub default_chat_id: ChatId,
    pub patient_synopsis: Option<String>,
}

/// Wire payload for `GET /elysianfields/api/patient-workspaces/{ws}/chats/{chat}/snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSnapshotWire {
    pub workspace_id: WorkspaceId,
    pub chat_id: ChatId,
    pub transcript: Vec<ExchangeRecord>,
    pub sources: BTreeMap<SourceKey, CitedSourcePayload>,
    pub queued: Vec<String>,
    pub archived: bool,
}

/// Request body for `PUT /elysianfields/api/physician-state/drafts/{ws}/{chat}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutDraftRequest {
    pub text: String,
    pub expected_revision: u64,
}

/// Boot configuration parsed from the initial browser location under `/elysianfields/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBootConfig {
    pub mount_subpath: Subpath,
    pub canonical_redirect: Option<String>,
    pub deep_link: DeepLinkTarget,
    pub deep_link_chat: Option<ChatId>,
    pub query_lang: Option<String>,
    pub loopback_api_override: Option<LoopbackApiOverride>,
}

impl ClientBootConfig {
    /// Parses initial path and query string enforcing `/elysianfields/` mount boundaries
    /// and loopback-only `?api=` safety (non-loopback overrides are ignored).
    pub fn from_location(path_and_query: &str) -> Result<Self, SubpathError> {
        let canonical_redirect = Subpath::canonical_redirect_if_bare(path_and_query);
        let mount_subpath = Subpath::parse_mounted(path_and_query)?;
        let query = path_and_query.split_once('?').map(|(_, q)| q).unwrap_or("");

        let mut deep_link = DeepLinkTarget::Today;
        let mut deep_link_chat = None;
        let mut query_lang = None;
        let mut loopback_api_override = None;

        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (raw_k, raw_v) = pair.split_once('=').unwrap_or((pair, ""));
            let val = percent_decode_query(raw_v);
            match raw_k {
                "patient" => {
                    if let Ok(ws) = WorkspaceId::new(&val) {
                        deep_link = DeepLinkTarget::LinkedPatient(ws);
                    }
                }
                "chat" => {
                    if let Ok(chat_id) = ChatId::new(&val) {
                        deep_link_chat = Some(chat_id);
                    }
                }
                "lang" => {
                    if !val.trim().is_empty() {
                        query_lang = Some(val);
                    }
                }
                "api" => {
                    // Security invariant: non-loopback ?api= overrides are silently ignored.
                    if let Ok(valid_loopback) = LoopbackApiOverride::parse(&val) {
                        loopback_api_override = Some(valid_loopback);
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            mount_subpath,
            canonical_redirect,
            deep_link,
            deep_link_chat,
            query_lang,
            loopback_api_override,
        })
    }

    /// Resolves a client API endpoint under `/elysianfields/api/...` (plus optional loopback origin).
    pub fn api_url(&self, endpoint: &ClinicEndpoint) -> Result<String, SubpathError> {
        endpoint.resolve_url(self.loopback_api_override.as_ref())
    }
}

/// Errors returned while decoding SSE wire frames.
#[derive(Debug, Error)]
pub enum SseWireError {
    #[error("malformed SSE frame missing data payload")]
    MissingDataLine,
    #[error("failed to decode SSE event JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Encodes a [`ChatStreamEvent`] into a standard SSE wire frame (`event:`, optional `id:`, `data:`).
pub fn encode_sse_frame(event: &ChatStreamEvent) -> Result<String, serde_json::Error> {
    if matches!(event, ChatStreamEvent::KeepAlive) {
        return Ok(": keep-alive\n\n".to_owned());
    }
    let event_id = match event {
        ChatStreamEvent::DeltaChunk { event_id, .. }
        | ChatStreamEvent::StepRecorded { event_id, .. }
        | ChatStreamEvent::Completed { event_id, .. }
        | ChatStreamEvent::Cancelled { event_id, .. }
        | ChatStreamEvent::CouldntFinish { event_id, .. }
        | ChatStreamEvent::ResyncRequired { event_id, .. } => *event_id,
        ChatStreamEvent::KeepAlive => 0,
    };
    let json = serde_json::to_string(event)?;
    Ok(format!("id: {event_id}\nevent: message\ndata: {json}\n\n"))
}

/// Decodes a standard SSE wire frame into a typed [`ChatStreamEvent`].
pub fn decode_sse_frame(raw_frame: &str) -> Result<ChatStreamEvent, SseWireError> {
    let trimmed = raw_frame.trim();
    if trimmed.starts_with(':') {
        return Ok(ChatStreamEvent::KeepAlive);
    }
    let mut data_lines = Vec::new();
    for line in trimmed.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start());
        }
    }
    if data_lines.is_empty() {
        return Err(SseWireError::MissingDataLine);
    }
    let joined = data_lines.join("\n");
    Ok(serde_json::from_str(&joined)?)
}

/// Verifies that a CSP header string enforces strict Wasm rules with zero unsafe-inline/eval
/// and frame-ancestors protection.
#[must_use]
pub fn is_strict_wasm_csp(policy: &str) -> bool {
    policy.contains("default-src 'self'")
        && policy.contains("script-src 'self' 'wasm-unsafe-eval'")
        && policy.contains("font-src 'self'")
        && policy.contains("frame-ancestors 'none'")
        && !policy.contains("'unsafe-inline'")
        && !policy.contains("'unsafe-eval'")
}

fn percent_encode_query(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn percent_decode_query(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[idx + 1]), hex_nibble(bytes[idx + 2]))
        {
            out.push((hi << 4) | lo);
            idx += 3;
            continue;
        }
        out.push(if bytes[idx] == b'+' { b' ' } else { bytes[idx] });
        idx += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
