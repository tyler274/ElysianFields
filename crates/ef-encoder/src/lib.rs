//! Deterministic offline semantic encoder for local clinical chart retrieval.
//!
//! Computes fixed-dimension L2-normalized embeddings strictly locally with zero
//! external API calls, zero rendering dependencies, and workspace-scoped search isolation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default fixed embedding dimension for offline semantic chart retrieval.
pub const DEFAULT_EMBEDDING_DIM: usize = 256;

/// Deterministic encoder model identifier stamped into persisted indexes.
pub const ENCODER_MODEL_VERSION: &str = "ef-offline-hash-v1";

/// Typed errors returned by the offline semantic encoder and index.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncoderError {
    #[error("embedding dimension must be at least 16 (got {dim})")]
    InvalidDimension { dim: usize },
    #[error("vector dimension mismatch ({left} != {right})")]
    DimensionMismatch { left: usize, right: usize },
    #[error("document scope identifier must not be empty")]
    EmptyScopeId,
    #[error("document identifier must not be empty")]
    EmptyDocumentId,
}

/// Fixed-dimension L2-normalized clinical text embedding vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingVector {
    dim: usize,
    values: Vec<f32>,
}

impl EmbeddingVector {
    /// Constructs an L2-normalized embedding from raw accumulators (zero vector if all zeros).
    #[must_use]
    pub fn from_raw_normalized(mut values: Vec<f32>) -> Self {
        let dim = values.len();
        let norm_sq: f64 = values.iter().map(|&v| f64::from(v) * f64::from(v)).sum();
        if norm_sq > 0.0 {
            let inv_norm = (1.0 / norm_sq.sqrt()) as f32;
            for v in &mut values {
                *v *= inv_norm;
            }
        }
        Self { dim, values }
    }

    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }

    #[must_use]
    pub fn as_slice(&self) -> &[f32] {
        &self.values
    }

    /// Computes Euclidean L2 norm (`1.0` for any non-empty encoded text, `0.0` for empty input).
    #[must_use]
    pub fn l2_norm(&self) -> f32 {
        let sum_sq: f64 = self
            .values
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum();
        sum_sq.sqrt() as f32
    }

    /// Computes cosine similarity in `[-1.0, 1.0]` between two same-dimension embeddings.
    pub fn cosine_similarity(&self, other: &Self) -> Result<f32, EncoderError> {
        if self.dim != other.dim {
            return Err(EncoderError::DimensionMismatch {
                left: self.dim,
                right: other.dim,
            });
        }
        let dot: f64 = self
            .values
            .iter()
            .zip(other.values.iter())
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum();
        let norm_a = f64::from(self.l2_norm());
        let norm_b = f64::from(other.l2_norm());
        if norm_a == 0.0 || norm_b == 0.0 {
            return Ok(0.0);
        }
        Ok((dot / (norm_a * norm_b)).clamp(-1.0, 1.0) as f32)
    }
}

/// Deterministic offline semantic encoder combining clinical word unigrams,
/// word bigrams, and character trigrams into a signed random-projection hash embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineSemanticEncoder {
    dim: usize,
}

impl Default for OfflineSemanticEncoder {
    fn default() -> Self {
        Self {
            dim: DEFAULT_EMBEDDING_DIM,
        }
    }
}

impl OfflineSemanticEncoder {
    pub fn new(dim: usize) -> Result<Self, EncoderError> {
        if dim < 16 {
            return Err(EncoderError::InvalidDimension { dim });
        }
        Ok(Self { dim })
    }

    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }

    #[must_use]
    pub const fn model_version(&self) -> &'static str {
        ENCODER_MODEL_VERSION
    }

    /// Encodes a clinical text snippet locally into an L2-normalized fixed-dimension vector.
    #[must_use]
    pub fn encode(&self, text: &str) -> EmbeddingVector {
        let mut accum = vec![0.0_f32; self.dim];
        let tokens = tokenize_clinical(text);
        if tokens.is_empty() {
            return EmbeddingVector::from_raw_normalized(accum);
        }

        // 1. Word unigrams (strongest weight for clinical terms & lab names).
        for tok in &tokens {
            project_feature(&mut accum, b"u:", tok.as_bytes(), 2.0);
            // 2. Character trigrams capture clinical prefixes/stems (e.g. nephro-, creatinin-).
            let chars: Vec<char> = tok.chars().collect();
            if chars.len() >= 3 {
                let mut tri = String::with_capacity(12);
                for win in chars.windows(3) {
                    tri.clear();
                    for &c in win {
                        tri.push(c);
                    }
                    project_feature(&mut accum, b"g:", tri.as_bytes(), 0.75);
                }
            }
        }

        // 3. Adjacent word bigrams capture multi-word clinical phrases (e.g. "blood pressure").
        for pair in tokens.windows(2) {
            let mut bigram = String::with_capacity(pair[0].len() + pair[1].len() + 1);
            bigram.push_str(&pair[0]);
            bigram.push('_');
            bigram.push_str(&pair[1]);
            project_feature(&mut accum, b"b:", bigram.as_bytes(), 1.25);
        }

        EmbeddingVector::from_raw_normalized(accum)
    }

    /// Batch-encodes a slice of clinical snippets deterministically.
    #[must_use]
    pub fn encode_batch(&self, texts: &[&str]) -> Vec<EmbeddingVector> {
        texts.iter().map(|t| self.encode(t)).collect()
    }
}

/// Entry stored inside [`SemanticChartIndex`], strictly isolated by `scope_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedDocument {
    pub doc_id: String,
    pub scope_id: String,
    pub embedding: EmbeddingVector,
}

/// Ranked semantic search result returned by [`SemanticChartIndex::search`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticSearchHit {
    pub doc_id: String,
    pub scope_id: String,
    pub score: f32,
}

/// Local batch semantic index supporting workspace-scoped cosine similarity retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticChartIndex {
    dim: usize,
    model_version: String,
    scopes: BTreeMap<String, BTreeMap<String, IndexedDocument>>,
}

impl Default for SemanticChartIndex {
    fn default() -> Self {
        Self::new(DEFAULT_EMBEDDING_DIM).expect("default dim valid")
    }
}

impl SemanticChartIndex {
    pub fn new(dim: usize) -> Result<Self, EncoderError> {
        if dim < 16 {
            return Err(EncoderError::InvalidDimension { dim });
        }
        Ok(Self {
            dim,
            model_version: ENCODER_MODEL_VERSION.to_owned(),
            scopes: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn dim(&self) -> usize {
        self.dim
    }

    #[must_use]
    pub fn model_version(&self) -> &str {
        &self.model_version
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.scopes.values().map(BTreeMap::len).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Encodes and inserts a single document strictly scoped to `scope_id`.
    pub fn insert_document(
        &mut self,
        encoder: &OfflineSemanticEncoder,
        scope_id: &str,
        doc_id: &str,
        text: &str,
    ) -> Result<(), EncoderError> {
        if scope_id.trim().is_empty() {
            return Err(EncoderError::EmptyScopeId);
        }
        if doc_id.trim().is_empty() {
            return Err(EncoderError::EmptyDocumentId);
        }
        if encoder.dim() != self.dim {
            return Err(EncoderError::DimensionMismatch {
                left: self.dim,
                right: encoder.dim(),
            });
        }
        let embedding = encoder.encode(text);
        self.scopes.entry(scope_id.to_owned()).or_default().insert(
            doc_id.to_owned(),
            IndexedDocument {
                doc_id: doc_id.to_owned(),
                scope_id: scope_id.to_owned(),
                embedding,
            },
        );
        Ok(())
    }

    /// Encodes and inserts a batch of `(scope_id, doc_id, text)` records.
    pub fn insert_batch(
        &mut self,
        encoder: &OfflineSemanticEncoder,
        items: &[(&str, &str, &str)],
    ) -> Result<usize, EncoderError> {
        let mut count = 0usize;
        for &(scope_id, doc_id, text) in items {
            self.insert_document(encoder, scope_id, doc_id, text)?;
            count += 1;
        }
        Ok(count)
    }

    /// Searches strictly within `scope_id` (enforcing zero cross-patient leakage), returning
    /// top `k` hits sorted by descending cosine similarity with deterministic `doc_id` tie-breaks.
    pub fn search(
        &self,
        encoder: &OfflineSemanticEncoder,
        scope_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SemanticSearchHit>, EncoderError> {
        if scope_id.trim().is_empty() {
            return Err(EncoderError::EmptyScopeId);
        }
        if encoder.dim() != self.dim {
            return Err(EncoderError::DimensionMismatch {
                left: self.dim,
                right: encoder.dim(),
            });
        }
        if top_k == 0 {
            return Ok(Vec::new());
        }
        let Some(scoped_docs) = self.scopes.get(scope_id) else {
            return Ok(Vec::new());
        };
        let query_vec = encoder.encode(query);
        let mut hits = Vec::with_capacity(scoped_docs.len());
        for doc in scoped_docs.values() {
            let score = query_vec.cosine_similarity(&doc.embedding)?;
            hits.push(SemanticSearchHit {
                doc_id: doc.doc_id.clone(),
                scope_id: doc.scope_id.clone(),
                score,
            });
        }

        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.doc_id.cmp(&b.doc_id))
        });
        hits.truncate(top_k);
        Ok(hits)
    }
}

fn tokenize_clinical(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                current.push(lower);
            }
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn project_feature(accum: &mut [f32], prefix: &[u8], payload: &[u8], weight: f32) {
    let hash = fnv1a_64(prefix, payload);
    let idx = (hash as usize) % accum.len();
    let sign = if (hash >> 63) == 0 { 1.0_f32 } else { -1.0_f32 };
    accum[idx] += sign * weight;
}

fn fnv1a_64(prefix: &[u8], payload: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &b in prefix.iter().chain(payload.iter()) {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
