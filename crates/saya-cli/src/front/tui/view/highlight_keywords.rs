//! The words each language reserves, as data.
//!
//! Short on purpose: a reader wants the shape of a line, and a word this table
//! does not carry paints plain rather than wrong. The lists are the constructs
//! an answer's fenced block actually shows, not a compiler's full grammar.

pub(super) const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where",
    "while",
];

pub(super) const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

pub(super) const SCRIPT: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "while",
];

pub(super) const SQL: &[&str] = &[
    "all", "and", "as", "asc", "avg", "between", "by", "case", "count", "create", "delete", "desc",
    "distinct", "drop", "else", "end", "from", "group", "having", "in", "inner", "insert", "into",
    "is", "join", "left", "limit", "max", "min", "not", "null", "offset", "on", "or", "order",
    "outer", "right", "select", "set", "sum", "table", "then", "union", "update", "values", "when",
    "where",
];

pub(super) const SHELL: &[&str] = &[
    "case", "cd", "do", "done", "echo", "elif", "else", "esac", "exit", "export", "fi", "for",
    "function", "if", "in", "local", "return", "set", "source", "then", "unset", "while",
];

pub(super) const JSON: &[&str] = &["true", "false", "null"];

pub(super) const BOOLEANS: &[&str] = &["true", "false"];

pub(super) const YAML: &[&str] = &["false", "no", "null", "off", "on", "true", "yes"];

pub(super) const GO: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "false",
    "for",
    "func",
    "go",
    "if",
    "import",
    "int",
    "interface",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "select",
    "string",
    "struct",
    "switch",
    "true",
    "type",
    "var",
];

pub(super) const JAVA: &[&str] = &[
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "do",
    "else",
    "extends",
    "false",
    "final",
    "finally",
    "for",
    "if",
    "implements",
    "import",
    "int",
    "interface",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "String",
    "switch",
    "this",
    "throw",
    "throws",
    "true",
    "try",
    "void",
    "while",
];
