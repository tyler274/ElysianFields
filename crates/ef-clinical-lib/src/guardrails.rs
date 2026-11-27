//! Shared execution guardrails (`EF-HARNESS-0002` / `SCN-M3-0014`):
//! - Frozen in-memory SHA-256-verified skill registry (`skill_view`, zero disk access)
//! - Explicit model role settings (zero implicit defaults)
//! - Typed JSON Schema tool argument validation
//! - Bounded turn/token/tool-step quota enforcement (`CutOff`) & explicit cancellation (`Stopped`)
//! - Strict allowlist content-free operational telemetry pipeline (`queue_size = 1000`, non-blocking)

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hash::sha256_hex;

/// Bounded non-blocking operational telemetry queue capacity mandated by §4.1.
pub const OPS_TELEMETRY_QUEUE_SIZE: usize = 1_000;

/// Canonical rewritten log body for all operational telemetry events (§4.1).
pub const OPERATIONAL_LOG_BODY: &str = "operational.event";

/// Allowlisted operational telemetry attribute keys permitted in `traces/ops` and `logs/ops`.
pub const ALLOWED_OPS_ATTRIBUTES: [&str; 11] = [
    "service.name",
    "http.route",
    "http.status_code",
    "run.id",
    "work.id",
    "job.id",
    "outcome",
    "model.id",
    "tokens.input",
    "tokens.output",
    "error.type",
];

/// Metadata exposed in system prompts (catalog only; never full resource bodies).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    pub resource_paths: Vec<String>,
}

/// Frozen in-memory skill package verified by SHA-256 at turn admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenSkillPackage {
    skills: BTreeMap<String, FrozenSkill>,
    package_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrozenSkill {
    description: String,
    resources: BTreeMap<String, String>,
}

/// Typed errors returned by `skill_view` and skill package verification.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillViewError {
    #[error("malformed or path-traversal skill path rejected: '{path}'")]
    PathTraversalRejected { path: String },
    #[error("unknown skill '{name}' in frozen package")]
    UnknownSkill { name: String },
    #[error("unknown resource '{resource}' in skill '{skill}'")]
    UnknownResource { skill: String, resource: String },
    #[error("frozen skill package SHA-256 digest mismatch (expected {expected}, got {actual})")]
    DigestMismatch { expected: String, actual: String },
}

impl FrozenSkillPackage {
    /// Freezes an in-memory skill bundle and computes its deterministic SHA-256 package digest.
    #[must_use]
    pub fn freeze(entries: BTreeMap<String, (String, BTreeMap<String, String>)>) -> Self {
        let mut skills = BTreeMap::new();
        for (name, (description, resources)) in entries {
            skills.insert(
                name,
                FrozenSkill {
                    description,
                    resources,
                },
            );
        }
        let package_sha256 = compute_package_digest(&skills);
        Self {
            skills,
            package_sha256,
        }
    }

    #[must_use]
    pub fn package_sha256(&self) -> &str {
        &self.package_sha256
    }

    /// Verifies that the in-memory package matches an admitted SHA-256 digest.
    pub fn verify_digest(&self, expected_sha256: &str) -> Result<(), SkillViewError> {
        let actual = compute_package_digest(&self.skills);
        if actual != expected_sha256 {
            return Err(SkillViewError::DigestMismatch {
                expected: expected_sha256.to_owned(),
                actual,
            });
        }
        Ok(())
    }

    /// Returns metadata-only catalog entries suitable for system prompts.
    #[must_use]
    pub fn catalog(&self) -> Vec<SkillCatalogEntry> {
        self.skills
            .iter()
            .map(|(name, skill)| SkillCatalogEntry {
                name: name.clone(),
                description: skill.description.clone(),
                resource_paths: skill.resources.keys().cloned().collect(),
            })
            .collect()
    }

    /// Executes `skill_view` strictly from frozen memory, rejecting traversal paths without disk I/O.
    pub fn skill_view(
        &self,
        skill_name: &str,
        resource_path: &str,
    ) -> Result<&str, SkillViewError> {
        validate_safe_skill_segment(skill_name)?;
        validate_safe_resource_path(resource_path)?;

        let Some(skill) = self.skills.get(skill_name) else {
            return Err(SkillViewError::UnknownSkill {
                name: skill_name.to_owned(),
            });
        };
        let Some(body) = skill.resources.get(resource_path) else {
            return Err(SkillViewError::UnknownResource {
                skill: skill_name.to_owned(),
                resource: resource_path.to_owned(),
            });
        };
        Ok(body.as_str())
    }
}

fn validate_safe_skill_segment(segment: &str) -> Result<(), SkillViewError> {
    if segment.is_empty()
        || segment.contains("..")
        || segment.contains('/')
        || segment.contains('\\')
        || segment.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(SkillViewError::PathTraversalRejected {
            path: segment.to_owned(),
        });
    }
    Ok(())
}

fn validate_safe_resource_path(path: &str) -> Result<(), SkillViewError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains("..")
        || path.chars().any(|c| c.is_control())
    {
        return Err(SkillViewError::PathTraversalRejected {
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn compute_package_digest(skills: &BTreeMap<String, FrozenSkill>) -> String {
    let mut buf = Vec::new();
    for (name, skill) in skills {
        buf.extend_from_slice(name.as_bytes());
        buf.push(b'\0');
        buf.extend_from_slice(skill.description.as_bytes());
        buf.push(b'\0');
        for (path, body) in &skill.resources {
            buf.extend_from_slice(path.as_bytes());
            buf.push(b'=');
            buf.extend_from_slice(body.as_bytes());
            buf.push(b'\n');
        }
    }
    sha256_hex(&buf)
}

/// Explicit reasoning effort required per enabled model role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

/// Explicit model role settings and bounded execution quotas (never implicit defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRolePolicy {
    pub role_name: String,
    pub model_id: String,
    pub reasoning_effort: ReasoningEffort,
    pub working_window_tokens: u32,
    pub input_token_limit: u32,
    pub max_turns: u32,
    pub max_tool_steps: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RolePolicyError {
    #[error("explicit non-empty value required for role policy field '{field}'")]
    MissingExplicitField { field: &'static str },
    #[error("input_token_limit ({input_limit}) exceeds working_window_tokens ({window})")]
    InputLimitExceedsWindow { input_limit: u32, window: u32 },
}

impl ModelRolePolicy {
    pub fn validate(&self) -> Result<(), RolePolicyError> {
        if self.role_name.trim().is_empty() {
            return Err(RolePolicyError::MissingExplicitField { field: "role_name" });
        }
        if self.model_id.trim().is_empty() {
            return Err(RolePolicyError::MissingExplicitField { field: "model_id" });
        }
        if self.working_window_tokens == 0 {
            return Err(RolePolicyError::MissingExplicitField {
                field: "working_window_tokens",
            });
        }
        if self.input_token_limit == 0 {
            return Err(RolePolicyError::MissingExplicitField {
                field: "input_token_limit",
            });
        }
        if self.input_token_limit > self.working_window_tokens {
            return Err(RolePolicyError::InputLimitExceedsWindow {
                input_limit: self.input_token_limit,
                window: self.working_window_tokens,
            });
        }
        if self.max_turns == 0 {
            return Err(RolePolicyError::MissingExplicitField { field: "max_turns" });
        }
        if self.max_tool_steps == 0 {
            return Err(RolePolicyError::MissingExplicitField {
                field: "max_tool_steps",
            });
        }
        Ok(())
    }
}

/// JSON property type for typed tool argument validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonPropertyType {
    String,
    Integer,
    Boolean,
    Array,
    Object,
}

/// Typed tool JSON Schema specification enforced before tool dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchemaSpec {
    pub tool_name: String,
    pub required_fields: BTreeMap<String, JsonPropertyType>,
    pub optional_fields: BTreeMap<String, JsonPropertyType>,
    pub reject_unknown_fields: bool,
}

/// Structured tool validation error returned verbatim to the model and audit log.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ToolValidationError {
    #[error("tool '{tool}' arguments must be a JSON object")]
    ArgumentsNotObject { tool: String },
    #[error("tool '{tool}' missing required field '{field}'")]
    MissingRequiredField { tool: String, field: String },
    #[error("tool '{tool}' field '{field}' has invalid JSON type (expected {expected})")]
    InvalidFieldType {
        tool: String,
        field: String,
        expected: String,
    },
    #[error("tool '{tool}' rejected unknown field '{field}'")]
    UnknownField { tool: String, field: String },
}

impl ToolSchemaSpec {
    pub fn validate(&self, args: &serde_json::Value) -> Result<(), ToolValidationError> {
        let Some(obj) = args.as_object() else {
            return Err(ToolValidationError::ArgumentsNotObject {
                tool: self.tool_name.clone(),
            });
        };

        for (field, expected) in &self.required_fields {
            let Some(val) = obj.get(field) else {
                return Err(ToolValidationError::MissingRequiredField {
                    tool: self.tool_name.clone(),
                    field: field.clone(),
                });
            };
            if !matches_json_type(val, *expected) {
                return Err(ToolValidationError::InvalidFieldType {
                    tool: self.tool_name.clone(),
                    field: field.clone(),
                    expected: format!("{expected:?}"),
                });
            }
        }

        for (field, val) in obj {
            if let Some(expected) = self.optional_fields.get(field) {
                if !matches_json_type(val, *expected) {
                    return Err(ToolValidationError::InvalidFieldType {
                        tool: self.tool_name.clone(),
                        field: field.clone(),
                        expected: format!("{expected:?}"),
                    });
                }
            } else if !self.required_fields.contains_key(field) && self.reject_unknown_fields {
                return Err(ToolValidationError::UnknownField {
                    tool: self.tool_name.clone(),
                    field: field.clone(),
                });
            }
        }
        Ok(())
    }
}

fn matches_json_type(val: &serde_json::Value, expected: JsonPropertyType) -> bool {
    match expected {
        JsonPropertyType::String => val.is_string(),
        JsonPropertyType::Integer => val.is_i64() || val.is_u64(),
        JsonPropertyType::Boolean => val.is_boolean(),
        JsonPropertyType::Array => val.is_array(),
        JsonPropertyType::Object => val.is_object(),
    }
}

/// Ordered execution event stream emitted by the shared harness (§4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum GuardrailEvent {
    WindowOpened { unix_ms: u64 },
    ProviderStarted { model_id: String, turn: u32 },
    ToolStepRecorded { tool_name: String, step_index: u32 },
    TextDeltaKept { text: String },
    Terminal { status: GuardrailTerminalStatus },
}

/// Terminal status for a bounded harness run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardrailTerminalStatus {
    Finished,
    Stopped,
    CutOff { reason: CutOffReason },
    CouldntFinish { error_type: String },
}

/// Explicit cause when a bounded execution hits its configured quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutOffReason {
    TurnLimitReached,
    ToolStepLimitReached,
    TokenBudgetReached,
}

/// Bounded execution quota & cancellation tracker with exact durable-event reconstruction.
#[derive(Debug, Clone)]
pub struct ExecutionQuotaTracker {
    policy: ModelRolePolicy,
    turns_used: u32,
    tool_steps_used: u32,
    tokens_used: u32,
    kept_output: String,
    events: Vec<GuardrailEvent>,
    terminal: Option<GuardrailTerminalStatus>,
}

impl ExecutionQuotaTracker {
    pub fn start(policy: ModelRolePolicy, opened_unix_ms: u64) -> Result<Self, RolePolicyError> {
        policy.validate()?;
        Ok(Self {
            policy,
            turns_used: 0,
            tool_steps_used: 0,
            tokens_used: 0,
            kept_output: String::new(),
            events: vec![GuardrailEvent::WindowOpened {
                unix_ms: opened_unix_ms,
            }],
            terminal: None,
        })
    }

    /// Reconstructs exact working state strictly from admitted durable events (never a shadow store).
    pub fn reconstruct_from_events(
        policy: ModelRolePolicy,
        admitted_events: &[GuardrailEvent],
    ) -> Result<Self, RolePolicyError> {
        policy.validate()?;
        let mut tracker = Self {
            policy,
            turns_used: 0,
            tool_steps_used: 0,
            tokens_used: 0,
            kept_output: String::new(),
            events: admitted_events.to_vec(),
            terminal: None,
        };
        for ev in admitted_events {
            match ev {
                GuardrailEvent::WindowOpened { .. } => {}
                GuardrailEvent::ProviderStarted { .. } => {
                    tracker.turns_used = tracker.turns_used.saturating_add(1);
                }
                GuardrailEvent::ToolStepRecorded { .. } => {
                    tracker.tool_steps_used = tracker.tool_steps_used.saturating_add(1);
                }
                GuardrailEvent::TextDeltaKept { text } => {
                    tracker.kept_output.push_str(text);
                }
                GuardrailEvent::Terminal { status } => {
                    tracker.terminal = Some(status.clone());
                }
            }
        }
        Ok(tracker)
    }

    pub fn begin_turn(&mut self, input_tokens: u32) -> Option<GuardrailTerminalStatus> {
        if self.terminal.is_some() {
            return self.terminal.clone();
        }
        if self.turns_used >= self.policy.max_turns {
            return Some(self.settle(GuardrailTerminalStatus::CutOff {
                reason: CutOffReason::TurnLimitReached,
            }));
        }
        if self.tokens_used.saturating_add(input_tokens) > self.policy.working_window_tokens {
            return Some(self.settle(GuardrailTerminalStatus::CutOff {
                reason: CutOffReason::TokenBudgetReached,
            }));
        }
        self.turns_used = self.turns_used.saturating_add(1);
        self.tokens_used = self.tokens_used.saturating_add(input_tokens);
        self.events.push(GuardrailEvent::ProviderStarted {
            model_id: self.policy.model_id.clone(),
            turn: self.turns_used,
        });
        None
    }

    pub fn record_tool_step(&mut self, tool_name: &str) -> Option<GuardrailTerminalStatus> {
        if self.terminal.is_some() {
            return self.terminal.clone();
        }
        if self.tool_steps_used >= self.policy.max_tool_steps {
            return Some(self.settle(GuardrailTerminalStatus::CutOff {
                reason: CutOffReason::ToolStepLimitReached,
            }));
        }
        self.tool_steps_used = self.tool_steps_used.saturating_add(1);
        self.events.push(GuardrailEvent::ToolStepRecorded {
            tool_name: tool_name.to_owned(),
            step_index: self.tool_steps_used,
        });
        None
    }

    pub fn keep_delta(&mut self, delta: &str) {
        if self.terminal.is_none() && !delta.is_empty() {
            self.kept_output.push_str(delta);
            self.events.push(GuardrailEvent::TextDeltaKept {
                text: delta.to_owned(),
            });
        }
    }

    pub fn cancel(&mut self) -> GuardrailTerminalStatus {
        self.settle(GuardrailTerminalStatus::Stopped)
    }

    pub fn finish(&mut self) -> GuardrailTerminalStatus {
        self.settle(GuardrailTerminalStatus::Finished)
    }

    #[must_use]
    pub fn kept_output(&self) -> &str {
        &self.kept_output
    }

    #[must_use]
    pub fn events(&self) -> &[GuardrailEvent] {
        &self.events
    }

    #[must_use]
    pub const fn terminal(&self) -> Option<&GuardrailTerminalStatus> {
        self.terminal.as_ref()
    }

    fn settle(&mut self, status: GuardrailTerminalStatus) -> GuardrailTerminalStatus {
        if let Some(existing) = &self.terminal {
            return existing.clone();
        }
        self.events.push(GuardrailEvent::Terminal {
            status: status.clone(),
        });
        self.terminal = Some(status.clone());
        status
    }
}

/// Sanitized content-free operational telemetry record exported to `traces/ops` or `logs/ops`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedOpsRecord {
    pub record_kind: OpsRecordKind,
    /// Always empty `""` for operational spans (`status.message` cleared).
    pub status_message: String,
    /// Always `"operational.event"` for operational logs.
    pub body: String,
    /// Strictly allowlisted operational attributes only.
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpsRecordKind {
    TraceSpan,
    LogEvent,
}

/// Non-blocking allowlist operational telemetry pipeline (`queue_size = 1000`, `block_on_overflow = false`).
#[derive(Debug, Clone)]
pub struct OpsTelemetryPipeline {
    max_queue_size: usize,
    queue: VecDeque<SanitizedOpsRecord>,
    dropped_overflow: u64,
}

impl Default for OpsTelemetryPipeline {
    fn default() -> Self {
        Self::new(OPS_TELEMETRY_QUEUE_SIZE)
    }
}

impl OpsTelemetryPipeline {
    #[must_use]
    pub fn new(max_queue_size: usize) -> Self {
        Self {
            max_queue_size: max_queue_size.max(1),
            queue: VecDeque::new(),
            dropped_overflow: 0,
        }
    }

    /// Sanitizes raw attributes against the strict operational allowlist, clears `status.message`,
    /// rewrites log body to `"operational.event"`, and enqueues non-blockingly.
    pub fn emit(
        &mut self,
        kind: OpsRecordKind,
        _raw_status_or_body: &str,
        raw_attributes: &BTreeMap<String, String>,
    ) -> bool {
        if self.queue.len() >= self.max_queue_size {
            self.dropped_overflow = self.dropped_overflow.saturating_add(1);
            return false;
        }
        let mut allowed = BTreeMap::new();
        for (k, v) in raw_attributes {
            if ALLOWED_OPS_ATTRIBUTES.contains(&k.as_str()) {
                allowed.insert(k.clone(), v.clone());
            }
        }
        self.queue.push_back(SanitizedOpsRecord {
            record_kind: kind,
            status_message: String::new(),
            body: match kind {
                OpsRecordKind::LogEvent => OPERATIONAL_LOG_BODY.to_owned(),
                OpsRecordKind::TraceSpan => String::new(),
            },
            attributes: allowed,
        });
        true
    }

    pub fn drain_exported(&mut self) -> Vec<SanitizedOpsRecord> {
        self.queue.drain(..).collect()
    }

    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub const fn dropped_overflow(&self) -> u64 {
        self.dropped_overflow
    }
}
