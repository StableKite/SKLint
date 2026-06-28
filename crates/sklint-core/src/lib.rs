//! SKLint core API.
//!
//! The crate intentionally keeps dependencies at zero for the release core:
//! project configuration is parsed from a small `[tool.sklint]` TOML subset,
//! diagnostics are plain structs, and consumers can embed the analyzer without
//! pulling in a CLI or VSCode-specific code.

pub mod analyzer;
pub mod blank_lines;
pub mod comments;
pub mod config;
pub mod diagnostic;
pub mod docstrings;
pub mod dynamic_attrs;
pub mod formatter;
pub mod rules;
pub mod suppression;
pub mod syntax_rules;

pub use analyzer::{analyze, AnalysisInput, AnalysisReport};
pub use config::{EffectiveConfig, FileInlineConfig, PyProjectConfig, VscodeConfig};
pub use diagnostic::{Diagnostic, Fix};
pub use rules::{Rule, RuleLevel, ALL_RULES};
