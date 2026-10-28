//! Bounded ast-grep matching on trees owned and parsed by code-intel.

use crate::{ByteSpan, CodeIntelError, Lang, Parsed};
use ast_grep_core::AstGrep;
use ast_grep_core::language::Language as AstLanguage;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::source::Doc;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use std::borrow::Cow;
use std::ops::Range;
use tree_sitter::{Node as TsNode, Tree};

/// Maximum number of syntax nodes visited by a security pattern query.
pub const MAX_AST_PATTERN_NODES: usize = 100_000;
const MAX_PATTERN_BYTES: usize = 4096;

/// Validates a bounded ast-grep pattern for one of code-intel's supported grammars.
pub fn validate_ast_pattern(lang: Lang, pattern: &str) -> Result<(), CodeIntelError> {
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_BYTES {
        return Err(CodeIntelError::AstPattern(format!(
            "pattern must contain 1..={MAX_PATTERN_BYTES} bytes"
        )));
    }
    Pattern::try_new(pattern, lang)
        .map(|_| ())
        .map_err(|error| CodeIntelError::AstPattern(error.to_string()))
}

#[derive(Clone)]
struct ParsedDoc {
    source: String,
    lang: Lang,
    tree: Tree,
}

impl Doc for ParsedDoc {
    type Source = String;
    type Lang = Lang;
    type Node<'r> = TsNode<'r>;

    fn get_lang(&self) -> &Self::Lang {
        &self.lang
    }

    fn get_source(&self) -> &Self::Source {
        &self.source
    }

    fn do_edit(&mut self, _edit: &ast_grep_core::source::Edit<Self::Source>) -> Result<(), String> {
        Err(String::from("code-intel parsed documents are immutable"))
    }

    fn root_node(&self) -> Self::Node<'_> {
        self.tree.root_node()
    }

    fn get_node_text<'a>(&'a self, node: &Self::Node<'a>) -> Cow<'a, str> {
        match node.utf8_text(self.source.as_bytes()) {
            Ok(text) => Cow::Borrowed(text),
            Err(_) => Cow::Owned(String::new()),
        }
    }
}

impl AstLanguage for Lang {
    fn expando_char(&self) -> char {
        match self {
            Lang::Python | Lang::Rust | Lang::Cpp | Lang::Cuda => 'µ',
            Lang::TypeScript => '$',
        }
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        if matches!(self, Lang::TypeScript) || !query.contains('$') {
            return Cow::Borrowed(query);
        }
        let mut output = String::with_capacity(query.len());
        let mut dollar_count = 0_usize;
        for character in query.chars() {
            if character == '$' {
                dollar_count = dollar_count.saturating_add(1);
                continue;
            }
            let replace = matches!(character, 'A'..='Z' | '_') || dollar_count == 3;
            let sigil = if replace { self.expando_char() } else { '$' };
            output.extend(std::iter::repeat_n(sigil, dollar_count));
            dollar_count = 0;
            output.push(character);
        }
        let sigil = if dollar_count == 3 {
            self.expando_char()
        } else {
            '$'
        };
        output.extend(std::iter::repeat_n(sigil, dollar_count));
        Cow::Owned(output)
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        self.grammar().id_for_node_kind(kind, true)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.grammar().field_id_for_name(field).map(|id| id.get())
    }

    fn build_pattern(&self, builder: &PatternBuilder<'_>) -> Result<Pattern, PatternError> {
        builder.build(|source| StrDoc::try_new(source, *self))
    }
}

impl LanguageExt for Lang {
    fn get_ts_language(&self) -> TSLanguage {
        self.grammar()
    }
}

/// Evaluates an ast-grep pattern without reparsing the source document.
pub(crate) fn find_ast_pattern(
    parsed: &Parsed,
    pattern: &str,
    max_matches: usize,
    max_nodes: usize,
) -> Result<Vec<ByteSpan>, CodeIntelError> {
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_BYTES {
        return Err(CodeIntelError::AstPattern(format!(
            "pattern must contain 1..={MAX_PATTERN_BYTES} bytes"
        )));
    }
    if max_nodes == 0 || parsed.node_count() > max_nodes.min(MAX_AST_PATTERN_NODES) {
        return Err(CodeIntelError::AstPattern(format!(
            "syntax tree exceeds node limit {}",
            max_nodes.min(MAX_AST_PATTERN_NODES)
        )));
    }
    let source = std::str::from_utf8(parsed.source())
        .map_err(|_| CodeIntelError::AstPattern(String::from("source is not valid UTF-8")))?;
    let compiled = Pattern::try_new(pattern, parsed.lang())
        .map_err(|error| CodeIntelError::AstPattern(error.to_string()))?;
    let doc = ParsedDoc {
        source: source.to_owned(),
        lang: parsed.lang(),
        tree: parsed.tree().clone(),
    };
    let root = AstGrep::doc(doc);
    let mut output = Vec::new();
    for matched in root.root().find_all(compiled) {
        if output.len() >= max_matches {
            return Err(CodeIntelError::AstPattern(format!(
                "match count exceeds limit {max_matches}"
            )));
        }
        let range: Range<usize> = matched.range();
        output.push(ByteSpan::new(range.start, range.end));
    }
    Ok(output)
}
