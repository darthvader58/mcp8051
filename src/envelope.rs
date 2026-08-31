//! The one response shape every tool returns.
//!
//! A model calling this server should never have to learn twelve result formats.
//! Every tool emits the same [`Envelope`], pretty-printed into `content` (so a
//! human reading a transcript can follow along) and byte-identical in
//! `structuredContent` (so a program can branch on it). The same type also backs
//! the declared `outputSchema` via [`envelope_output_schema`].

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::errors::ErrorCode;

/// Coarse outcome, orthogonal to `ok`.
///
/// `Warning` means the call succeeded but something deserves attention — the
/// caller may proceed. `Error` always accompanies `ok: false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warning,
    Error,
}

/// Captured child output, capped head+tail.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Tail {
    /// Head and tail of the stream, joined by an elision marker when truncated.
    pub text: String,
    /// How many bytes the child actually produced, including any elided middle.
    pub total_bytes: u64,
    /// Bytes dropped from the middle. Zero when nothing was elided.
    pub elided_bytes: u64,
    /// True when `text` is not the complete stream.
    pub truncated: bool,
}

impl Tail {
    pub fn is_empty(&self) -> bool {
        self.total_bytes == 0
    }
}

/// A concrete follow-up call, so the model does not have to guess.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NextAction {
    /// Tool to call next, if the suggestion is a call rather than advice.
    pub tool: Option<String>,
    /// Why this is worth doing.
    pub reason: String,
    /// Suggested arguments for `tool`.
    pub arguments: Value,
}

impl NextAction {
    pub fn call(tool: &str, reason: impl Into<String>, arguments: Value) -> Self {
        Self {
            tool: Some(tool.to_string()),
            reason: reason.into(),
            arguments,
        }
    }

    pub fn advice(reason: impl Into<String>) -> Self {
        Self {
            tool: None,
            reason: reason.into(),
            arguments: Value::Null,
        }
    }
}

/// The uniform tool response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Envelope {
    /// Did the operation do what was asked? Callers may branch on this alone.
    pub ok: bool,
    /// `ok` / `warning` / `error`.
    pub status: Status,
    /// Which tool produced this.
    pub tool: String,
    /// Stable machine-readable failure code. Null on success.
    pub error_code: Option<ErrorCode>,
    /// Human-readable failure message. Null on success.
    pub error: Option<String>,
    /// What to actually do about it. Null when there is nothing useful to add.
    pub remedy: Option<String>,
    /// The exact argv that ran, shell-quoted **for display only**. Never re-executed.
    pub command: Option<String>,
    /// Child exit status, when a child ran.
    pub exit_code: Option<i32>,
    /// Wall-clock time for this call.
    pub duration_ms: u64,
    /// Child stdout, capped.
    pub stdout: Option<Tail>,
    /// Child stderr, capped.
    pub stderr: Option<Tail>,
    /// Tool-specific payload.
    pub data: Value,
    /// Suggested follow-up calls.
    pub next_actions: Vec<NextAction>,
}

impl Envelope {
    pub fn new(tool: impl Into<String>) -> Self {
        Self {
            ok: true,
            status: Status::Ok,
            tool: tool.into(),
            error_code: None,
            error: None,
            remedy: None,
            command: None,
            exit_code: None,
            duration_ms: 0,
            stdout: None,
            stderr: None,
            data: json!({}),
            next_actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    #[must_use]
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    #[must_use]
    pub fn exit_code(mut self, code: Option<i32>) -> Self {
        self.exit_code = code;
        self
    }

    #[must_use]
    pub fn duration(mut self, elapsed: Duration) -> Self {
        self.duration_ms = elapsed.as_millis() as u64;
        self
    }

    #[must_use]
    pub fn stdout(mut self, tail: Tail) -> Self {
        self.stdout = Some(tail);
        self
    }

    #[must_use]
    pub fn stderr(mut self, tail: Tail) -> Self {
        self.stderr = Some(tail);
        self
    }

    /// Succeeded, but flag it. `ok` stays true.
    #[must_use]
    pub fn warn(mut self) -> Self {
        if self.status == Status::Ok {
            self.status = Status::Warning;
        }
        self
    }

    /// Failed. Sets `ok: false` and `status: error` together so the two can
    /// never disagree.
    #[must_use]
    pub fn error(mut self, code: ErrorCode, message: impl Into<String>) -> Self {
        self.ok = false;
        self.status = Status::Error;
        self.error_code = Some(code);
        self.error = Some(message.into());
        self
    }

    #[must_use]
    pub fn remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }

    #[must_use]
    pub fn next_action(mut self, action: NextAction) -> Self {
        self.next_actions.push(action);
        self
    }

    /// Render to the MCP result: pretty JSON in `content`, the same value in
    /// `structuredContent`, and `isError` mirroring `ok`.
    ///
    /// `CallToolResult` is `#[non_exhaustive]`, so it is built through
    /// `::success` and then mutated rather than by struct literal.
    pub fn finish(self) -> CallToolResult {
        let value = serde_json::to_value(&self).unwrap_or_else(|e| {
            json!({
                "ok": false,
                "status": "error",
                "tool": self.tool,
                "error_code": "INTERNAL_ERROR",
                "error": format!("envelope serialization failed: {e}"),
            })
        });
        let pretty =
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{\"ok\":false}".to_string());
        let mut result = CallToolResult::success(vec![ContentBlock::text(pretty)]);
        result.structured_content = Some(value);
        if !self.ok {
            result.is_error = Some(true);
        }
        result
    }
}

/// The shared `outputSchema` for all twelve tools.
///
/// Tools returning `Result<CallToolResult, McpError>` get no schema from the
/// macro — it only derives one for `Json<T>` returns — so it is declared
/// explicitly and identically at every `#[tool]` site.
pub fn envelope_output_schema() -> Arc<JsonObject> {
    rmcp::handler::server::common::schema_for_output::<Envelope>()
}

/// Fold a `Result` into a delivered MCP result. This is the whole body of every
/// shim in `server.rs`.
pub fn deliver(tool: &str, outcome: Result<Envelope, crate::errors::AppError>) -> CallToolResult {
    outcome
        .unwrap_or_else(|err| err.into_envelope(tool))
        .finish()
}
