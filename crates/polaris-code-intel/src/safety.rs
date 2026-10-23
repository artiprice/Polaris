//! Bounded AST observations used by deterministic safety and verifier checks.
//!
//! This module reports syntax facts only. It does not decide whether a change is safe or whether
//! a test is meaningful. Coverage flags are deliberately conservative: an absent observation is
//! not evidence that a language or syntax family contains no such construct.

use crate::error::CodeIntelError;
use crate::lang::Lang;
use crate::parse::{ByteSpan, Parsed};
use crate::walk::{Descend, walk_preorder};
use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use tree_sitter::Node;

/// Maximum syntax-tree nodes inspected by one safety analysis.
pub const MAX_SAFETY_NODES: usize = 1_000_000;

/// Whether an observation family was analyzed with the supported AST patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisCoverage {
    /// The language's current AST patterns were inspected.
    Complete,
    /// Some relevant syntax families or test conventions are not recognized.
    Incomplete,
    /// The current analyzer does not inventory this language family.
    Unsupported,
}

/// A source-located AST observation with its tree-sitter node kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstObservation {
    /// The node kind reported by the active grammar.
    pub node_kind: String,
    /// The byte span of the AST node.
    pub span: ByteSpan,
    /// A normalized AST-derived identity, when the node has one.
    pub identity: Option<String>,
}

/// A recognized assertion call and its canonical callee form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionObservation {
    /// The AST-derived assertion API name, such as `assert_eq` or `toEqual`.
    pub canonical_form: String,
    /// The source span of the call expression.
    pub span: ByteSpan,
    /// The nearest containing test identity, if discovered.
    pub test_identity: Option<String>,
}

/// A conservative classification of a test-disabling or test-skipping marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// Rust `ignore` attribute or a recognized skip API invocation.
    IgnoreOrSkip,
    /// Rust `cfg`/`cfg_attr` attribute whose arguments include a false configuration.
    CfgDisabled,
    /// A marker-like attribute, decorator or call that could not be classified safely.
    UnknownDisableMarker,
}

/// A disable marker attached to or invoked within a discovered test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestMarker {
    /// Marker classification.
    pub kind: MarkerKind,
    /// AST node kind containing the marker.
    pub node_kind: String,
    /// Span of the identified attribute, decorator or call node.
    pub span: ByteSpan,
    /// Text within the identified AST node, retained for downstream exact review.
    pub evidence: String,
}

/// A test-like declaration or framework invocation discovered in the AST.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestObservation {
    /// Qualified identity assembled from enclosing declarations and the test's own name.
    pub identity: String,
    /// Span of the function declaration or framework test invocation.
    pub span: ByteSpan,
    /// Assertion call spans contained by this test.
    pub assertion_spans: Vec<ByteSpan>,
    /// Skip, ignore, cfg-disable and unresolved disable markers associated with this test.
    pub markers: Vec<TestMarker>,
}

/// Deterministic tautology pattern detected from expression AST nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TautologyKind {
    /// An assertion call's first expression argument is the boolean literal `true`.
    LiteralTrueAssertion,
    /// An assertion call compares an expression with itself using equality.
    SelfEqualityAssertion,
}

/// A source-located assertion tautology candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TautologyObservation {
    /// Detected tautology rule.
    pub kind: TautologyKind,
    /// Span of the whole assertion call.
    pub span: ByteSpan,
    /// Containing test identity, if one was discovered.
    pub test_identity: Option<String>,
}

/// Aggregate counts from the same AST walk as the detailed observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SafetyCounts {
    /// Number of discovered test declarations and test-framework invocations.
    pub tests: usize,
    /// Number of recognized assertion calls.
    pub assertions: usize,
    /// Number of recognized function/method call expressions.
    pub calls: usize,
    /// Number of recognized unsafe constructs.
    pub unsafe_constructs: usize,
    /// Number of discovered tautology candidates.
    pub tautologies: usize,
}

/// Bounded observations for one parsed source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyAnalysis {
    /// Parsed language.
    pub lang: Lang,
    /// Whether tree-sitter reported a syntax error or missing token.
    pub syntax_error: bool,
    /// Coverage for each observation family; unsupported is never silently treated as empty.
    pub test_coverage: AnalysisCoverage,
    /// Coverage for unsafe construct patterns.
    pub unsafe_coverage: AnalysisCoverage,
    /// Coverage for call expressions.
    pub call_coverage: AnalysisCoverage,
    /// Coverage for assertions and tautology patterns.
    pub assertion_coverage: AnalysisCoverage,
    /// Discovered tests in source order.
    pub tests: Vec<TestObservation>,
    /// Discovered unsafe constructs.
    pub unsafe_constructs: Vec<AstObservation>,
    /// Discovered call expressions.
    pub calls: Vec<AstObservation>,
    /// Discovered assertion calls.
    pub assertions: Vec<AssertionObservation>,
    /// Discovered tautology candidates.
    pub tautologies: Vec<TautologyObservation>,
    /// Counts corresponding to the observation vectors.
    pub counts: SafetyCounts,
}

/// Inspects a parsed syntax tree for tests, assertions, skip markers, calls, unsafe constructs,
/// and simple assertion tautologies.
///
/// Rust, Python, and TypeScript use explicit AST patterns. C++ and CUDA report call and unsafe
/// constructs but mark test inventory as unsupported. TypeScript's grammar also parses much
/// JavaScript, but the result does not distinguish the two dialects. This crate has no Java
/// grammar, so Java source must not be passed as a parsed value. Any tree exceeding
/// [`MAX_SAFETY_NODES`] is rejected before traversal.
///
/// # Errors
/// Returns [`CodeIntelError::TooLarge`] when the parsed tree exceeds the inspection bound.
pub fn analyze_safety(parsed: &Parsed) -> Result<SafetyAnalysis, CodeIntelError> {
    if parsed.node_count() > MAX_SAFETY_NODES {
        return Err(CodeIntelError::TooLarge {
            limit: MAX_SAFETY_NODES,
        });
    }

    let lang = parsed.lang();
    let source = parsed.source();
    let mut nodes = Vec::new();
    walk_preorder(parsed.tree().root_node(), |node, _depth| {
        nodes.push(node);
        Descend::Into
    });

    let mut tests = Vec::new();
    let mut assertions = Vec::new();
    let mut calls = Vec::new();
    let mut unsafe_constructs = Vec::new();
    let mut tautologies = Vec::new();
    let mut skip_markers = Vec::new();

    for node in &nodes {
        if is_call_node(*node, lang) {
            let callee = call_name(*node, source);
            if callee.as_deref().is_some_and(is_skip_name) {
                skip_markers.push(TestMarker {
                    kind: MarkerKind::IgnoreOrSkip,
                    node_kind: node.kind().to_owned(),
                    span: span(*node),
                    evidence: text(*node, source).into_owned(),
                });
            }
            calls.push(AstObservation {
                node_kind: node.kind().to_owned(),
                span: span(*node),
                identity: callee.clone(),
            });
        }
        if is_assertion_node(*node, lang)
            && let Some(assertion_name) = assertion_name(*node, lang, source)
        {
            assertions.push(AssertionObservation {
                canonical_form: assertion_name,
                span: span(*node),
                test_identity: None,
            });
        }
        if is_unsafe_node(*node, lang, source) {
            unsafe_constructs.push(AstObservation {
                node_kind: node.kind().to_owned(),
                span: span(*node),
                identity: None,
            });
        }
        if let Some(kind) = tautology_kind(*node, lang, source) {
            tautologies.push(TautologyObservation {
                kind,
                span: span(*node),
                test_identity: None,
            });
        }
        if let Some((identity, test_node)) = test_identity(*node, lang, source) {
            tests.push(TestObservation {
                identity,
                span: span(test_node),
                assertion_spans: Vec::new(),
                markers: markers_for(test_node, lang, source),
            });
        }
    }

    assign_marker_tests(&mut tests, skip_markers);
    let assertion_spans_by_test = assign_test_identities(&tests, &mut assertions, &mut tautologies);
    for (test, spans) in tests.iter_mut().zip(assertion_spans_by_test) {
        test.assertion_spans = spans;
    }
    let test_coverage = match lang {
        Lang::Rust | Lang::Python | Lang::TypeScript => AnalysisCoverage::Incomplete,
        Lang::Cpp | Lang::Cuda => AnalysisCoverage::Unsupported,
    };
    let general_coverage = match lang {
        Lang::Rust | Lang::Python | Lang::Cpp | Lang::Cuda | Lang::TypeScript => {
            AnalysisCoverage::Incomplete
        }
    };
    let counts = SafetyCounts {
        tests: tests.len(),
        assertions: assertions.len(),
        calls: calls.len(),
        unsafe_constructs: unsafe_constructs.len(),
        tautologies: tautologies.len(),
    };

    Ok(SafetyAnalysis {
        lang,
        syntax_error: parsed.has_syntax_error(),
        test_coverage,
        unsafe_coverage: general_coverage,
        call_coverage: general_coverage,
        assertion_coverage: general_coverage,
        tests,
        unsafe_constructs,
        calls,
        assertions,
        tautologies,
        counts,
    })
}

fn assign_test_identities(
    tests: &[TestObservation],
    assertions: &mut [AssertionObservation],
    tautologies: &mut [TautologyObservation],
) -> Vec<Vec<ByteSpan>> {
    let mut test_order: Vec<(usize, usize, usize)> = tests
        .iter()
        .enumerate()
        .map(|(index, test)| (test.span.start, test.span.end, index))
        .collect();
    test_order.sort_unstable();
    let mut observation_order: Vec<(ByteSpan, bool, usize)> = assertions
        .iter()
        .enumerate()
        .map(|(index, value)| (value.span, false, index))
        .chain(
            tautologies
                .iter()
                .enumerate()
                .map(|(index, value)| (value.span, true, index)),
        )
        .collect();
    observation_order.sort_by_key(|(span, _, _)| (span.start, span.end));

    let mut next_test = 0;
    let mut active: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut expirations: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    let mut assertion_spans_by_test = vec![Vec::new(); tests.len()];
    for (observed_span, is_tautology, observation_index) in observation_order {
        while let Some((test_start, test_end, test_index)) = test_order.get(next_test).copied() {
            if test_start > observed_span.start {
                break;
            }
            let Some(test) = tests.get(test_index) else {
                next_test = next_test.saturating_add(1);
                continue;
            };
            let test_span = test.span;
            if test_span.end != test_end {
                next_test = next_test.saturating_add(1);
                continue;
            }
            active.insert((test_span.len(), test_index));
            expirations.push(Reverse((test_span.end, test_index)));
            next_test = next_test.saturating_add(1);
        }
        while let Some(Reverse((end, test_index))) = expirations.peek().copied() {
            if end >= observed_span.end {
                break;
            }
            if let Some(_expired) = expirations.pop() {
                if let Some(test) = tests.get(test_index) {
                    active.remove(&(test.span.len(), test_index));
                }
            } else {
                break;
            }
        }
        // Tests are inserted only after their start and removed once their end precedes this
        // observation, so every active test contains the observation. The smallest span is the
        // nearest lexical test and the first BTreeSet item gives it in O(log n) total sweep work.
        let owner = active.iter().next().map(|(_, index)| *index);
        if is_tautology {
            if let Some(value) = tautologies.get_mut(observation_index) {
                value.test_identity =
                    owner.and_then(|index| tests.get(index).map(|test| test.identity.clone()));
            }
        } else if let Some(value) = assertions.get_mut(observation_index)
            && let Some((index, identity)) =
                owner.and_then(|index| tests.get(index).map(|test| (index, test.identity.clone())))
        {
            value.test_identity = Some(identity);
            if let Some(spans) = assertion_spans_by_test.get_mut(index) {
                spans.push(value.span);
            }
        }
    }
    assertion_spans_by_test
}

fn span(node: Node<'_>) -> ByteSpan {
    ByteSpan::new(node.start_byte(), node.end_byte())
}

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> Cow<'a, str> {
    match source.get(node.start_byte()..node.end_byte()) {
        Some(bytes) => String::from_utf8_lossy(bytes),
        None => Cow::Borrowed(""),
    }
}

fn is_call_node(node: Node<'_>, lang: Lang) -> bool {
    match lang {
        Lang::Rust => node.kind() == "call_expression",
        Lang::Python => node.kind() == "call",
        Lang::Cpp | Lang::Cuda => node.kind() == "call_expression",
        Lang::TypeScript => node.kind() == "call_expression" || node.kind() == "new_expression",
    }
}

fn is_assertion_node(node: Node<'_>, lang: Lang) -> bool {
    is_call_node(node, lang)
        || lang == Lang::Rust && node.kind() == "macro_invocation"
        || lang == Lang::Python && node.kind() == "assert_statement"
}

fn assertion_name(node: Node<'_>, lang: Lang, source: &[u8]) -> Option<String> {
    if lang == Lang::Python && node.kind() == "assert_statement" {
        Some("assert".to_owned())
    } else {
        call_name(node, source).filter(|name| is_assertion_name(name))
    }
}

fn call_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let callee = node
        .child_by_field_name("function")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child_by_field_name("macro"))
        .or_else(|| first_named_child(node))?;
    let raw = text(callee, source);
    let name = raw.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

fn first_named_child<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn is_assertion_name(name: &str) -> bool {
    let mut tail_parts = name.rsplit(['.', ':', '>']);
    let tail = match tail_parts.next() {
        Some(value) => value.trim(),
        None => name.trim(),
    };
    matches!(
        tail,
        "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "assertEqual"
            | "assertNotEqual"
            | "assertTrue"
            | "assertFalse"
            | "assertIsNone"
            | "assertIsNotNone"
            | "assertRaises"
            | "ok"
            | "equal"
            | "strictEqual"
            | "deepEqual"
            | "expect"
            | "toBe"
            | "toEqual"
            | "toStrictEqual"
            | "toBeTruthy"
            | "toBeFalsy"
            | "toHaveBeenCalled"
            | "toThrow"
    )
}

fn is_skip_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".skip") || lowered.ends_with(".ignore") || lowered.ends_with(".todo")
}

fn is_unsafe_node(node: Node<'_>, lang: Lang, source: &[u8]) -> bool {
    match lang {
        Lang::Rust => matches!(node.kind(), "unsafe_block" | "unsafe_function_item"),
        Lang::Python => {
            node.kind() == "call"
                && call_name(node, source)
                    .is_some_and(|name| matches!(name.as_str(), "eval" | "exec" | "compile"))
        }
        Lang::Cpp | Lang::Cuda => {
            matches!(node.kind(), "pointer_declarator" | "cast_expression")
        }
        Lang::TypeScript => node.kind() == "type_assertion" || node.kind() == "as_expression",
    }
}

fn tautology_kind(node: Node<'_>, lang: Lang, source: &[u8]) -> Option<TautologyKind> {
    if !is_assertion_node(node, lang) || assertion_name(node, lang, source).is_none() {
        return None;
    }
    if lang == Lang::Rust && node.kind() == "macro_invocation" {
        let name = call_name(node, source)?;
        return rust_macro_tautology(node, &name, source);
    }
    let first = if lang == Lang::Python && node.kind() == "assert_statement" {
        first_named_child(node)?
    } else {
        let argument = node.child_by_field_name("arguments")?;
        first_named_child(argument)?
    };
    if is_true_literal(first, lang, source) {
        return Some(TautologyKind::LiteralTrueAssertion);
    }
    let expression = if first.kind() == "binary_expression" || first.kind() == "comparison_operator"
    {
        first
    } else {
        return None;
    };
    let operator = expression.child_by_field_name("operator");
    let is_equal = operator.is_some_and(|operator| {
        let value = text(operator, source);
        matches!(value.trim(), "==" | "===")
    });
    if !is_equal {
        return None;
    }
    let left = expression.child_by_field_name("left")?;
    let right = expression.child_by_field_name("right")?;
    (text(left, source).trim() == text(right, source).trim())
        .then_some(TautologyKind::SelfEqualityAssertion)
}

fn rust_macro_tautology(node: Node<'_>, name: &str, source: &[u8]) -> Option<TautologyKind> {
    let mut cursor = node.walk();
    let token_tree = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "token_tree")?;
    let mut token_cursor = token_tree.walk();
    let arguments: Vec<Node<'_>> = token_tree.named_children(&mut token_cursor).collect();
    if let Some(first) = arguments.first().copied()
        && is_true_literal(first, Lang::Rust, source)
    {
        return Some(TautologyKind::LiteralTrueAssertion);
    }
    let is_equality_macro = matches!(
        name.rsplit([':', '>']).next(),
        Some("assert_eq") | Some("debug_assert_eq")
    );
    if is_equality_macro && arguments.len() == 2 {
        let first = arguments.first().copied()?;
        let second = arguments.get(1).copied()?;
        if text(first, source).trim() == text(second, source).trim() {
            return Some(TautologyKind::SelfEqualityAssertion);
        }
    }
    None
}

fn is_true_literal(node: Node<'_>, lang: Lang, source: &[u8]) -> bool {
    let kind_matches = match lang {
        Lang::Python => node.kind() == "true",
        Lang::Rust => node.kind() == "boolean_literal",
        Lang::Cpp | Lang::Cuda | Lang::TypeScript => {
            node.kind() == "true" || node.kind() == "boolean_literal"
        }
    };
    kind_matches && text(node, source).trim() == "true"
        || lang == Lang::Python && text(node, source).trim() == "True"
}

fn test_identity<'tree>(
    node: Node<'tree>,
    lang: Lang,
    source: &[u8],
) -> Option<(String, Node<'tree>)> {
    match lang {
        Lang::Rust if node.kind() == "function_item" => {
            if !has_rust_test_attribute(node, source) {
                return None;
            }
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source).into_owned())?;
            Some((qualified_identity(node, name, lang, source), node))
        }
        Lang::Python
            if matches!(
                node.kind(),
                "function_definition" | "async_function_definition"
            ) =>
        {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source).into_owned())?;
            let decorators =
                has_child_kind(node, "decorator") || has_child_kind(node, "decorated_definition");
            let conventional = name.starts_with("test_") || name.starts_with("test");
            if conventional || decorators && (name.starts_with("test_") || name.starts_with("test"))
            {
                Some((qualified_identity(node, name, lang, source), node))
            } else {
                None
            }
        }
        Lang::TypeScript if node.kind() == "call_expression" => {
            let name = call_name(node, source)?;
            if !is_test_invocation(&name) {
                return None;
            }
            let label = match first_string_argument(node, source) {
                Some(value) => value,
                None => format!("{}@{}", name, node.start_byte()),
            };
            Some((qualified_identity(node, label, lang, source), node))
        }
        Lang::Rust | Lang::Python | Lang::Cpp | Lang::Cuda | Lang::TypeScript => None,
    }
}

fn is_test_invocation(name: &str) -> bool {
    let mut tail_parts = name.rsplit('.');
    let tail = match tail_parts.next() {
        Some(value) => value.trim(),
        None => name.trim(),
    };
    matches!(tail, "test" | "it" | "specify")
        || matches!(name, "test.skip" | "test.todo" | "it.skip" | "it.todo")
}

fn has_rust_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    preceding_attributes(node)
        .iter()
        .any(|attribute| is_test_attribute(*attribute, source))
}

fn is_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    let value = text(node, source);
    let body = value
        .trim()
        .strip_prefix("#[")
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_default();
    let name = body.split(['(', '=']).next().unwrap_or_default().trim();
    matches!(name, "test" | "tokio::test" | "async_std::test")
}

fn preceding_attributes<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut attributes = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() != "attribute_item" {
            break;
        }
        attributes.push(sibling);
        previous = sibling.prev_named_sibling();
    }
    attributes.reverse();
    attributes
}

fn has_child_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() == kind)
}

fn qualified_identity(node: Node<'_>, own_name: String, lang: Lang, source: &[u8]) -> String {
    let mut parents = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        let mut scope_name = match lang {
            Lang::Rust => matches!(parent.kind(), "mod_item" | "impl_item" | "trait_item"),
            Lang::Python => matches!(parent.kind(), "class_definition" | "function_definition"),
            Lang::TypeScript => matches!(
                parent.kind(),
                "class_declaration" | "class_body" | "namespace_declaration"
            ),
            Lang::Cpp | Lang::Cuda => {
                matches!(parent.kind(), "namespace_definition" | "class_specifier")
            }
        };
        if lang == Lang::TypeScript && parent.kind() == "call_expression" {
            scope_name = call_name(parent, source)
                .is_some_and(|name| matches!(name.as_str(), "describe" | "suite"));
            if scope_name && let Some(label) = first_string_argument(parent, source) {
                parents.push(label);
            }
        } else if scope_name && let Some(name_node) = parent.child_by_field_name("name") {
            parents.push(text(name_node, source).into_owned());
        }
        current = parent.parent();
    }
    parents.reverse();
    parents.push(own_name);
    parents.join("::")
}

fn first_string_argument(node: Node<'_>, source: &[u8]) -> Option<String> {
    let args = node.child_by_field_name("arguments")?;
    let first = first_named_child(args)?;
    let kind = first.kind();
    if matches!(kind, "string" | "string_literal" | "template_string") {
        let value = text(first, source).into_owned();
        Some(
            value
                .trim_matches(|character| matches!(character, '\'' | '"' | '`'))
                .to_owned(),
        )
    } else {
        None
    }
}

fn markers_for(test: Node<'_>, lang: Lang, source: &[u8]) -> Vec<TestMarker> {
    let mut markers = Vec::new();
    let mut marker_nodes = Vec::new();
    marker_nodes.extend(preceding_attributes(test));
    let mut cursor = test.walk();
    marker_nodes.extend(
        test.named_children(&mut cursor)
            .filter(|child| child.kind() == "attribute_item" || child.kind() == "decorator"),
    );
    if lang == Lang::Python
        && let Some(parent) = test
            .parent()
            .filter(|parent| parent.kind() == "decorated_definition")
    {
        let mut parent_cursor = parent.walk();
        marker_nodes.extend(
            parent
                .named_children(&mut parent_cursor)
                .filter(|child| child.kind() == "decorator"),
        );
    }
    for child in marker_nodes {
        if child.kind() == "attribute_item" || child.kind() == "decorator" {
            let evidence = text(child, source).into_owned();
            let lowered = evidence.to_ascii_lowercase();
            let trimmed = lowered.trim();
            let body = trimmed
                .strip_prefix("#[")
                .and_then(|value| value.strip_suffix(']'))
                .or_else(|| trimmed.strip_prefix('@'))
                .unwrap_or_default();
            let name = body.split(['(', '=']).next().unwrap_or_default().trim();
            let kind = if name == "cfg" || name == "cfg_attr" {
                if lowered.contains("false") || lowered.contains("any()") {
                    MarkerKind::CfgDisabled
                } else {
                    MarkerKind::UnknownDisableMarker
                }
            } else if is_skip_name(name) || matches!(name, "ignore" | "disabled") {
                MarkerKind::IgnoreOrSkip
            } else if lang == Lang::Python && name.starts_with("pytest.mark.skip") {
                if name.ends_with("skipif") {
                    MarkerKind::UnknownDisableMarker
                } else {
                    MarkerKind::IgnoreOrSkip
                }
            } else if lang == Lang::Python && name.starts_with("unittest.skip") {
                MarkerKind::IgnoreOrSkip
            } else {
                continue;
            };
            markers.push(TestMarker {
                kind,
                node_kind: child.kind().to_owned(),
                span: span(child),
                evidence,
            });
        }
    }
    markers
}

fn assign_marker_tests(tests: &mut [TestObservation], markers: Vec<TestMarker>) {
    let mut test_order: Vec<(usize, usize, usize)> = tests
        .iter()
        .enumerate()
        .map(|(index, test)| (test.span.start, test.span.end, index))
        .collect();
    test_order.sort_unstable();
    let mut marker_order: Vec<TestMarker> = markers;
    marker_order.sort_by_key(|marker| (marker.span.start, marker.span.end));

    let mut next_test = 0usize;
    let mut active: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut expirations: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new();
    for marker in marker_order {
        while let Some((test_start, test_end, test_index)) = test_order.get(next_test).copied() {
            if test_start > marker.span.start {
                break;
            }
            if let Some(test) = tests.get(test_index)
                && test.span.end == test_end
            {
                active.insert((test.span.len(), test_index));
                expirations.push(Reverse((test.span.end, test_index)));
            }
            next_test = next_test.saturating_add(1);
        }
        while let Some(Reverse((end, test_index))) = expirations.peek().copied() {
            if end >= marker.span.end {
                break;
            }
            if expirations.pop().is_some()
                && let Some(test) = tests.get(test_index)
            {
                active.remove(&(test.span.len(), test_index));
            }
        }
        if let Some((_, index)) = active.iter().next()
            && let Some(test) = tests.get_mut(*index)
        {
            test.markers.push(marker);
        }
    }
}
