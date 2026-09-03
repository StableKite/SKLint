//! SKLint core API.
//!
//! The core uses a real Python parser (`rustpython-parser`) for semantic rules.
//! Diagnostics remain plain structs, and CLI/VSCode integration stays outside
//! the syntax layer so consumers can embed the analyzer directly.

pub mod analyzer;
pub mod blank_lines;
pub mod comments;
pub mod config;
pub mod dataclass_model;
pub mod diagnostic;
pub mod docstrings;
pub mod dynamic_attrs;
pub mod formatter;
mod identifier;
pub mod magic_constants;
pub mod pydoclint;
pub mod pydoclint_doc;
pub mod python_ast;
pub mod rules;
pub mod suppression;
pub mod syntax_rules;

pub use analyzer::{analyze, AnalysisInput, AnalysisReport};
pub use config::{
    EffectiveConfig, FileInlineConfig, PyProjectConfig, PydoclintCliOverrides, VscodeConfig,
};
pub use diagnostic::{Diagnostic, Fix};
pub use rules::{Rule, RuleLevel, ALL_RULES};
