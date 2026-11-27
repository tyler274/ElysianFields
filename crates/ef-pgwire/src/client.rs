//! Native TCP PostgreSQL Wire Protocol v3.0 client (`#[cfg(not(target_arch = "wasm32"))]`).

use std::collections::BTreeMap;
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::config::PgConnectionConfig;
use crate::crypto::{ScramSha256Client, generate_client_nonce};
use crate::error::PgError;
use crate::protocol::{
    AuthChallenge, BackendMessage, decode_backend_message, encode_extended_query_pipeline,
    encode_password_message, encode_sasl_initial_response, encode_sasl_response,
    encode_simple_query, encode_startup_message, encode_terminate, parse_command_tag_rows,
};
use crate::types::{PgColumn, PgParam, PgRow};

/// Synchronous pure-Rust PostgreSQL Wire Protocol v3.0 client over [`TcpStream`].
#[derive(Debug)]
pub struct PgClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    tx_status: u8,
    server_params: BTreeMap<String, String>,
}

impl PgClient {
    /// Connects to a PostgreSQL server and completes v3.0 startup + authentication
    /// (`trust`, `cleartext`, or RFC 5802 `SCRAM-SHA-256`).
    pub fn connect(config: &PgConnectionConfig) -> Result<Self, PgError> {
        let addr = format!("{}:{}", config.host, config.port);
        let stream = TcpStream::connect(&addr)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));

        let reader_stream = stream.try_clone()?;
        let mut client = Self {
            reader: BufReader::new(reader_stream),
            writer: stream,
            tx_status: b'I',
            server_params: BTreeMap::new(),
        };

        let startup = encode_startup_message(
            &config.user,
            &config.database,
            config.application_name.as_deref(),
        );
        client.write_all(&startup)?;

        let mut scram: Option<ScramSha256Client> = None;

        loop {
            match client.read_message()? {
                BackendMessage::Authentication(AuthChallenge::Ok) => {}
                BackendMessage::Authentication(AuthChallenge::CleartextPassword) => {
                    let password = config.password.as_deref().ok_or_else(|| PgError::Auth {
                        detail: "server requested cleartext password but none configured"
                            .to_owned(),
                    })?;
                    let msg = encode_password_message(password);
                    client.write_all(&msg)?;
                }
                BackendMessage::Authentication(AuthChallenge::Sasl { mechanisms }) => {
                    if !mechanisms.iter().any(|m| m == "SCRAM-SHA-256") {
                        return Err(PgError::Auth {
                            detail: format!(
                                "server SASL mechanisms {mechanisms:?} do not include SCRAM-SHA-256"
                            ),
                        });
                    }
                    let password = config.password.as_deref().ok_or_else(|| PgError::Auth {
                        detail: "server requested SCRAM-SHA-256 but no password configured"
                            .to_owned(),
                    })?;
                    let scram_state =
                        ScramSha256Client::new(&config.user, password, generate_client_nonce());
                    let first = scram_state.client_first_message();
                    let frame = encode_sasl_initial_response("SCRAM-SHA-256", first.as_bytes());
                    scram = Some(scram_state);
                    client.write_all(&frame)?;
                }
                BackendMessage::Authentication(AuthChallenge::SaslContinue { data }) => {
                    let server_first = std::str::from_utf8(&data).map_err(|err| PgError::Auth {
                        detail: format!("non-UTF-8 SCRAM server-first: {err}"),
                    })?;
                    let scram_state = scram.as_mut().ok_or_else(|| PgError::Auth {
                        detail: "unexpected SASLContinue before SASL negotiation".to_owned(),
                    })?;
                    let client_final = scram_state.process_server_first(server_first)?;
                    let frame = encode_sasl_response(client_final.as_bytes());
                    client.write_all(&frame)?;
                }
                BackendMessage::Authentication(AuthChallenge::SaslFinal { data }) => {
                    let server_final = std::str::from_utf8(&data).map_err(|err| PgError::Auth {
                        detail: format!("non-UTF-8 SCRAM server-final: {err}"),
                    })?;
                    let scram_state = scram.as_ref().ok_or_else(|| PgError::Auth {
                        detail: "unexpected SASLFinal before SASL negotiation".to_owned(),
                    })?;
                    scram_state.verify_server_final(server_final)?;
                }
                BackendMessage::Authentication(AuthChallenge::Unsupported { code }) => {
                    return Err(PgError::Auth {
                        detail: format!("unsupported PostgreSQL auth scheme code {code}"),
                    });
                }
                BackendMessage::ParameterStatus { name, value } => {
                    client.server_params.insert(name, value);
                }
                BackendMessage::BackendKeyData { .. }
                | BackendMessage::NoticeResponse { .. }
                | BackendMessage::Ignored { .. } => {}
                BackendMessage::ReadyForQuery { tx_status } => {
                    client.tx_status = tx_status;
                    break;
                }
                BackendMessage::ErrorResponse {
                    severity,
                    code,
                    message,
                    detail,
                } => {
                    return Err(PgError::Server {
                        severity,
                        code,
                        message,
                        detail,
                    });
                }
                other => {
                    return Err(PgError::Protocol {
                        detail: format!("unexpected message during startup: {other:?}"),
                    });
                }
            }
        }

        Ok(client)
    }

    /// Returns server-reported runtime parameters (e.g. `server_version`).
    #[must_use]
    pub fn server_parameter(&self, name: &str) -> Option<&str> {
        self.server_params.get(name).map(String::as_str)
    }

    /// Executes a lightweight parameterized `SELECT 1` liveness probe against PostgreSQL.
    pub fn ping(&mut self) -> Result<(), PgError> {
        let rows = self.query("SELECT 1", &[])?;
        let ok = rows.first().map(|r| r.get_i64(0)).transpose()?.flatten() == Some(1);
        if ok {
            Ok(())
        } else {
            Err(PgError::Protocol {
                detail: "SELECT 1 liveness probe returned unexpected result".to_owned(),
            })
        }
    }

    /// Attempts to acquire a session-scoped PostgreSQL advisory lock without blocking
    /// (`SELECT pg_try_advisory_lock($1)` via Extended Query — zero SQL string splicing).
    pub fn try_advisory_lock(&mut self, lock_key: i64) -> Result<bool, PgError> {
        let rows = self.query(
            "SELECT pg_try_advisory_lock($1)",
            &[PgParam::Int64(lock_key)],
        )?;
        rows.first()
            .ok_or_else(|| PgError::Protocol {
                detail: "pg_try_advisory_lock returned zero rows".to_owned(),
            })?
            .get_bool(0)?
            .ok_or(PgError::UnexpectedNull { column: 0 })
    }

    /// Acquires a session-scoped exclusive PostgreSQL advisory lock, waiting if held by a peer
    /// (`SELECT pg_advisory_lock($1)` via Extended Query — zero SQL string splicing).
    pub fn advisory_lock(&mut self, lock_key: i64) -> Result<(), PgError> {
        let _rows = self.query("SELECT pg_advisory_lock($1)", &[PgParam::Int64(lock_key)])?;
        Ok(())
    }

    /// Releases a previously acquired session-scoped PostgreSQL advisory lock
    /// (`SELECT pg_advisory_unlock($1)` via Extended Query — zero SQL string splicing).
    pub fn advisory_unlock(&mut self, lock_key: i64) -> Result<bool, PgError> {
        let rows = self.query("SELECT pg_advisory_unlock($1)", &[PgParam::Int64(lock_key)])?;
        rows.first()
            .ok_or_else(|| PgError::Protocol {
                detail: "pg_advisory_unlock returned zero rows".to_owned(),
            })?
            .get_bool(0)?
            .ok_or(PgError::UnexpectedNull { column: 0 })
    }

    /// Executes a parameterized Extended Query returning all result rows.
    pub fn query(&mut self, sql: &str, params: &[PgParam<'_>]) -> Result<Vec<PgRow>, PgError> {
        let (rows, _) = self.run_extended(sql, params)?;
        Ok(rows)
    }

    /// Executes a parameterized Extended Query returning the affected row count.
    pub fn execute(&mut self, sql: &str, params: &[PgParam<'_>]) -> Result<u64, PgError> {
        let (_, affected) = self.run_extended(sql, params)?;
        Ok(affected)
    }

    /// Executes a multi-statement idempotent DDL migration script via Simple Query (`'Q'`).
    pub fn execute_batch_ddl(&mut self, sql: &str) -> Result<(), PgError> {
        let frame = encode_simple_query(sql);
        self.write_all(&frame)?;

        let mut first_error: Option<PgError> = None;
        loop {
            match self.read_message()? {
                BackendMessage::ReadyForQuery { tx_status } => {
                    self.tx_status = tx_status;
                    break;
                }
                BackendMessage::ErrorResponse {
                    severity,
                    code,
                    message,
                    detail,
                } => {
                    let _ = first_error.get_or_insert(PgError::Server {
                        severity,
                        code,
                        message,
                        detail,
                    });
                }
                _ => {}
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(())
    }

    /// Begins an explicit SQL transaction (`BEGIN`).
    pub fn begin(&mut self) -> Result<(), PgError> {
        let _ = self.execute("BEGIN", &[])?;
        Ok(())
    }

    /// Commits the current SQL transaction (`COMMIT`).
    pub fn commit(&mut self) -> Result<(), PgError> {
        let _ = self.execute("COMMIT", &[])?;
        Ok(())
    }

    /// Rolls back the current SQL transaction (`ROLLBACK`).
    pub fn rollback(&mut self) -> Result<(), PgError> {
        let _ = self.execute("ROLLBACK", &[])?;
        Ok(())
    }

    /// Runs `f` inside an explicit transaction, committing on `Ok` and rolling back on `Err`.
    pub fn in_transaction<T, F>(&mut self, f: F) -> Result<T, PgError>
    where
        F: FnOnce(&mut Self) -> Result<T, PgError>,
    {
        self.begin()?;
        match f(self) {
            Ok(val) => {
                self.commit()?;
                Ok(val)
            }
            Err(err) => {
                let _ = self.rollback();
                Err(err)
            }
        }
    }

    fn run_extended(
        &mut self,
        sql: &str,
        params: &[PgParam<'_>],
    ) -> Result<(Vec<PgRow>, u64), PgError> {
        let pipeline = encode_extended_query_pipeline(sql, params)?;
        self.write_all(&pipeline)?;

        let mut columns: Vec<PgColumn> = Vec::new();
        let mut rows: Vec<PgRow> = Vec::new();
        let mut affected = 0u64;
        let mut first_error: Option<PgError> = None;

        loop {
            match self.read_message()? {
                BackendMessage::RowDescription(cols) => columns = cols,
                BackendMessage::DataRow(values) => {
                    rows.push(PgRow::new(columns.clone(), values));
                }
                BackendMessage::CommandComplete { tag } => {
                    affected = parse_command_tag_rows(&tag);
                }
                BackendMessage::ErrorResponse {
                    severity,
                    code,
                    message,
                    detail,
                } => {
                    let _ = first_error.get_or_insert(PgError::Server {
                        severity,
                        code,
                        message,
                        detail,
                    });
                }
                BackendMessage::ReadyForQuery { tx_status } => {
                    self.tx_status = tx_status;
                    break;
                }
                BackendMessage::ParseComplete
                | BackendMessage::BindComplete
                | BackendMessage::NoData
                | BackendMessage::EmptyQueryResponse
                | BackendMessage::PortalSuspended
                | BackendMessage::NoticeResponse { .. }
                | BackendMessage::ParameterStatus { .. }
                | BackendMessage::BackendKeyData { .. }
                | BackendMessage::Authentication(_)
                | BackendMessage::Ignored { .. } => {}
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        Ok((rows, affected))
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), PgError> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<BackendMessage, PgError> {
        let mut header = [0u8; 5];
        self.reader.read_exact(&mut header)?;
        let msg_type = header[0];
        let total_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        if total_len < 4 {
            return Err(PgError::Protocol {
                detail: format!("invalid PostgreSQL frame length {total_len}"),
            });
        }
        let body_len = (total_len - 4) as usize;
        let mut body = vec![0u8; body_len];
        if body_len > 0 {
            self.reader.read_exact(&mut body)?;
        }
        decode_backend_message(msg_type, &body)
    }
}

impl Drop for PgClient {
    fn drop(&mut self) {
        let _ = self.writer.write_all(&encode_terminate());
        let _ = self.writer.flush();
    }
}
