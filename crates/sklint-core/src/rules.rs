#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleLevel {
    Normal,
    Strict,
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
        name: "StandalonePublicObjectBlankLines",
        level: RuleLevel::Normal,
        summary: "Standalone public functions and classes must be separated by exactly three blank lines",
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
        summary: "Definitions should appear before their first use",
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
        code: "SK507",
        name: "RaiseHotPath",
        level: RuleLevel::Normal,
        summary: "raise is restricted to lifecycle methods and their private helpers",
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
        code: "SK601",
        name: "DocstringLineTooLong",
        level: RuleLevel::Normal,
        summary: "Docstring lines must not exceed 72 characters from the start of the line",
    },
    Rule {
        code: "SK602",
        name: "DocstringGoogleStyleOnly",
        level: RuleLevel::Normal,
        summary: "Docstrings must use Google style only",
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
        summary: "Only Args, Attributes, Returns and Raises sections are allowed in docstrings",
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
        summary: "Function and method Args, Returns and Raises sections must not be separated by blank lines",
    },
    Rule {
        code: "SK623",
        name: "FunctionSectionOrder",
        level: RuleLevel::Normal,
        summary: "Function and method sections must appear in Args, Returns, Raises order",
    },
    Rule {
        code: "SK624",
        name: "DocstringItemContinuationStartsNextLine",
        level: RuleLevel::Normal,
        summary: "Long argument, attribute and exception descriptions must start on the next indented line",
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
    selector == "ALL" || code == selector || code.starts_with(&selector)
}
