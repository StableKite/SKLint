#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleLevel {
    Normal,
    Strict,
    /// Disabled in every default mode; enabled only by an explicit selector.
    OptIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub code: &'static str,
    pub name: &'static str,
    pub level: RuleLevel,
    pub summary: &'static str,
}

pub const ALL_RULES: &[Rule] = &[
    Rule {
        code: "SK001",
        name: "TrailingWhitespace",
        level: RuleLevel::Normal,
        summary: "Trailing spaces and tabs are not allowed",
    },
    Rule {
        code: "SK101",
        name: "TodoComment",
        level: RuleLevel::Strict,
        summary: "TODO comments are not allowed in strict mode",
    },
    Rule {
        code: "SK201",
        name: "PrintStatement",
        level: RuleLevel::Normal,
        summary: "print calls are forbidden outside if __name__ == \"__main__\" blocks",
    },
    Rule {
        code: "SK211",
        name: "CommentCyrillicSentenceCapitalized",
        level: RuleLevel::Normal,
        summary: "Cyrillic comment sentences must start with an uppercase letter",
    },
    Rule {
        code: "SK212",
        name: "CommentTrailingPeriod",
        level: RuleLevel::Normal,
        summary: "Comments must not end with a period",
    },
    Rule {
        code: "SK301",
        name: "NestedClassBlankLines",
        level: RuleLevel::Normal,
        summary: "Nested classes must use exactly one blank line between methods and no blank lines elsewhere",
    },
    Rule {
        code: "SK302",
        name: "NestedFunctionBlankLines",
        level: RuleLevel::Normal,
        summary: "Nested function bodies must not contain blank lines",
    },
    Rule {
        code: "SK303",
        name: "RegularClassMethodBlankLines",
        level: RuleLevel::Normal,
        summary: "Regular class methods must be separated by exactly two blank lines",
    },
    Rule {
        code: "SK305",
        name: "FunctionBodyConsecutiveBlankLines",
        level: RuleLevel::Normal,
        summary: "Function and method bodies must not contain more than one consecutive blank line",
    },
    Rule {
        code: "SK306",
        name: "StandaloneTopLevelObjectBlankLines",
        level: RuleLevel::Normal,
        summary: "Standalone top-level functions and classes must be separated by exactly three blank lines",
    },
    Rule {
        code: "SK307",
        name: "MainBlockBlankLinesBefore",
        level: RuleLevel::Normal,
        summary: "The __main__ block must be preceded by exactly three blank lines",
    },
    Rule {
        code: "SK308",
        name: "MainBlockConsecutiveBlankLines",
        level: RuleLevel::Normal,
        summary: "The __main__ block must not contain more than one consecutive blank line",
    },
    Rule {
        code: "SK309",
        name: "FinalNewline",
        level: RuleLevel::Normal,
        summary: "Files must not end with a newline",
    },
    Rule {
        code: "SK310",
        name: "PrivateHelperBlankLines",
        level: RuleLevel::Normal,
        summary: "Private helpers used only by the following object must be separated by exactly two blank lines",
    },
    Rule {
        code: "SK311",
        name: "StubClassMethodBlankLines",
        level: RuleLevel::Normal,
        summary: "Stub class methods must be separated by exactly one blank line",
    },
    Rule {
        code: "SK312",
        name: "StubClassBlankLines",
        level: RuleLevel::Normal,
        summary: "Top-level stub classes must be separated by exactly two blank lines",
    },
    Rule {
        code: "SK313",
        name: "StubEllipsisDocstringBlankLines",
        level: RuleLevel::Normal,
        summary: "Stub function ellipsis must not be separated from the following docstring by a blank line",
    },
    Rule {
        code: "SK314",
        name: "TypeCheckingStubClassBlankLines",
        level: RuleLevel::Normal,
        summary: "Argumentless TYPE_CHECKING stub classes must be separated by exactly one blank line",
    },
    Rule {
        code: "SK315",
        name: "StubDocstringEllipsisBlankLines",
        level: RuleLevel::Normal,
        summary: "Stub function docstrings must not be separated from the following ellipsis by a blank line",
    },
    Rule {
        code: "SK401",
        name: "AssignmentOperatorSpacing",
        level: RuleLevel::Normal,
        summary: "Assignment operators must have spaces on both sides",
    },
    Rule {
        code: "SK403",
        name: "MultilineBracketItemLayout",
        level: RuleLevel::Normal,
        summary: "Multiline bracket items must be placed one per line with indentation",
    },
    Rule {
        code: "SK404",
        name: "TrailingComma",
        level: RuleLevel::Normal,
        summary: "Trailing commas are not allowed outside import blocks",
    },
    Rule {
        code: "SK502",
        name: "FromImportOnly",
        level: RuleLevel::Normal,
        summary: "Imports must use from-import form except sys platform/version guards",
    },
    Rule {
        code: "SK503",
        name: "PreferSysPlatform",
        level: RuleLevel::Normal,
        summary: "Use sys.platform instead of os.name for platform checks",
    },
    Rule {
        code: "SK504",
        name: "DirectSysPlatformImport",
        level: RuleLevel::Normal,
        summary: "Use import sys and sys.platform instead of from sys import platform",
    },
    Rule {
        code: "SK505",
        name: "DefinitionOrder",
        level: RuleLevel::Normal,
        summary: "Definitions should appear before their first eager use",
    },
    Rule {
        code: "SK509",
        name: "SpecialMethodOrder",
        level: RuleLevel::Normal,
        summary: "__new__, __init__ and __post_init__ must appear before regular methods in this order",
    },
    Rule {
        code: "SK506",
        name: "TryExceptFinallyForbidden",
        level: RuleLevel::Normal,
        summary: "try, except and finally blocks are forbidden in hot runtime code",
    },
    Rule {
        code: "SK510",
        name: "ContextlibSuppressForbidden",
        level: RuleLevel::Strict,
        summary: "contextlib.suppress is forbidden in strict mode because it hides exceptional control flow like try/except",
    },
    Rule {
        code: "SK507",
        name: "RaiseHotPath",
        level: RuleLevel::Normal,
        summary: "raise is restricted to lifecycle/private helpers except module __getattr__ AttributeError",
    },
    Rule {
        code: "SK508",
        name: "FutureAnnotationsImport",
        level: RuleLevel::Normal,
        summary: "from __future__ import annotations is forbidden",
    },
    Rule {
        code: "SK801",
        name: "InlineSingleUseVariable",
        level: RuleLevel::Strict,
        summary: "Single-use intermediate variables should be inlined in strict mode",
    },
    Rule {
        code: "SK802",
        name: "ReturnTernary",
        level: RuleLevel::Strict,
        summary: "Return branches should be collapsed into ternary expressions in strict mode",
    },
    Rule {
        code: "SK803",
        name: "LoopComprehension",
        level: RuleLevel::Strict,
        summary: "Append-only loops should be list comprehensions in strict mode",
    },
    Rule {
        code: "SK804",
        name: "PublicAllTuple",
        level: RuleLevel::Strict,
        summary: "Modules with public symbols must define __all__ as a tuple in strict mode",
    },
    Rule {
        code: "SK805",
        name: "FileWideSuppression",
        level: RuleLevel::Strict,
        summary: "File-wide linter suppressions are forbidden in strict mode",
    },
    Rule {
        code: "SK601",
        name: "DocstringLineTooLong",
        level: RuleLevel::Normal,
        summary: "Docstring lines must not exceed 72 characters from the start of the line",
    },
    Rule {
        code: "SK602",
        name: "DocstringConfiguredStyle",
        level: RuleLevel::Normal,
        summary: "Docstrings must use the configured Google, NumPy or Sphinx style",
    },
    Rule {
        code: "SK603",
        name: "DocstringSectionTrailingPeriod",
        level: RuleLevel::Normal,
        summary: "The last line of a docstring section must not end with a period",
    },
    Rule {
        code: "SK604",
        name: "DocstringRequiresCyrillic",
        level: RuleLevel::Normal,
        summary: "Docstrings must not be fully English and must contain Cyrillic text",
    },
    Rule {
        code: "SK605",
        name: "DocstringProcessStyle",
        level: RuleLevel::Normal,
        summary: "Docstring descriptions must describe a process or state rather than an imperative action",
    },
    Rule {
        code: "SK606",
        name: "DocstringUnknownSection",
        level: RuleLevel::Normal,
        summary: "Only Args, Attributes, Returns, Yields and Raises sections are allowed in docstrings",
    },
    Rule {
        code: "SK607",
        name: "NestedDocstringBlankLine",
        level: RuleLevel::Normal,
        summary: "Nested object docstrings must not contain blank lines",
    },
    Rule {
        code: "SK608",
        name: "NestedDocstringCanBeOneLine",
        level: RuleLevel::Normal,
        summary: "Short nested object docstrings must be written on one line",
    },
    Rule {
        code: "SK609",
        name: "FinalConstantMissingDocstring",
        level: RuleLevel::Normal,
        summary: "Final constants must have an immediate string docstring description",
    },
    Rule {
        code: "SK610",
        name: "FinalConstantDocstringCanBeOneLine",
        level: RuleLevel::Normal,
        summary: "Short Final constant docstrings must be written on one line",
    },
    Rule {
        code: "SK611",
        name: "ModuleDocstringCanBeOneLine",
        level: RuleLevel::Normal,
        summary: "Short module docstrings must be written on one line",
    },
    Rule {
        code: "SK612",
        name: "PublicDocstringQuotesOwnLines",
        level: RuleLevel::Normal,
        summary: "Non-nested function, method and class docstring quotes must be on separate lines",
    },
    Rule {
        code: "SK613",
        name: "BlankLineAfterDocstring",
        level: RuleLevel::Normal,
        summary: "Multiline function, method and class docstrings must be followed by a blank line",
    },
    Rule {
        code: "SK614",
        name: "DocstringMissingDescription",
        level: RuleLevel::Normal,
        summary: "Function, method and class docstrings must contain a description outside structured sections",
    },
    Rule {
        code: "SK615",
        name: "DocstringDescriptionSectionGap",
        level: RuleLevel::Normal,
        summary: "A blank docstring line is required between the description and structured sections",
    },
    Rule {
        code: "SK616",
        name: "DocstringRedundantObjectPrefix",
        level: RuleLevel::Normal,
        summary: "Descriptions must not start with redundant words such as Method, Function or Class",
    },
    Rule {
        code: "SK617",
        name: "DocstringCyrillicSentenceCapitalized",
        level: RuleLevel::Normal,
        summary: "Cyrillic docstring sentences must start with an uppercase letter",
    },
    Rule {
        code: "SK618",
        name: "DocstringTrailingWhitespace",
        level: RuleLevel::Normal,
        summary: "Docstring lines must not end with whitespace except an intentional two-space Markdown break",
    },
    Rule {
        code: "SK619",
        name: "DataclassAttributesMissingInherited",
        level: RuleLevel::Normal,
        summary: "Dataclass Attributes sections must document all fields including inherited fields",
    },
    Rule {
        code: "SK620",
        name: "DataclassAttributeTypeMismatch",
        level: RuleLevel::Normal,
        summary: "Dataclass Attributes types must match field annotations including inherited fields",
    },
    Rule {
        code: "SK621",
        name: "ModuleDocstringNoGapBeforeCode",
        level: RuleLevel::Normal,
        summary: "Module docstrings must not be separated from following code by blank lines",
    },
    Rule {
        code: "SK622",
        name: "FunctionSectionsNoBlankLinesBetween",
        level: RuleLevel::Normal,
        summary: "Function and method Args, Returns, Yields and Raises sections must not be separated by blank lines",
    },
    Rule {
        code: "SK623",
        name: "FunctionSectionOrder",
        level: RuleLevel::Normal,
        summary: "Function and method sections must appear in Args, Returns, Yields, Raises order",
    },
    Rule {
        code: "SK624",
        name: "DocstringItemContinuationStartsNextLine",
        level: RuleLevel::Normal,
        summary: "Long argument, attribute and exception descriptions must start on the next indented line",
    },
    Rule {
        code: "SKD001",
        name: "DocstringParseError",
        level: RuleLevel::Strict,
        summary: "Potential formatting errors in docstring",
    },
    Rule {
        code: "SKD002",
        name: "PythonSyntaxError",
        level: RuleLevel::Normal,
        summary: "Syntax errors prevent Python AST analysis",
    },
    Rule {
        code: "SKD003",
        name: "DocstringStyleMismatch",
        level: RuleLevel::Strict,
        summary: "Docstring style does not match configured pydoclint style",
    },
    Rule {
        code: "SKD101",
        name: "DocstringArgumentsMissing",
        level: RuleLevel::Strict,
        summary: "Docstring contains fewer arguments than the function signature",
    },
    Rule {
        code: "SKD102",
        name: "DocstringArgumentsExtra",
        level: RuleLevel::Strict,
        summary: "Docstring contains more arguments than the function signature",
    },
    Rule {
        code: "SKD103",
        name: "DocstringArgumentsMismatch",
        level: RuleLevel::Strict,
        summary: "Docstring argument names differ from the function signature",
    },
    Rule {
        code: "SKD104",
        name: "DocstringArgumentOrder",
        level: RuleLevel::Strict,
        summary: "Docstring argument order differs from the function signature",
    },
    Rule {
        code: "SKD105",
        name: "DocstringArgumentTypeMismatch",
        level: RuleLevel::Strict,
        summary: "Docstring argument type hints differ from signature annotations",
    },
    Rule {
        code: "SKD106",
        name: "SignatureArgumentTypesMissingAll",
        level: RuleLevel::Strict,
        summary: "Argument type hints are required in the signature but none are present",
    },
    Rule {
        code: "SKD107",
        name: "SignatureArgumentTypesMissingSome",
        level: RuleLevel::Strict,
        summary: "Argument type hints are required in the signature but some are missing",
    },
    Rule {
        code: "SKD108",
        name: "SignatureArgumentTypesForbidden",
        level: RuleLevel::Strict,
        summary: "Argument type hints are forbidden in the signature by configuration",
    },
    Rule {
        code: "SKD109",
        name: "DocstringArgumentTypesMissingAll",
        level: RuleLevel::Strict,
        summary: "Argument type hints are required in the docstring but none are present",
    },
    Rule {
        code: "SKD110",
        name: "DocstringArgumentTypesMissingSome",
        level: RuleLevel::Strict,
        summary: "Argument type hints are required in the docstring but some are missing",
    },
    Rule {
        code: "SKD111",
        name: "DocstringArgumentTypesForbidden",
        level: RuleLevel::Strict,
        summary: "Argument type hints are forbidden in the docstring by configuration",
    },
    Rule {
        code: "SKD201",
        name: "ReturnSectionMissing",
        level: RuleLevel::Strict,
        summary: "Function returns a value but has no Returns section",
    },
    Rule {
        code: "SKD202",
        name: "ReturnSectionUnnecessary",
        level: RuleLevel::Strict,
        summary: "Docstring has a Returns section but the function has no return statement or annotation",
    },
    Rule {
        code: "SKD203",
        name: "ReturnTypeMismatch",
        level: RuleLevel::Strict,
        summary: "Docstring return type is inconsistent with the return annotation",
    },
    Rule {
        code: "SKD301",
        name: "InitDocstringForbidden",
        level: RuleLevel::OptIn,
        summary: "__init__ should not have its own docstring",
    },
    Rule {
        code: "SKD302",
        name: "ClassReturnSectionForbidden",
        level: RuleLevel::Strict,
        summary: "Class docstring must not contain a Returns section for __init__",
    },
    Rule {
        code: "SKD303",
        name: "InitReturnSectionForbidden",
        level: RuleLevel::Strict,
        summary: "__init__ docstring must not contain a Returns section",
    },
    Rule {
        code: "SKD304",
        name: "ClassArgumentSectionForbidden",
        level: RuleLevel::Strict,
        summary: "Class docstring argument section is forbidden when __init__ owns documentation",
    },
    Rule {
        code: "SKD305",
        name: "ClassRaisesSectionForbidden",
        level: RuleLevel::Strict,
        summary: "Class docstring Raises section is forbidden when __init__ owns documentation",
    },
    Rule {
        code: "SKD306",
        name: "ClassYieldSectionForbidden",
        level: RuleLevel::Strict,
        summary: "Class docstring must not contain a Yields section for __init__",
    },
    Rule {
        code: "SKD307",
        name: "InitYieldSectionForbidden",
        level: RuleLevel::Strict,
        summary: "__init__ docstring must not contain a Yields section",
    },
    Rule {
        code: "SKD402",
        name: "YieldSectionMissing",
        level: RuleLevel::Strict,
        summary: "Generator has yield statements but no Yields section",
    },
    Rule {
        code: "SKD403",
        name: "YieldSectionUnnecessary",
        level: RuleLevel::Strict,
        summary: "Docstring has a Yields section but the function is not a documented generator",
    },
    Rule {
        code: "SKD404",
        name: "YieldTypeMismatch",
        level: RuleLevel::Strict,
        summary: "Docstring yield type is inconsistent with the generator annotation",
    },
    Rule {
        code: "SKD405",
        name: "GeneratorReturnAnnotationMismatch",
        level: RuleLevel::Strict,
        summary: "Function with both return and yield requires a Generator-style annotation",
    },
    Rule {
        code: "SKD501",
        name: "RaisesSectionMissing",
        level: RuleLevel::Strict,
        summary: "Function raises exceptions but has no Raises section",
    },
    Rule {
        code: "SKD502",
        name: "RaisesSectionUnnecessary",
        level: RuleLevel::Strict,
        summary: "Docstring has a Raises section but the function does not raise",
    },
    Rule {
        code: "SKD503",
        name: "RaisedExceptionsMismatch",
        level: RuleLevel::Strict,
        summary: "Documented exceptions differ from exceptions raised in the function body",
    },
    Rule {
        code: "SKD504",
        name: "AssertRaisesSectionMissing",
        level: RuleLevel::Strict,
        summary: "Function contains assert but has no Raises section for AssertionError",
    },
    Rule {
        code: "SKD601",
        name: "ClassAttributesMissing",
        level: RuleLevel::Strict,
        summary: "Class docstring contains fewer attributes than the effective class fields",
    },
    Rule {
        code: "SKD602",
        name: "ClassAttributesExtra",
        level: RuleLevel::Strict,
        summary: "Class docstring contains more attributes than the effective class fields",
    },
    Rule {
        code: "SKD603",
        name: "ClassAttributesMismatch",
        level: RuleLevel::Strict,
        summary: "Documented class attributes differ from effective class fields",
    },
    Rule {
        code: "SKD604",
        name: "ClassAttributeOrder",
        level: RuleLevel::Strict,
        summary: "Class attribute order in the docstring differs from effective field order",
    },
    Rule {
        code: "SKD605",
        name: "ClassAttributeTypeMismatch",
        level: RuleLevel::Strict,
        summary: "Documented class attribute types differ from field annotations",
    },
    Rule {
        code: "SKD606",
        name: "InlineClassAttributeDocForbidden",
        level: RuleLevel::Strict,
        summary: "Class attribute is documented inline but should be documented in the class docstring",
    },
    Rule {
        code: "SKD607",
        name: "ClassAttributesSectionForbidden",
        level: RuleLevel::Strict,
        summary: "Attributes section is forbidden when inline class attribute docs are required",
    },
    Rule {
        code: "SKD608",
        name: "DocstringArgumentDescriptionEmpty",
        level: RuleLevel::Strict,
        summary: "Documented parameters and arguments must have non-empty descriptions",
    },
    Rule {
        code: "SK701",
        name: "DynamicSelfAttributeOutsideInit",
        level: RuleLevel::Normal,
        summary: "Dynamic self attributes must be introduced in __init__/__post_init__ or declared on the class",
    },
    Rule {
        code: "SK702",
        name: "DynamicObjectAttributeAssignment",
        level: RuleLevel::Normal,
        summary: "Dynamic attributes assigned to known dynamic containers must be declared on their class",
    },
    Rule {
        code: "SK900",
        name: "UnusedSuppression",
        level: RuleLevel::Normal,
        summary: "SKLint suppressions must suppress at least one active diagnostic",
    },
    Rule {
        code: "SK901",
        name: "MagicNumericConstant",
        level: RuleLevel::Strict,
        summary: "Magic numeric constants are forbidden outside self-documenting literal contexts",
    },
    Rule {
        code: "SK902",
        name: "PartialAstAnalysis",
        level: RuleLevel::Normal,
        summary: "Python syntax is valid but the embedded parser cannot provide full AST coverage",
    },
    Rule {
        code: "SK903",
        name: "SyntaxOracleUnavailable",
        level: RuleLevel::Normal,
        summary: "Embedded parsing failed and an external syntax oracle was unavailable or timed out",
    },
];

pub fn rule_by_code(code: &str) -> Option<&'static Rule> {
    ALL_RULES.iter().find(|rule| rule.code == code)
}

pub fn is_known_rule(code: &str) -> bool {
    rule_by_code(code).is_some()
}

pub fn code_matches_selector(code: &str, selector: &str) -> bool {
    let selector = selector.trim().to_ascii_uppercase();
    if selector.is_empty() {
        return false;
    }
    let canonical_selector = doc_alias_to_skd(&selector);
    canonical_selector == "ALL"
        || code == canonical_selector
        || code.starts_with(&canonical_selector)
}

fn doc_alias_to_skd(selector: &str) -> String {
    match selector.strip_prefix("DOC") {
        Some(suffix) => format!("SKD{suffix}"),
        None => selector.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::code_matches_selector;

    #[test]
    fn foreign_t201_selector_is_not_owned_by_sklint() {
        assert!(!code_matches_selector("SK201", "T201"));
        assert!(code_matches_selector("SK201", "SK201"));
    }

    #[test]
    fn doc_codes_alias_skd_codes_and_prefixes() {
        assert!(code_matches_selector("SKD105", "DOC105"));
        assert!(code_matches_selector("SKD105", "DOC1"));
        assert!(code_matches_selector("SKD605", "DOC6"));
        assert!(code_matches_selector("SKD605", "SKD6"));
        assert!(!code_matches_selector("SKD605", "DOC5"));
        assert!(!code_matches_selector("SK605", "DOC605"));
    }
}
