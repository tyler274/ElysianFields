//! Live native HTTP/1.1 + SSE [`ClinicTransport`] built strictly on `std::net::TcpStream`.
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` with zero additional third-party crates.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use ef_clinic_domain::auth::{
    AuthError, SignInInitiationResponse, VerifiedSessionPayload, WhoAmIPayload,
};
use ef_clinic_domain::chat::{
    ChatStreamEvent, CommandAcceptance, CommandEnvelope, CommandPayload, DraftSaveOutcome,
    PersistedDraft,
};
use ef_clinic_domain::workspace::{CitedSourcePayload, PatientRosterState, WorkspaceAccessError};
use ef_core::{CachePolicy, ChatId, SessionId, SourceKey, StructuredRefusal, WorkspaceId};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::transport::{ClinicTransport, TransportError, TransportResponseMeta};
use crate::wire::{
    ChatSnapshotWire, ClinicEndpoint, PatientWorkspaceDetail, PutDraftRequest, SignInRequest,
    VerifySignInRequest, decode_sse_frame,
};

/// Raw HTTP/1.1 response received over a live TCP socket.
#[derive(Debug, Clone)]
pub struct LiveHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl LiveHttpResponse {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Parses the response body as UTF-8 text.
    pub fn body_text(&self) -> Result<&str, TransportError> {
        std::str::from_utf8(&self.body).map_err(|err| TransportError::Unavailable {
            message: format!("non-UTF-8 HTTP response body: {err}"),
        })
    }

    /// Parses the response body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, TransportError> {
        serde_json::from_slice(&self.body).map_err(TransportError::from)
    }
}

/// Real native HTTP/1.1 + SSE [`ClinicTransport`] communicating over [`TcpStream`]
/// with a live `ef-clinic-server` mounted under `/elysianfields/`.
#[derive(Debug, Clone)]
pub struct LiveHttpClinicTransport {
    addr: SocketAddr,
    timeout: Duration,
    last_meta: Option<TransportResponseMeta>,
    active_chat: Option<(WorkspaceId, ChatId)>,
}

impl LiveHttpClinicTransport {
    /// Creates a new live HTTP/1.1 + SSE transport targeting `addr`.
    #[must_use]
    pub const fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            timeout: Duration::from_secs(10),
            last_meta: None,
            active_chat: None,
        }
    }

    /// Returns the target server socket address.
    #[must_use]
    pub const fn server_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Updates the socket read/write timeout.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Executes a raw HTTP/1.1 request against the live server without enforcing status == 200.
    pub fn request_raw(
        &mut self,
        method: &str,
        target: &str,
        bearer: Option<&SessionId>,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> Result<LiveHttpResponse, TransportError> {
        let mut stream = TcpStream::connect_timeout(&self.addr, self.timeout).map_err(|err| {
            TransportError::Unavailable {
                message: format!("failed to connect to {}: {err}", self.addr),
            }
        })?;
        let _ = stream.set_read_timeout(Some(self.timeout));
        let _ = stream.set_write_timeout(Some(self.timeout));

        let payload = body.unwrap_or(&[]);
        let mut wire = format!("{method} {target} HTTP/1.1\r\nHost: {}\r\n", self.addr);
        if let Some(session) = bearer {
            wire.push_str(&format!("Authorization: Bearer {}\r\n", session.as_str()));
        }
        for (name, value) in extra_headers {
            wire.push_str(&format!("{name}: {value}\r\n"));
        }
        if !payload.is_empty() {
            wire.push_str("Content-Type: application/json\r\n");
            wire.push_str(&format!("Content-Length: {}\r\n", payload.len()));
        }
        wire.push_str("Connection: close\r\n\r\n");

        let mut req_bytes = wire.into_bytes();
        req_bytes.extend_from_slice(payload);

        stream
            .write_all(&req_bytes)
            .and_then(|()| stream.flush())
            .map_err(|err| TransportError::Unavailable {
                message: format!("failed to write HTTP request to {target}: {err}"),
            })?;

        let mut raw_resp = Vec::new();
        stream
            .read_to_end(&mut raw_resp)
            .map_err(|err| TransportError::Unavailable {
                message: format!("failed to read HTTP response from {target}: {err}"),
            })?;

        parse_http1_response(&raw_resp)
    }

    /// Executes an HTTP/1.1 JSON request against the live server, returning the raw status,
    /// headers, and decoded JSON value.
    pub fn request_json<B: Serialize>(
        &mut self,
        method: &str,
        target: &str,
        bearer: Option<&SessionId>,
        body: Option<&B>,
    ) -> Result<(u16, BTreeMap<String, String>, serde_json::Value), TransportError> {
        let encoded = match body {
            Some(val) => Some(serde_json::to_vec(val)?),
            None => None,
        };
        let resp = self.request_raw(method, target, bearer, encoded.as_deref(), &[])?;
        let json = if resp.body.is_empty() {
            serde_json::Value::Null
        } else {
            resp.json()?
        };
        Ok((resp.status, resp.headers, json))
    }

    fn record_api_meta(&mut self, endpoint: &ClinicEndpoint, resp: &LiveHttpResponse) {
        if let Ok(url) = endpoint.resolve_url(None) {
            let cache_policy = if resp
                .header("cache-control")
                .is_some_and(|cc| cc.contains("no-store"))
            {
                CachePolicy::ClinicalNoStore
            } else {
                endpoint.required_cache_policy()
            };
            self.last_meta = Some(TransportResponseMeta {
                endpoint_url: url,
                cache_policy,
            });
        }
    }

    fn map_error_response(resp: &LiveHttpResponse) -> TransportError {
        if resp.status == 429 {
            let retry_after_secs = resp
                .header("retry-after")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1);
            return TransportError::RateLimited { retry_after_secs };
        }

        if let Ok(refusal) = serde_json::from_slice::<StructuredRefusal>(&resp.body) {
            return TransportError::Auth(AuthError::Refused(refusal));
        }

        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
            let error_code = val.get("error").and_then(serde_json::Value::as_str);
            let reason = val.get("reason").and_then(serde_json::Value::as_str);
            let detail = val
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
                .to_owned();

            if error_code == Some("mail_delivery_failed") {
                return TransportError::Auth(AuthError::MailDeliveryFailed);
            }
            if error_code == Some("invalid_credentials") {
                return TransportError::Auth(AuthError::BlankOrMalformedCredentials);
            }
            if reason == Some("source_withdrawn") {
                return TransportError::Workspace(WorkspaceAccessError::SourceWithdrawn);
            }
            if error_code == Some("clinical_disclosure_forbidden") {
                return TransportError::Workspace(
                    WorkspaceAccessError::ClinicalDisclosureForbidden,
                );
            }
            if error_code == Some("not_found") || resp.status == 404 {
                return TransportError::Workspace(WorkspaceAccessError::NotFound);
            }
            if error_code == Some("store_unavailable") {
                return TransportError::Workspace(WorkspaceAccessError::StoreUnavailable {
                    message: detail,
                });
            }
            if error_code == Some("chat_command_error") {
                return TransportError::Unavailable {
                    message: format!("chat command error (HTTP {}): {detail}", resp.status),
                };
            }
            return TransportError::Unavailable {
                message: format!("HTTP {}: {detail}", resp.status),
            };
        }

        if resp.status == 404 {
            return TransportError::Workspace(WorkspaceAccessError::NotFound);
        }

        TransportError::Unavailable {
            message: format!("unexpected HTTP status {}", resp.status),
        }
    }
}

impl ClinicTransport for LiveHttpClinicTransport {
    fn initiate_sign_in(
        &mut self,
        req: &SignInRequest,
        _now_secs: u64,
    ) -> Result<SignInInitiationResponse, TransportError> {
        let endpoint = ClinicEndpoint::AuthSignIn;
        let url = endpoint.resolve_url(None)?;
        let body = serde_json::to_vec(req)?;
        let resp = self.request_raw("POST", &url, None, Some(&body), &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn verify_sign_in(
        &mut self,
        req: &VerifySignInRequest,
        _now_secs: u64,
    ) -> Result<VerifiedSessionPayload, TransportError> {
        let endpoint = ClinicEndpoint::AuthVerify;
        let url = endpoint.resolve_url(None)?;
        let body = serde_json::to_vec(req)?;
        let resp = self.request_raw("POST", &url, None, Some(&body), &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn whoami(
        &mut self,
        session: Option<&SessionId>,
        _now_secs: u64,
    ) -> Result<WhoAmIPayload, TransportError> {
        let endpoint = ClinicEndpoint::AuthWhoAmI;
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("GET", &url, session, None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn sign_out(&mut self, session: &SessionId, _now_secs: u64) -> Result<(), TransportError> {
        let endpoint = ClinicEndpoint::AuthSignOut;
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("POST", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            Ok(())
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_patient_roster(
        &mut self,
        session: &SessionId,
        query: Option<&str>,
        _now_secs: u64,
    ) -> Result<PatientRosterState, TransportError> {
        let endpoint = match query {
            Some(q) => ClinicEndpoint::PatientRoster {
                query: q.to_owned(),
            },
            None => ClinicEndpoint::PatientWorkspaces,
        };
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("GET", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 || resp.status == 503 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_workspace_detail(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        _now_secs: u64,
    ) -> Result<PatientWorkspaceDetail, TransportError> {
        let endpoint = ClinicEndpoint::PatientWorkspaceDetail {
            workspace_id: workspace_id.clone(),
        };
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("GET", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            let detail: PatientWorkspaceDetail = resp.json()?;
            self.active_chat = Some((
                detail.patient_workspace_id.clone(),
                detail.default_chat_id.clone(),
            ));
            Ok(detail)
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_chat_snapshot(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        _now_secs: u64,
    ) -> Result<ChatSnapshotWire, TransportError> {
        let endpoint = ClinicEndpoint::ChatSnapshot {
            workspace_id: workspace_id.clone(),
            chat_id: chat_id.clone(),
        };
        let url = endpoint.resolve_url(None)?;
        self.active_chat = Some((workspace_id.clone(), chat_id.clone()));
        let resp = self.request_raw("GET", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_cited_source(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_id: &ChatId,
        source_key: &SourceKey,
        _now_secs: u64,
    ) -> Result<CitedSourcePayload, TransportError> {
        let endpoint = ClinicEndpoint::CitedSource {
            workspace_id: workspace_id.clone(),
            chat_id: chat_id.clone(),
            source_key: source_key.clone(),
        };
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("GET", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn submit_command(
        &mut self,
        session: &SessionId,
        envelope: &CommandEnvelope,
        _now_secs: u64,
    ) -> Result<CommandAcceptance, TransportError> {
        let (ws_id, chat_id) = match &envelope.command {
            CommandPayload::MessageSubmit(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
            CommandPayload::WorkStop(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
            CommandPayload::WorkResume(p) => (p.patient_workspace_id.clone(), p.chat_id.clone()),
        };
        self.active_chat = Some((ws_id, chat_id));

        let endpoint = ClinicEndpoint::Commands;
        let url = endpoint.resolve_url(None)?;
        let body = serde_json::to_vec(envelope)?;
        let resp = self.request_raw("POST", &url, Some(session), Some(&body), &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn poll_stream_events(
        &mut self,
        session: &SessionId,
        since_event_id: u64,
        _now_secs: u64,
    ) -> Result<Vec<ChatStreamEvent>, TransportError> {
        let endpoint = ClinicEndpoint::WorkspaceEvents { since_event_id };
        let mut url = endpoint.resolve_url(None)?;
        if let Some((ref ws_id, ref chat_id)) = self.active_chat {
            url.push_str(&format!(
                "&workspace_id={}&chat_id={}",
                ws_id.as_str(),
                chat_id.as_str()
            ));
        }

        let resp = self.request_raw(
            "GET",
            &url,
            Some(session),
            None,
            &[
                ("Accept", "text/event-stream"),
                ("Last-Event-ID", &since_event_id.to_string()),
            ],
        )?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status != 200 {
            return Err(Self::map_error_response(&resp));
        }

        let text = resp.body_text()?;
        let normalized = text.replace("\r\n", "\n");
        let mut events = Vec::new();
        for block in normalized.split("\n\n") {
            let trimmed = block.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = decode_sse_frame(trimmed)?;
            if !matches!(event, ChatStreamEvent::KeepAlive) {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn get_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        _now_secs: u64,
    ) -> Result<PersistedDraft, TransportError> {
        let endpoint = ClinicEndpoint::ComposerDraft {
            workspace_id: workspace_id.clone(),
            chat_target: chat_target.to_owned(),
        };
        let url = endpoint.resolve_url(None)?;
        let resp = self.request_raw("GET", &url, Some(session), None, &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn put_draft(
        &mut self,
        session: &SessionId,
        workspace_id: &WorkspaceId,
        chat_target: &str,
        req: &PutDraftRequest,
        _now_secs: u64,
    ) -> Result<DraftSaveOutcome, TransportError> {
        let endpoint = ClinicEndpoint::ComposerDraft {
            workspace_id: workspace_id.clone(),
            chat_target: chat_target.to_owned(),
        };
        let url = endpoint.resolve_url(None)?;
        let body = serde_json::to_vec(req)?;
        let resp = self.request_raw("PUT", &url, Some(session), Some(&body), &[])?;
        self.record_api_meta(&endpoint, &resp);
        if resp.status == 200 {
            let saved: PersistedDraft = resp.json()?;
            return Ok(DraftSaveOutcome::Saved(saved));
        }
        if resp.status == 409 {
            let val: serde_json::Value = resp.json()?;
            if let Some(current_val) = val.get("current") {
                let current: PersistedDraft = serde_json::from_value(current_val.clone())?;
                return Ok(DraftSaveOutcome::Conflict { current });
            }
        }
        if resp.status == 503 {
            let detail = serde_json::from_slice::<serde_json::Value>(&resp.body)
                .ok()
                .and_then(|v| {
                    v.get("detail")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "draft save temporarily unavailable".to_owned());
            return Ok(DraftSaveOutcome::TransientFailure { message: detail });
        }
        Err(Self::map_error_response(&resp))
    }

    fn last_response_meta(&self) -> Option<TransportResponseMeta> {
        self.last_meta.clone()
    }
}

fn parse_http1_response(raw: &[u8]) -> Result<LiveHttpResponse, TransportError> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| TransportError::Unavailable {
            message: "malformed HTTP/1.1 response missing header separator".to_owned(),
        })?;

    let head = std::str::from_utf8(&raw[..sep]).map_err(|err| TransportError::Unavailable {
        message: format!("invalid UTF-8 HTTP response header: {err}"),
    })?;
    let body_slice = &raw[sep + 4..];

    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or_else(|| TransportError::Unavailable {
        message: "missing HTTP status line".to_owned(),
    })?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| TransportError::Unavailable {
            message: format!("invalid HTTP status line: {status_line}"),
        })?;

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    let is_chunked = headers
        .get("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));

    let body = if is_chunked {
        decode_chunked_body(body_slice)?
    } else if let Some(len) = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
    {
        body_slice[..len.min(body_slice.len())].to_vec()
    } else {
        body_slice.to_vec()
    };

    Ok(LiveHttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked_body(mut input: &[u8]) -> Result<Vec<u8>, TransportError> {
    let mut out = Vec::new();
    while let Some(crlf) = input.windows(2).position(|w| w == b"\r\n") {
        let size_str =
            std::str::from_utf8(&input[..crlf]).map_err(|err| TransportError::Unavailable {
                message: format!("invalid chunked length header: {err}"),
            })?;
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size =
            usize::from_str_radix(size_hex, 16).map_err(|err| TransportError::Unavailable {
                message: format!("invalid hex chunk length '{size_hex}': {err}"),
            })?;
        input = &input[crlf + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size {
            return Err(TransportError::Unavailable {
                message: "truncated chunked HTTP body".to_owned(),
            });
        }
        out.extend_from_slice(&input[..size]);
        input = &input[size..];
        if input.starts_with(b"\r\n") {
            input = &input[2..];
        }
    }
    Ok(out)
}
