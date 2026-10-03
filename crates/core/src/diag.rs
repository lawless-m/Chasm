//! Structured diagnostics. Codes are the contract; message wording may change.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub location: Location,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependants: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_effect: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &str, message: impl Into<String>, location: Location) -> Self {
        Diagnostic {
            code: code.to_string(),
            severity: Severity::Error,
            message: message.into(),
            location,
            word: None,
            expected: None,
            actual: None,
            dependants: None,
            declared_effect: None,
        }
    }

    pub fn warning(code: &str, message: impl Into<String>, location: Location) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(code, message, location)
        }
    }

    pub fn with_stacks(mut self, expected: Vec<String>, actual: Vec<String>) -> Self {
        self.expected = Some(expected);
        self.actual = Some(actual);
        self
    }

    pub fn with_word(mut self, word: &str) -> Self {
        self.word = Some(word.to_string());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// One-line text rendering: `file:line:col: error[CODE]: message`.
    pub fn render(&self) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut s = format!(
            "{}:{}:{}: {}[{}]: {}",
            self.location.file,
            self.location.line,
            self.location.column,
            sev,
            self.code,
            self.message
        );
        if let (Some(e), Some(a)) = (&self.expected, &self.actual) {
            s.push_str(&format!(
                "\n    expected: ( {} )\n    actual:   ( {} )",
                e.join(" "),
                a.join(" ")
            ));
        }
        if let Some(d) = &self.dependants {
            if !d.is_empty() {
                s.push_str(&format!("\n    dependants: {}", d.join(", ")));
            }
        }
        if let Some(e) = &self.declared_effect {
            s.push_str(&format!("\n    declared: {e}"));
        }
        s
    }
}

/// Stable diagnostic codes.
pub mod codes {
    pub const E_LEX: &str = "E_LEX";
    pub const E_SYNTAX: &str = "E_SYNTAX";
    pub const E_LITERAL_RANGE: &str = "E_LITERAL_RANGE";
    pub const E_UNKNOWN_TYPE: &str = "E_UNKNOWN_TYPE";
    pub const E_UNDEFINED: &str = "E_UNDEFINED";
    pub const E_STACK_UNDERFLOW: &str = "E_STACK_UNDERFLOW";
    pub const E_TYPE_MISMATCH: &str = "E_TYPE_MISMATCH";
    pub const E_EFFECT_MISMATCH: &str = "E_EFFECT_MISMATCH";
    pub const E_ASSERTION: &str = "E_ASSERTION";
    pub const E_BRANCH_MISMATCH: &str = "E_BRANCH_MISMATCH";
    pub const E_LOOP_EFFECT: &str = "E_LOOP_EFFECT";
    pub const E_LEAVE: &str = "E_LEAVE";
    pub const E_UNREACHABLE: &str = "E_UNREACHABLE";
    pub const E_LOCAL: &str = "E_LOCAL";
    pub const E_CAPTURE: &str = "E_CAPTURE";
    pub const E_AMBIGUOUS_TYPE: &str = "E_AMBIGUOUS_TYPE";
    pub const E_REDEFINE_EFFECT: &str = "E_REDEFINE_EFFECT";
    pub const E_DECLARE_MISMATCH: &str = "E_DECLARE_MISMATCH";
    pub const E_UNRESOLVED: &str = "E_UNRESOLVED";
    pub const E_TEST_TYPE: &str = "E_TEST_TYPE";
    pub const E_MAIN_EFFECT: &str = "E_MAIN_EFFECT";
    pub const E_NO_MAIN: &str = "E_NO_MAIN";
    pub const E_IO: &str = "E_IO";
    pub const E_USAGE: &str = "E_USAGE";
    pub const E_INTERNAL: &str = "E_INTERNAL";
    pub const E_FORGET: &str = "E_FORGET";
    pub const E_FORCE: &str = "E_FORCE";
    pub const E_NEEDS_EFFECT: &str = "E_NEEDS_EFFECT";
    pub const E_MATCH_ARM: &str = "E_MATCH_ARM";
    pub const E_MATCH_MISSING: &str = "E_MATCH_MISSING";
}
