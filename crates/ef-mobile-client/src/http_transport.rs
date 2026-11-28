//! Live native HTTP/1.1 [`MobileClientTransport`] built strictly on `std::net::TcpStream`.
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` with zero additional third-party crates
//! and zero cross-product imports.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use ef_connected_care::{
    ConnectionId, ConnectionStanding, DateOfBirth, DobVerificationOutcome, EnvelopeItem,
    InviteChallenge, InviteCredential, PairKey, Part2WrittenConsent, TransferEnvelope,
};
use ef_core::IdempotencyKey;
use ef_mobile_domain::{
    AccountFencedHealthBatch, HealthSampleEntry, HealthUploadReceipt, IncomingBatchItem,
    MobileAuthError, MobileSessionId, PatientRecord, PortablePatientExport, SavepointBatchReport,
    VerifiedMobileSession,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::transport::{
    AccountDeleteReceipt, ConnectedCareStatusResult, MobileApiEndpoint, MobileClientTransport,
    MobileTransportError,
};

/// Raw HTTP/1.1 response received over a live TCP socket from `ef-mobile-server`.
#[derive(Debug, Clone)]
pub struct LiveMobileHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl LiveMobileHttpResponse {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Parses the response body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, MobileTransportError> {
        serde_json::from_slice(&self.body).map_err(MobileTransportError::from)
    }
}

#[derive(Debug, Deserialize)]
struct RequestLinkResponse {
    token: String,
}

#[derive(Debug, Deserialize)]
struct SignOutResponse {
    revoked: usize,
}

#[derive(Debug, Deserialize)]
struct ListRecordsResponse {
    records: Vec<PatientRecord>,
}

#[derive(Debug, Deserialize)]
struct HealthTrendsResponse {
    samples: Vec<HealthSampleEntry>,
}

#[derive(Debug, Deserialize)]
struct ActiveConnectionsResponse {
    connections: Vec<ConnectionStanding>,
}

/// Pure-`std::net::TcpStream` HTTP/1.1 [`MobileClientTransport`] targeting a live `ef-mobile-server`.
#[derive(Debug, Clone)]
pub struct LiveHttpMobileTransport {
    addr: SocketAddr,
    timeout: Duration,
}

impl LiveHttpMobileTransport {
    /// Creates a new live HTTP/1.1 transport targeting `addr`.
    #[must_use]
    pub const fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            timeout: Duration::from_secs(10),
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

    /// Queries `/elysianfields/mobile-api/readyz`, returning the raw status code and JSON body.
    pub fn readyz(&self) -> Result<(u16, serde_json::Value), MobileTransportError> {
        let resp = self.request_raw("GET", &MobileApiEndpoint::Readyz.path(), None, None)?;
        let json = resp.json()?;
        Ok((resp.status, json))
    }

    /// Executes a raw HTTP/1.1 request against `ef-mobile-server`.
    pub fn request_raw(
        &self,
        method: &str,
        target: &str,
        bearer: Option<&MobileSessionId>,
        body: Option<&[u8]>,
    ) -> Result<LiveMobileHttpResponse, MobileTransportError> {
        let mut stream = TcpStream::connect_timeout(&self.addr, self.timeout).map_err(|err| {
            MobileTransportError::Unavailable {
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
            .map_err(|err| MobileTransportError::Unavailable {
                message: format!("failed to write HTTP request to {target}: {err}"),
            })?;
        let _ = stream.shutdown(Shutdown::Write);

        let raw_resp = read_http1_response_bytes(&mut stream, target)?;
        parse_http1_response(&raw_resp)
    }

    fn map_error_response(resp: &LiveMobileHttpResponse) -> MobileTransportError {
        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&resp.body) {
            let error = val
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("http_error")
                .to_owned();
            let detail = val
                .get("detail")
                .or_else(|| val.get("reason"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
                .to_owned();

            if error == "auth_error" {
                if resp.status == 403 && detail.contains("deleted") {
                    return MobileTransportError::Auth(MobileAuthError::AccountDeleted);
                }
                if resp.status == 401 {
                    return MobileTransportError::Auth(MobileAuthError::SessionEnded);
                }
            }

            return MobileTransportError::Http {
                status: resp.status,
                error,
                detail,
            };
        }

        MobileTransportError::Http {
            status: resp.status,
            error: "http_error".to_owned(),
            detail: format!("unexpected HTTP status {}", resp.status),
        }
    }
}

impl MobileClientTransport for LiveHttpMobileTransport {
    fn request_sign_in_link(
        &mut self,
        email: &str,
        _now_secs: u64,
    ) -> Result<String, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({ "email": email }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::RequestLink.path(),
            None,
            Some(&body),
        )?;
        if resp.status == 200 {
            let parsed: RequestLinkResponse = resp.json()?;
            Ok(parsed.token)
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn verify_sign_in(
        &mut self,
        email: &str,
        token: &str,
        _now_secs: u64,
    ) -> Result<VerifiedMobileSession, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "email": email,
            "token": token,
        }))?;
        let resp =
            self.request_raw("POST", &MobileApiEndpoint::Verify.path(), None, Some(&body))?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn revoke_session(&mut self, session_id: &MobileSessionId) -> Result<(), MobileTransportError> {
        let _ = self.revoke_sessions(Some(session_id), &[])?;
        Ok(())
    }

    fn revoke_sessions(
        &mut self,
        active_session: Option<&MobileSessionId>,
        owed_sessions: &[MobileSessionId],
    ) -> Result<usize, MobileTransportError> {
        let body = if owed_sessions.is_empty() {
            None
        } else {
            Some(serde_json::to_vec(&serde_json::json!({
                "owed_sessions": owed_sessions,
            }))?)
        };
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::SignOut.path(),
            active_session,
            body.as_deref(),
        )?;
        if resp.status == 200 {
            let parsed: SignOutResponse = resp.json()?;
            Ok(parsed.revoked)
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn ingest_records(
        &mut self,
        session_id: &MobileSessionId,
        items: Vec<IncomingBatchItem>,
        _now_secs: u64,
    ) -> Result<SavepointBatchReport, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({ "items": items }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::IngestRecords.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_records(
        &self,
        session_id: &MobileSessionId,
        _now_secs: u64,
    ) -> Result<Vec<PatientRecord>, MobileTransportError> {
        let resp = self.request_raw(
            "GET",
            &MobileApiEndpoint::ListRecords.path(),
            Some(session_id),
            None,
        )?;
        if resp.status == 200 {
            let parsed: ListRecordsResponse = resp.json()?;
            Ok(parsed.records)
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn upload_health_batch(
        &mut self,
        session_id: &MobileSessionId,
        batch: AccountFencedHealthBatch,
        _now_secs: u64,
    ) -> Result<HealthUploadReceipt, MobileTransportError> {
        let body = serde_json::to_vec(&batch)?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::UploadHealth.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn fetch_health_trends(
        &self,
        session_id: &MobileSessionId,
        _now_secs: u64,
    ) -> Result<Vec<HealthSampleEntry>, MobileTransportError> {
        let resp = self.request_raw(
            "GET",
            &MobileApiEndpoint::HealthTrends.path(),
            Some(session_id),
            None,
        )?;
        if resp.status == 200 {
            let parsed: HealthTrendsResponse = resp.json()?;
            Ok(parsed.samples)
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn is_session_live(&self, session_id: &MobileSessionId, _now_secs: u64) -> bool {
        self.request_raw(
            "GET",
            &MobileApiEndpoint::Session.path(),
            Some(session_id),
            None,
        )
        .is_ok_and(|resp| resp.status == 200)
    }

    fn export_patient_data(
        &self,
        session_id: &MobileSessionId,
        _now_secs: u64,
    ) -> Result<PortablePatientExport, MobileTransportError> {
        let resp = self.request_raw(
            "GET",
            &MobileApiEndpoint::Export.path(),
            Some(session_id),
            None,
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn delete_account(
        &mut self,
        session_id: &MobileSessionId,
        event_id: &str,
        occurred_at: &str,
        _now_secs: u64,
    ) -> Result<AccountDeleteReceipt, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "occurred_at": occurred_at,
        }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::DeleteAccount.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_lookup(
        &self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        _now_secs: u64,
    ) -> Result<InviteChallenge, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({ "credential": credential }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::ConnectedCareLookup.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_verify_dob(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        dob: DateOfBirth,
        _now_secs: u64,
    ) -> Result<DobVerificationOutcome, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "credential": credential,
            "year": dob.year(),
            "month": dob.month(),
            "day": dob.day(),
        }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::ConnectedCareVerifyDob.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_share(
        &mut self,
        session_id: &MobileSessionId,
        credential: InviteCredential,
        _now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({ "credential": credential }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::ConnectedCareShare.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_transfer(
        &mut self,
        session_id: &MobileSessionId,
        idempotency_key: IdempotencyKey,
        connection_id: ConnectionId,
        items: Vec<EnvelopeItem>,
        part2_consent: Option<Part2WrittenConsent>,
        _now_secs: u64,
    ) -> Result<TransferEnvelope, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "idempotency_key": idempotency_key,
            "connection_id": connection_id,
            "items": items,
            "part2_consent": part2_consent,
        }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::ConnectedCareTransfer.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_disconnect(
        &mut self,
        session_id: &MobileSessionId,
        connection_id: ConnectionId,
        _now_secs: u64,
    ) -> Result<ConnectionStanding, MobileTransportError> {
        let body = serde_json::to_vec(&serde_json::json!({ "connection_id": connection_id }))?;
        let resp = self.request_raw(
            "POST",
            &MobileApiEndpoint::ConnectedCareDisconnect.path(),
            Some(session_id),
            Some(&body),
        )?;
        if resp.status == 200 {
            resp.json()
        } else {
            Err(Self::map_error_response(&resp))
        }
    }

    fn connected_care_status(
        &self,
        session_id: &MobileSessionId,
        pair: Option<&PairKey>,
        _now_secs: u64,
    ) -> Result<ConnectedCareStatusResult, MobileTransportError> {
        let mut target = MobileApiEndpoint::ConnectedCareStatus.path();
        if let Some(pair_key) = pair {
            target.push_str(&format!(
                "?practice_id={}&patient_id={}",
                pair_key.practice_id.as_str(),
                pair_key.practice_patient_id.as_str()
            ));
        }
        let resp = self.request_raw("GET", &target, Some(session_id), None)?;
        if resp.status != 200 {
            return Err(Self::map_error_response(&resp));
        }
        if pair.is_some() {
            let standing: ConnectionStanding = resp.json()?;
            Ok(ConnectedCareStatusResult::Pair(standing))
        } else {
            let active: ActiveConnectionsResponse = resp.json()?;
            Ok(ConnectedCareStatusResult::ActiveConnections(
                active.connections,
            ))
        }
    }
}

fn read_http1_response_bytes(
    stream: &mut TcpStream,
    target: &str,
) -> Result<Vec<u8>, MobileTransportError> {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut expected_content_len: Option<usize> = None;

    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if header_end.is_none()
                    && let Some(sep) = raw.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    header_end = Some(sep + 4);
                    if let Ok(head_str) = std::str::from_utf8(&raw[..sep]) {
                        for line in head_str.split("\r\n").skip(1) {
                            if let Some((name, value)) = line.split_once(':')
                                && name.trim().eq_ignore_ascii_case("content-length")
                                && let Ok(len) = value.trim().parse::<usize>()
                            {
                                expected_content_len = Some(len);
                                break;
                            }
                        }
                    }
                }
                if let (Some(body_start), Some(content_len)) = (header_end, expected_content_len)
                    && raw.len().saturating_sub(body_start) >= content_len
                {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::UnexpectedEof
                ) && header_end.is_some() =>
            {
                // On Windows MSVC Winsock2, peer close after sending the full body can surface
                // WSAECONNRESET instead of clean EOF. Accept when headers and body are present.
                let body_start = header_end.unwrap_or(raw.len());
                let body_complete = expected_content_len
                    .is_none_or(|len| raw.len().saturating_sub(body_start) >= len);
                if body_complete {
                    break;
                }
                return Err(MobileTransportError::Unavailable {
                    message: format!("failed to read HTTP response from {target}: {err}"),
                });
            }
            Err(err) => {
                return Err(MobileTransportError::Unavailable {
                    message: format!("failed to read HTTP response from {target}: {err}"),
                });
            }
        }
    }

    Ok(raw)
}

fn parse_http1_response(raw: &[u8]) -> Result<LiveMobileHttpResponse, MobileTransportError> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| MobileTransportError::Unavailable {
            message: "malformed HTTP/1.1 response missing header separator".to_owned(),
        })?;

    let head =
        std::str::from_utf8(&raw[..sep]).map_err(|err| MobileTransportError::Unavailable {
            message: format!("invalid UTF-8 HTTP response header: {err}"),
        })?;
    let body_slice = &raw[sep + 4..];

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| MobileTransportError::Unavailable {
            message: "missing HTTP status line".to_owned(),
        })?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| MobileTransportError::Unavailable {
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

    Ok(LiveMobileHttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked_body(mut input: &[u8]) -> Result<Vec<u8>, MobileTransportError> {
    let mut out = Vec::new();
    while let Some(crlf) = input.windows(2).position(|w| w == b"\r\n") {
        let size_str = std::str::from_utf8(&input[..crlf]).map_err(|err| {
            MobileTransportError::Unavailable {
                message: format!("invalid chunked length header: {err}"),
            }
        })?;
        let size_hex = size_str.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|err| {
            MobileTransportError::Unavailable {
                message: format!("invalid hex chunk length '{size_hex}': {err}"),
            }
        })?;
        input = &input[crlf + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size {
            return Err(MobileTransportError::Unavailable {
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
