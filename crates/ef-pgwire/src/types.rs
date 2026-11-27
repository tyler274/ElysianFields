//! Typed Extended Query parameter binding and row extraction.

use crate::error::PgError;

/// Typed parameter bound via PostgreSQL Extended Query (`Bind`) — zero SQL string interpolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgParam<'a> {
    Null,
    Text(&'a str),
    Int64(i64),
    Bool(bool),
    Bytes(&'a [u8]),
}

impl PgParam<'_> {
    /// Encodes the parameter into PostgreSQL text-format Bind bytes (`None` for SQL `NULL`).
    #[must_use]
    pub fn encode_text_value(&self) -> Option<Vec<u8>> {
        match self {
            Self::Null => None,
            Self::Text(s) => Some(s.as_bytes().to_vec()),
            Self::Int64(n) => Some(n.to_string().into_bytes()),
            Self::Bool(b) => Some(if *b {
                b"true".to_vec()
            } else {
                b"false".to_vec()
            }),
            Self::Bytes(bytes) => {
                let mut out = String::with_capacity(2 + bytes.len() * 2);
                out.push_str("\\x");
                for b in *bytes {
                    out.push_str(&format!("{b:02x}"));
                }
                Some(out.into_bytes())
            }
        }
    }
}

/// Column metadata received in a `RowDescription` (`T`) message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgColumn {
    pub name: String,
    pub type_oid: u32,
}

/// One decoded result row returned by [`crate::PgClient::query`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRow {
    columns: Vec<PgColumn>,
    values: Vec<Option<Vec<u8>>>,
}

impl PgRow {
    #[must_use]
    pub const fn new(columns: Vec<PgColumn>, values: Vec<Option<Vec<u8>>>) -> Self {
        Self { columns, values }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }

    /// Returns `true` if column `index` is SQL `NULL`.
    pub fn is_null(&self, index: usize) -> Result<bool, PgError> {
        let cell = self.values.get(index).ok_or(PgError::ColumnOutOfBounds {
            index,
            count: self.values.len(),
        })?;
        Ok(cell.is_none())
    }

    /// Extracts a UTF-8 text column value (`Ok(None)` for SQL `NULL`).
    pub fn get_text(&self, index: usize) -> Result<Option<String>, PgError> {
        let Some(raw) = self.raw_cell(index)? else {
            return Ok(None);
        };
        let text = String::from_utf8(raw.to_vec()).map_err(|err| PgError::TypeConversion {
            column: index,
            expected: "TEXT",
            detail: err.to_string(),
        })?;
        Ok(Some(text))
    }

    /// Extracts a non-null UTF-8 text column value.
    pub fn get_text_required(&self, index: usize) -> Result<String, PgError> {
        self.get_text(index)?
            .ok_or(PgError::UnexpectedNull { column: index })
    }

    /// Extracts an `INT8`/`INT4` integer column value (`Ok(None)` for SQL `NULL`).
    pub fn get_i64(&self, index: usize) -> Result<Option<i64>, PgError> {
        let Some(text) = self.get_text(index)? else {
            return Ok(None);
        };
        let val = text
            .trim()
            .parse::<i64>()
            .map_err(|err| PgError::TypeConversion {
                column: index,
                expected: "INT64",
                detail: format!("cannot parse '{text}': {err}"),
            })?;
        Ok(Some(val))
    }

    /// Extracts a `BOOL` column value (`Ok(None)` for SQL `NULL`).
    pub fn get_bool(&self, index: usize) -> Result<Option<bool>, PgError> {
        let Some(text) = self.get_text(index)? else {
            return Ok(None);
        };
        match text.trim() {
            "t" | "true" | "TRUE" | "1" => Ok(Some(true)),
            "f" | "false" | "FALSE" | "0" => Ok(Some(false)),
            other => Err(PgError::TypeConversion {
                column: index,
                expected: "BOOL",
                detail: format!("unexpected boolean literal '{other}'"),
            }),
        }
    }

    /// Extracts a `BYTEA` column value (`\x<hex>` text format decoded to raw bytes).
    pub fn get_bytes(&self, index: usize) -> Result<Option<Vec<u8>>, PgError> {
        let Some(raw) = self.raw_cell(index)? else {
            return Ok(None);
        };
        if let Some(hex_slice) = raw.strip_prefix(b"\\x") {
            let hex_str =
                std::str::from_utf8(hex_slice).map_err(|err| PgError::TypeConversion {
                    column: index,
                    expected: "BYTEA",
                    detail: err.to_string(),
                })?;
            let decoded = decode_hex(hex_str).map_err(|detail| PgError::TypeConversion {
                column: index,
                expected: "BYTEA",
                detail,
            })?;
            return Ok(Some(decoded));
        }
        Ok(Some(raw.to_vec()))
    }

    fn raw_cell(&self, index: usize) -> Result<Option<&[u8]>, PgError> {
        let cell = self.values.get(index).ok_or(PgError::ColumnOutOfBounds {
            index,
            count: self.values.len(),
        })?;
        Ok(cell.as_deref())
    }
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err("odd hex bytea length".to_owned());
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = hex_nibble(pair[0]).ok_or_else(|| "invalid hex digit".to_owned())?;
        let lo = hex_nibble(pair[1]).ok_or_else(|| "invalid hex digit".to_owned())?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
