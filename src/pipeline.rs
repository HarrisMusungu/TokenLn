use crate::ir::DeviationReport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    FailureHeader {
        test_name: String,
    },
    PanicThread {
        test_name: String,
    },
    PanicLocation {
        file: String,
        line: u32,
        column: u32,
    },
    AssertionLeft {
        value: String,
    },
    AssertionRight {
        value: String,
    },
    PanicMessage {
        message: String,
    },
    BuildErrorHeader {
        code: Option<String>,
        message: String,
    },
    BuildLocation {
        file: String,
        line: u32,
        column: u32,
    },
    BuildHelp {
        message: String,
    },
    BuildExplainCode {
        code: String,
    },
    BuildSourceSnippet {
        line: Option<u32>,
        snippet: String,
    },
    BuildCaretMarker {
        marker: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFailure {
    pub test_name: Option<String>,
    pub panic_message: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub assertion_left: Option<String>,
    pub assertion_right: Option<String>,
    pub build_error_code: Option<String>,
    pub build_explain_code: Option<String>,
    pub build_help: Option<String>,
    pub build_source_line: Option<u32>,
    pub build_source_snippet: Option<String>,
    pub build_caret_marker: Option<String>,
    pub trace: Vec<String>,
}

pub trait Lexer {
    fn lex(&self, input: &str) -> Vec<Token>;
}

pub trait Parser {
    fn parse(&self, tokens: &[Token]) -> Vec<ParsedFailure>;
}

pub trait SemanticAnalyzer {
    fn analyze(&self, parsed_failures: &[ParsedFailure]) -> DeviationReport;
}

pub trait Optimizer {
    fn optimize(&self, report: DeviationReport) -> DeviationReport;
}

pub trait Emitter {
    fn emit(&self, report: &DeviationReport) -> String;
}
