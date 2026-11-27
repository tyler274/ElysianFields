//! PostgreSQL Wire Protocol v3.0 frame encoders and decoders.
//!
//! Pure byte framing that cross-compiles cleanly on both native hosts and `wasm32-unknown-unknown`.

use crate::error::PgError;
use crate::types::{PgColumn, PgParam};

/// PostgreSQL Wire Protocol v3.0 version number (`(3 << 16) | 0 = 196608`).
pub const PG_PROTOCOL_V3: u32 = 196_608;

/// Decoded authentication challenge received in an `'R'` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthChallenge {
    Ok,
    CleartextPassword,
    Sasl { mechanisms: Vec<String> },
    SaslContinue { data: Vec<u8> },
    SaslFinal { data: Vec<u8> },
    Unsupported { code: i32 },
}

/// Decoded backend message received from a PostgreSQL v3.0 server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendMessage {
    Authentication(AuthChallenge),
    ParameterStatus {
        name: String,
        value: String,
    },
    BackendKeyData {
        pid: i32,
        secret_key: i32,
    },
    ReadyForQuery {
        tx_status: u8,
    },
    RowDescription(Vec<PgColumn>),
    DataRow(Vec<Option<Vec<u8>>>),
    CommandComplete {
        tag: String,
    },
    EmptyQueryResponse,
    ParseComplete,
    BindComplete,
    NoData,
    PortalSuspended,
    ErrorResponse {
        severity: String,
        code: String,
        message: String,
        detail: Option<String>,
    },
    NoticeResponse {
        message: String,
    },
    Ignored {
        msg_type: u8,
    },
}

/// Encodes the initial v3.0 `StartupMessage` (no leading message-type byte).
#[must_use]
pub fn encode_startup_message(user: &str, database: &str, app_name: Option<&str>) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&PG_PROTOCOL_V3.to_be_bytes());
    write_cstr(&mut body, "user");
    write_cstr(&mut body, user);
    write_cstr(&mut body, "database");
    write_cstr(&mut body, database);
    write_cstr(&mut body, "client_encoding");
    write_cstr(&mut body, "UTF8");
    if let Some(app) = app_name {
        write_cstr(&mut body, "application_name");
        write_cstr(&mut body, app);
    }
    body.push(0);

    let total_len = (body.len() + 4) as u32;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&total_len.to_be_bytes());
    frame.extend_from_slice(&body);
    frame
}

/// Encodes a cleartext `PasswordMessage` (`'p'`).
#[must_use]
pub fn encode_password_message(password: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(password.len() + 1);
    write_cstr(&mut body, password);
    wrap_frontend_message(b'p', &body)
}

/// Encodes a `SASLInitialResponse` (`'p'`).
#[must_use]
pub fn encode_sasl_initial_response(mechanism: &str, initial_data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(mechanism.len() + 1 + 4 + initial_data.len());
    write_cstr(&mut body, mechanism);
    body.extend_from_slice(&(initial_data.len() as i32).to_be_bytes());
    body.extend_from_slice(initial_data);
    wrap_frontend_message(b'p', &body)
}

/// Encodes a subsequent `SASLResponse` (`'p'`).
#[must_use]
pub fn encode_sasl_response(data: &[u8]) -> Vec<u8> {
    wrap_frontend_message(b'p', data)
}

/// Encodes a Simple Query (`'Q'`) message (used for multi-statement idempotent DDL scripts).
#[must_use]
pub fn encode_simple_query(sql: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(sql.len() + 1);
    write_cstr(&mut body, sql);
    wrap_frontend_message(b'Q', &body)
}

/// Encodes a complete Extended Query pipeline (`Parse` + `Bind` + `Describe` + `Execute` + `Sync`)
/// binding all parameters out-of-band with zero SQL string interpolation.
pub fn encode_extended_query_pipeline(
    sql: &str,
    params: &[PgParam<'_>],
) -> Result<Vec<u8>, PgError> {
    let param_count = i16::try_from(params.len()).map_err(|err| PgError::Protocol {
        detail: format!("too many query parameters ({}): {err}", params.len()),
    })?;

    let mut out = Vec::new();

    // 1. Parse ('P'): unnamed statement "", SQL, 0 explicit param type OIDs (inferred by server).
    let mut parse_body = Vec::with_capacity(sql.len() + 4);
    parse_body.push(0); // statement name ""
    write_cstr(&mut parse_body, sql);
    parse_body.extend_from_slice(&0i16.to_be_bytes());
    out.extend_from_slice(&wrap_frontend_message(b'P', &parse_body));

    // 2. Bind ('B'): unnamed portal "", unnamed statement "", all-text param & result formats.
    let mut bind_body = Vec::new();
    bind_body.push(0); // portal ""
    bind_body.push(0); // statement ""
    bind_body.extend_from_slice(&0i16.to_be_bytes()); // 0 parameter format codes -> all text
    bind_body.extend_from_slice(&param_count.to_be_bytes());
    for param in params {
        match param.encode_text_value() {
            None => bind_body.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(bytes) => {
                let len = i32::try_from(bytes.len()).map_err(|err| PgError::Protocol {
                    detail: format!("parameter payload exceeds i32::MAX: {err}"),
                })?;
                bind_body.extend_from_slice(&len.to_be_bytes());
                bind_body.extend_from_slice(&bytes);
            }
        }
    }
    bind_body.extend_from_slice(&0i16.to_be_bytes()); // 0 result format codes -> all text
    out.extend_from_slice(&wrap_frontend_message(b'B', &bind_body));

    // 3. Describe ('D'): portal ""
    out.extend_from_slice(&wrap_frontend_message(b'D', b"P\0"));

    // 4. Execute ('E'): portal "", max_rows = 0 (unlimited)
    out.extend_from_slice(&wrap_frontend_message(b'E', b"\0\0\0\0\0"));

    // 5. Sync ('S')
    out.extend_from_slice(&wrap_frontend_message(b'S', &[]));

    Ok(out)
}

/// Encodes a graceful `Terminate` (`'X'`) frame.
#[must_use]
pub fn encode_terminate() -> Vec<u8> {
    wrap_frontend_message(b'X', &[])
}

/// Decodes a single PostgreSQL v3.0 backend message given its type byte and body payload.
pub fn decode_backend_message(msg_type: u8, payload: &[u8]) -> Result<BackendMessage, PgError> {
    match msg_type {
        b'R' => decode_auth(payload),
        b'S' => {
            let (name, rest) = read_cstr(payload)?;
            let (value, _) = read_cstr(rest)?;
            Ok(BackendMessage::ParameterStatus { name, value })
        }
        b'K' => {
            if payload.len() < 8 {
                return Err(PgError::Protocol {
                    detail: "truncated BackendKeyData".to_owned(),
                });
            }
            let pid = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let secret_key = i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
            Ok(BackendMessage::BackendKeyData { pid, secret_key })
        }
        b'Z' => {
            let &tx_status = payload.first().ok_or_else(|| PgError::Protocol {
                detail: "truncated ReadyForQuery".to_owned(),
            })?;
            Ok(BackendMessage::ReadyForQuery { tx_status })
        }
        b'T' => decode_row_description(payload),
        b'D' => decode_data_row(payload),
        b'C' => {
            let (tag, _) = read_cstr(payload)?;
            Ok(BackendMessage::CommandComplete { tag })
        }
        b'I' => Ok(BackendMessage::EmptyQueryResponse),
        b'1' => Ok(BackendMessage::ParseComplete),
        b'2' => Ok(BackendMessage::BindComplete),
        b'n' => Ok(BackendMessage::NoData),
        b's' => Ok(BackendMessage::PortalSuspended),
        b'E' => decode_error_or_notice(payload, true),
        b'N' => decode_error_or_notice(payload, false),
        other => Ok(BackendMessage::Ignored { msg_type: other }),
    }
}

/// Parses affected row counts from a PostgreSQL `CommandComplete` tag (e.g. `INSERT 0 1`, `UPDATE 3`).
#[must_use]
pub fn parse_command_tag_rows(tag: &str) -> u64 {
    tag.split_whitespace()
        .last()
        .and_then(|tok| tok.parse::<u64>().ok())
        .unwrap_or(0)
}

fn decode_auth(payload: &[u8]) -> Result<BackendMessage, PgError> {
    if payload.len() < 4 {
        return Err(PgError::Protocol {
            detail: "truncated Authentication message".to_owned(),
        });
    }
    let code = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let rest = &payload[4..];
    let challenge = match code {
        0 => AuthChallenge::Ok,
        3 => AuthChallenge::CleartextPassword,
        10 => {
            let mut mechanisms = Vec::new();
            let mut cur = rest;
            while !cur.is_empty() && cur[0] != 0 {
                let (mech, next) = read_cstr(cur)?;
                mechanisms.push(mech);
                cur = next;
            }
            AuthChallenge::Sasl { mechanisms }
        }
        11 => AuthChallenge::SaslContinue {
            data: rest.to_vec(),
        },
        12 => AuthChallenge::SaslFinal {
            data: rest.to_vec(),
        },
        other => AuthChallenge::Unsupported { code: other },
    };
    Ok(BackendMessage::Authentication(challenge))
}

fn decode_row_description(payload: &[u8]) -> Result<BackendMessage, PgError> {
    if payload.len() < 2 {
        return Err(PgError::Protocol {
            detail: "truncated RowDescription".to_owned(),
        });
    }
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut cur = &payload[2..];
    let mut columns = Vec::with_capacity(count);
    for _ in 0..count {
        let (name, after_name) = read_cstr(cur)?;
        if after_name.len() < 18 {
            return Err(PgError::Protocol {
                detail: "truncated RowDescription column metadata".to_owned(),
            });
        }
        // Layout after name: table_oid(4), col_attr(2), type_oid(4), type_size(2), type_mod(4), fmt(2)
        let type_oid =
            u32::from_be_bytes([after_name[6], after_name[7], after_name[8], after_name[9]]);
        columns.push(PgColumn { name, type_oid });
        cur = &after_name[18..];
    }
    Ok(BackendMessage::RowDescription(columns))
}

fn decode_data_row(payload: &[u8]) -> Result<BackendMessage, PgError> {
    if payload.len() < 2 {
        return Err(PgError::Protocol {
            detail: "truncated DataRow".to_owned(),
        });
    }
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut cur = &payload[2..];
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        if cur.len() < 4 {
            return Err(PgError::Protocol {
                detail: "truncated DataRow column length".to_owned(),
            });
        }
        let len = i32::from_be_bytes([cur[0], cur[1], cur[2], cur[3]]);
        cur = &cur[4..];
        if len < 0 {
            values.push(None);
        } else {
            let usize_len = len as usize;
            if cur.len() < usize_len {
                return Err(PgError::Protocol {
                    detail: "truncated DataRow column value".to_owned(),
                });
            }
            values.push(Some(cur[..usize_len].to_vec()));
            cur = &cur[usize_len..];
        }
    }
    Ok(BackendMessage::DataRow(values))
}

fn decode_error_or_notice(payload: &[u8], is_error: bool) -> Result<BackendMessage, PgError> {
    let mut severity = String::new();
    let mut code = String::new();
    let mut message = String::new();
    let mut detail = None;

    let mut cur = payload;
    while !cur.is_empty() && cur[0] != 0 {
        let field_type = cur[0];
        let (val, next) = read_cstr(&cur[1..])?;
        match field_type {
            b'V' | b'S' => {
                if severity.is_empty() || field_type == b'V' {
                    severity = val;
                }
            }
            b'C' => code = val,
            b'M' => message = val,
            b'D' => detail = Some(val),
            _ => {}
        }
        cur = next;
    }

    if is_error {
        Ok(BackendMessage::ErrorResponse {
            severity,
            code,
            message,
            detail,
        })
    } else {
        Ok(BackendMessage::NoticeResponse { message })
    }
}

fn wrap_frontend_message(msg_type: u8, body: &[u8]) -> Vec<u8> {
    let total_len = (body.len() + 4) as u32;
    let mut frame = Vec::with_capacity(1 + 4 + body.len());
    frame.push(msg_type);
    frame.extend_from_slice(&total_len.to_be_bytes());
    frame.extend_from_slice(body);
    frame
}

fn write_cstr(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
}

fn read_cstr(input: &[u8]) -> Result<(String, &[u8]), PgError> {
    let nul_pos = input
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| PgError::Protocol {
            detail: "unterminated C-string in PostgreSQL wire message".to_owned(),
        })?;
    let text = String::from_utf8_lossy(&input[..nul_pos]).into_owned();
    Ok((text, &input[nul_pos + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_and_extended_query_frames_are_well_formed() {
        let startup = encode_startup_message("ef_user", "ef_db", Some("ef-test"));
        let declared_len = u32::from_be_bytes([startup[0], startup[1], startup[2], startup[3]]);
        assert_eq!(declared_len as usize, startup.len());
        let proto = u32::from_be_bytes([startup[4], startup[5], startup[6], startup[7]]);
        assert_eq!(proto, PG_PROTOCOL_V3);

        let pipeline = encode_extended_query_pipeline(
            "SELECT $1, $2, $3, $4, $5",
            &[
                PgParam::Text("'; DROP TABLE users; --"),
                PgParam::Int64(42),
                PgParam::Bool(true),
                PgParam::Bytes(b"\x00\x01\xff"),
                PgParam::Null,
            ],
        )
        .expect("encode pipeline");
        assert_eq!(pipeline[0], b'P');
        assert!(pipeline.ends_with(&[b'S', 0, 0, 0, 4]));
    }

    #[test]
    fn parse_command_tag_extracts_row_counts() {
        assert_eq!(parse_command_tag_rows("INSERT 0 1"), 1);
        assert_eq!(parse_command_tag_rows("UPDATE 17"), 17);
        assert_eq!(parse_command_tag_rows("CREATE TABLE"), 0);
    }
}
