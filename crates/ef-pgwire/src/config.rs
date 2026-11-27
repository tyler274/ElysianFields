//! Typed PostgreSQL connection configuration with redacted `Debug` output.

use std::fmt;

use crate::error::PgError;

/// Parsed PostgreSQL connection parameters (`postgres://user:pass@host:port/db`).
#[derive(Clone, PartialEq, Eq)]
pub struct PgConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
    pub application_name: Option<String>,
}

impl fmt::Debug for PgConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("database", &self.database)
            .field("application_name", &self.application_name)
            .finish()
    }
}

impl PgConnectionConfig {
    /// Parses a `postgres://` or `postgresql://` connection URL into typed parameters.
    pub fn parse_url(raw: &str) -> Result<Self, PgError> {
        let trimmed = raw.trim();
        let rest = trimmed
            .strip_prefix("postgres://")
            .or_else(|| trimmed.strip_prefix("postgresql://"))
            .ok_or_else(|| PgError::InvalidUrl {
                detail: "URL must start with postgres:// or postgresql://".to_owned(),
            })?;

        let (before_query, query_opt) = match rest.split_once('?') {
            Some((a, b)) => (a, Some(b)),
            None => (rest, None),
        };

        let (authority, raw_db) = match before_query.split_once('/') {
            Some((auth, db)) => (auth, db),
            None => (before_query, ""),
        };

        let (userinfo_opt, hostport) = match authority.rsplit_once('@') {
            Some((u, hp)) => (Some(u), hp),
            None => (None, authority),
        };

        let (user, password) = match userinfo_opt {
            Some(ui) => match ui.split_once(':') {
                Some((u, p)) => (percent_decode(u)?, Some(percent_decode(p)?)),
                None => (percent_decode(ui)?, None),
            },
            None => ("postgres".to_owned(), None),
        };

        if user.is_empty() {
            return Err(PgError::InvalidUrl {
                detail: "username must not be empty".to_owned(),
            });
        }

        let (host, port) = parse_host_port(hostport)?;
        let database = if raw_db.is_empty() {
            user.clone()
        } else {
            percent_decode(raw_db)?
        };

        let mut application_name = None;
        if let Some(query) = query_opt {
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=')
                    && k == "application_name"
                    && !v.is_empty()
                {
                    application_name = Some(percent_decode(v)?);
                }
            }
        }

        Ok(Self {
            host,
            port,
            user,
            password,
            database,
            application_name,
        })
    }
}

fn parse_host_port(raw: &str) -> Result<(String, u16), PgError> {
    if raw.is_empty() {
        return Ok(("127.0.0.1".to_owned(), 5432));
    }
    if let Some(after_bracket) = raw.strip_prefix('[') {
        let Some((ipv6, remainder)) = after_bracket.split_once(']') else {
            return Err(PgError::InvalidUrl {
                detail: "unterminated IPv6 host bracket".to_owned(),
            });
        };
        if ipv6.is_empty() {
            return Err(PgError::InvalidUrl {
                detail: "empty IPv6 host".to_owned(),
            });
        }
        let port = if remainder.is_empty() {
            5432
        } else if let Some(port_str) = remainder.strip_prefix(':') {
            port_str.parse::<u16>().map_err(|err| PgError::InvalidUrl {
                detail: format!("invalid port '{port_str}': {err}"),
            })?
        } else {
            return Err(PgError::InvalidUrl {
                detail: format!("unexpected trailing host characters '{remainder}'"),
            });
        };
        return Ok((ipv6.to_owned(), port));
    }

    match raw.rsplit_once(':') {
        Some((h, port_str)) => {
            let host = if h.is_empty() { "127.0.0.1" } else { h };
            let port = port_str.parse::<u16>().map_err(|err| PgError::InvalidUrl {
                detail: format!("invalid port '{port_str}': {err}"),
            })?;
            Ok((host.to_owned(), port))
        }
        None => Ok((raw.to_owned(), 5432)),
    }
}

fn percent_decode(raw: &str) -> Result<String, PgError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(PgError::InvalidUrl {
                    detail: "truncated percent-encoding".to_owned(),
                });
            }
            let hi = hex_nibble(bytes[i + 1]).ok_or_else(|| PgError::InvalidUrl {
                detail: "invalid hex escape in URL".to_owned(),
            })?;
            let lo = hex_nibble(bytes[i + 2]).ok_or_else(|| PgError::InvalidUrl {
                detail: "invalid hex escape in URL".to_owned(),
            })?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|err| PgError::InvalidUrl {
        detail: format!("URL component is not valid UTF-8: {err}"),
    })
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_extracts_components_and_redacts_password_in_debug() {
        let cfg = PgConnectionConfig::parse_url(
            "postgres://ef_user:s3cr%40t@127.0.0.1:5439/ef_clinic?application_name=clinic-srv",
        )
        .expect("valid url");
        assert_eq!(cfg.user, "ef_user");
        assert_eq!(cfg.password.as_deref(), Some("s3cr@t"));
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 5439);
        assert_eq!(cfg.database, "ef_clinic");
        assert_eq!(cfg.application_name.as_deref(), Some("clinic-srv"));

        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("s3cr@t"));
    }

    #[test]
    fn parse_url_supports_postgresql_scheme_defaults_and_ipv6() {
        let cfg = PgConnectionConfig::parse_url("postgresql://alice@[::1]/mydb").expect("ipv6 url");
        assert_eq!(cfg.user, "alice");
        assert_eq!(cfg.password, None);
        assert_eq!(cfg.host, "::1");
        assert_eq!(cfg.port, 5432);
        assert_eq!(cfg.database, "mydb");
    }
}
