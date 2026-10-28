//! Declaration discovery: which syntax nodes declare symbols, per language.
//!
//! [`classify`] decides whether one node is a declaration (unwrapping `export`, decorators and
//! `template<...>` wrappers) and [`declarations`] collects every declaration of a file with an
//! explicit stack, so hostile nesting cannot overflow the call stack.

use crate::error::CodeIntelError;
use crate::lang::Lang;
use crate::parse::{ByteSpan, Parsed};
use crate::walk::{field_text, named_children, node_text};
use tree_sitter::Node;

/// What kind of symbol a declaration introduces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolKind {
    /// A free function (or a TypeScript arrow-function constant).
    Function,
    /// A function declared inside a class, struct, trait, impl or interface.
    Method,
    /// A class.
    Class,
    /// A struct or union.
    Struct,
    /// An enum.
    Enum,
    /// A Rust trait.
    Trait,
    /// A TypeScript interface.
    Interface,
    /// A Rust `impl` block.
    Impl,
    /// A module or namespace.
    Module,
    /// A type alias.
    TypeAlias,
    /// A constant or static.
    Const,
    /// A macro definition.
    Macro,
    /// Top-level code that is not a declaration (imports, statements, comments).
    Other,
}

/// A symbol identified by its qualified name and kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolRef {
    /// Qualified name, scopes joined with `::` (for example `Foo::bar`).
    pub name: String,
    /// The kind of symbol.
    pub kind: SymbolKind,
}

/// One declaration node, resolved against its language.
#[derive(Debug, Clone)]
pub(crate) struct Classified<'t> {
    pub kind: SymbolKind,
    pub name: String,
    /// The outermost node of the declaration, including `export` / decorator / template wrappers.
    pub outer: Node<'t>,
    /// The declaring node proper.
    pub inner: Node<'t>,
    /// The body that a skeleton elides.
    pub body: Option<Node<'t>>,
    /// The node whose named children are nested declarations (classes, impls, modules).
    pub members: Option<Node<'t>>,
    /// Where the signature ends when it does not end at the body.
    pub head_end: Option<usize>,
}

/// A declaration with plain offsets, in source order.
#[derive(Debug, Clone)]
pub(crate) struct DeclInfo {
    pub kind: SymbolKind,
    pub name: String,
    pub qualified: String,
    pub depth: u32,
    pub span: ByteSpan,
    pub sig_start: usize,
    pub sig_end: usize,
    pub body: Option<ByteSpan>,
    pub is_container: bool,
}

/// Wrapper depth that is unwrapped at most (`export` + `declare` + the declaration itself).
const MAX_WRAPPERS: usize = 4;
/// Longest declarator chain followed when looking for a C++ function name.
const MAX_DECLARATOR_CHAIN: usize = 64;

/// Strips wrapper nodes (decorators, `export`, `template<...>`, ambient declarations).
fn unwrap_wrapper<'t>(lang: Lang, outer: Node<'t>) -> Option<Node<'t>> {
    let mut node = outer;
    for _ in 0..MAX_WRAPPERS {
        let next = match (lang, node.kind()) {
            (Lang::Python, "decorated_definition") => node.child_by_field_name("definition"),
            (Lang::Cpp | Lang::Cuda, "template_declaration") => named_children(node)
                .into_iter()
                .rfind(|c| !matches!(c.kind(), "template_parameter_list" | "comment")),
            (Lang::TypeScript, "export_statement") => node.child_by_field_name("declaration"),
            (Lang::TypeScript, "ambient_declaration") => named_children(node).into_iter().next(),
            (Lang::TypeScript, "expression_statement") => named_children(node)
                .into_iter()
                .next()
                .filter(|c| matches!(c.kind(), "internal_module" | "module")),
            _ => return Some(node),
        };
        node = next?;
    }
    Some(node)
}

/// Resolves `outer` to a declaration of `lang`, or `None` if it declares nothing we track.
pub(crate) fn classify<'t>(lang: Lang, outer: Node<'t>, src: &[u8]) -> Option<Classified<'t>> {
    let inner = unwrap_wrapper(lang, outer)?;
    let spec = match lang {
        Lang::Rust => rust_spec(inner, src),
        Lang::Python => python_spec(inner, src),
        Lang::Cpp | Lang::Cuda => cpp_spec(inner, src),
        Lang::TypeScript => typescript_spec(inner, src),
    }?;
    Some(Classified {
        kind: spec.kind,
        name: spec.name,
        outer,
        inner,
        body: spec.body,
        members: spec.members,
        head_end: spec.head_end,
    })
}

struct Spec<'t> {
    kind: SymbolKind,
    name: String,
    body: Option<Node<'t>>,
    members: Option<Node<'t>>,
    head_end: Option<usize>,
}

impl<'t> Spec<'t> {
    fn leaf(kind: SymbolKind, name: String) -> Self {
        Self {
            kind,
            name,
            body: None,
            members: None,
            head_end: None,
        }
    }

    fn with_body(kind: SymbolKind, name: String, body: Option<Node<'t>>) -> Self {
        Self {
            kind,
            name,
            body,
            members: None,
            head_end: None,
        }
    }

    fn container(kind: SymbolKind, name: String, body: Option<Node<'t>>) -> Self {
        Self {
            kind,
            name,
            body,
            members: body,
            head_end: None,
        }
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rust_spec<'t>(node: Node<'t>, src: &[u8]) -> Option<Spec<'t>> {
    let name = || field_text(node, "name", src);
    let body = || node.child_by_field_name("body");
    Some(match node.kind() {
        "function_item" => Spec::with_body(SymbolKind::Function, name()?, body()),
        "function_signature_item" => Spec::leaf(SymbolKind::Function, name()?),
        "struct_item" | "union_item" => {
            // Tuple-struct fields are part of the signature; only braced bodies are elided.
            let braces = body().filter(|b| b.kind() == "field_declaration_list");
            Spec::with_body(SymbolKind::Struct, name()?, braces)
        }
        "enum_item" => Spec::with_body(SymbolKind::Enum, name()?, body()),
        "trait_item" => Spec::container(SymbolKind::Trait, name()?, body()),
        "impl_item" => {
            let ty = node.child_by_field_name("type")?;
            let ty = collapse(&node_text(ty, src));
            let full = match node.child_by_field_name("trait") {
                Some(tr) => format!("{} for {ty}", collapse(&node_text(tr, src))),
                None => ty,
            };
            Spec::container(SymbolKind::Impl, full, body())
        }
        "mod_item" => Spec::container(SymbolKind::Module, name()?, body()),
        "type_item" => Spec::leaf(SymbolKind::TypeAlias, name()?),
        "const_item" | "static_item" => Spec::leaf(SymbolKind::Const, name()?),
        "macro_definition" => {
            let n = node.child_by_field_name("name")?;
            Spec {
                head_end: Some(n.end_byte()),
                ..Spec::leaf(SymbolKind::Macro, node_text(n, src).into_owned())
            }
        }
        _ => return None,
    })
}

fn python_spec<'t>(node: Node<'t>, src: &[u8]) -> Option<Spec<'t>> {
    let name = field_text(node, "name", src)?;
    let body = node.child_by_field_name("body");
    match node.kind() {
        "function_definition" => Some(Spec::with_body(SymbolKind::Function, name, body)),
        "class_definition" => Some(Spec::container(SymbolKind::Class, name, body)),
        _ => None,
    }
}

/// Follows `declarator` fields down to the declared name; also reports whether a function
/// declarator was crossed on the way.
fn cpp_declarator_name(node: Node<'_>, src: &[u8]) -> Option<(String, bool)> {
    let mut d = node.child_by_field_name("declarator")?;
    let mut saw_function = false;
    for _ in 0..MAX_DECLARATOR_CHAIN {
        let next = match d.kind() {
            "function_declarator" => {
                saw_function = true;
                d.child_by_field_name("declarator")
            }
            "pointer_declarator"
            | "array_declarator"
            | "parenthesized_declarator"
            | "attributed_declarator"
            | "reference_declarator" => d
                .child_by_field_name("declarator")
                .or_else(|| named_children(d).into_iter().last()),
            _ => return Some((collapse(&node_text(d, src)), saw_function)),
        };
        d = next?;
    }
    None
}

fn cpp_spec<'t>(node: Node<'t>, src: &[u8]) -> Option<Spec<'t>> {
    let body = || node.child_by_field_name("body");
    match node.kind() {
        "function_definition" => {
            let (name, _) = cpp_declarator_name(node, src)?;
            Some(Spec::with_body(SymbolKind::Function, name, body()))
        }
        "declaration" | "field_declaration" => {
            let (name, saw_function) = cpp_declarator_name(node, src)?;
            saw_function.then(|| Spec::leaf(SymbolKind::Function, name))
        }
        "class_specifier" | "struct_specifier" | "union_specifier" => {
            let name = field_text(node, "name", src)?;
            let body = body()?;
            let kind = if node.kind() == "class_specifier" {
                SymbolKind::Class
            } else {
                SymbolKind::Struct
            };
            Some(Spec::container(kind, collapse(&name), Some(body)))
        }
        "enum_specifier" => {
            let name = field_text(node, "name", src)?;
            Some(Spec::with_body(SymbolKind::Enum, name, Some(body()?)))
        }
        "namespace_definition" => {
            let name = field_text(node, "name", src).unwrap_or_else(|| "(anonymous)".to_owned());
            Some(Spec::container(SymbolKind::Module, collapse(&name), body()))
        }
        "alias_declaration" => Some(Spec::leaf(
            SymbolKind::TypeAlias,
            field_text(node, "name", src)?,
        )),
        _ => None,
    }
}

fn typescript_spec<'t>(node: Node<'t>, src: &[u8]) -> Option<Spec<'t>> {
    let name = || field_text(node, "name", src);
    let body = || node.child_by_field_name("body");
    Some(match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            Spec::with_body(SymbolKind::Function, name()?, body())
        }
        "function_signature" => Spec::leaf(SymbolKind::Function, name()?),
        "class_declaration" | "abstract_class_declaration" => {
            Spec::container(SymbolKind::Class, name()?, body())
        }
        "interface_declaration" => Spec::container(SymbolKind::Interface, name()?, body()),
        "type_alias_declaration" => Spec::leaf(SymbolKind::TypeAlias, name()?),
        "enum_declaration" => Spec::with_body(SymbolKind::Enum, name()?, body()),
        "internal_module" | "module" => Spec::container(SymbolKind::Module, name()?, body()),
        "method_definition" => Spec::with_body(SymbolKind::Method, name()?, body()),
        "method_signature" | "abstract_method_signature" => Spec::leaf(SymbolKind::Method, name()?),
        "lexical_declaration" | "variable_declaration" => {
            let decl = named_children(node).into_iter().find(|c| {
                c.kind() == "variable_declarator"
                    && c.child_by_field_name("value").is_some_and(|v| {
                        matches!(
                            v.kind(),
                            "arrow_function" | "function_expression" | "function"
                        )
                    })
            })?;
            let value = decl.child_by_field_name("value")?;
            Spec::with_body(
                SymbolKind::Function,
                field_text(decl, "name", src)?,
                value.child_by_field_name("body"),
            )
        }
        _ => return None,
    })
}

/// Whether a node only groups declarations (preprocessor blocks, `extern "C"`), so the walk looks
/// through it without treating it as a scope.
fn is_transparent(lang: Lang, kind: &str) -> bool {
    matches!(lang, Lang::Cpp | Lang::Cuda)
        && matches!(
            kind,
            "preproc_if"
                | "preproc_ifdef"
                | "preproc_else"
                | "preproc_elif"
                | "preproc_elifdef"
                | "linkage_specification"
                | "declaration_list"
        )
}

pub(crate) fn is_type_scope(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Trait
            | SymbolKind::Impl
            | SymbolKind::Interface
    )
}

/// Every declaration of a file in source order (nested ones directly after their parent).
///
/// # Errors
/// [`CodeIntelError::TooLarge`] if the file has more than `max_items` declarations.
pub(crate) fn declarations(p: &Parsed, max_items: usize) -> Result<Vec<DeclInfo>, CodeIntelError> {
    let lang = p.lang();
    let src = p.source();
    let mut out: Vec<DeclInfo> = Vec::new();
    // Stack entries: node, depth, index of the parent declaration.
    let mut stack: Vec<(Node<'_>, u32, Option<usize>)> = named_children(p.tree().root_node())
        .into_iter()
        .rev()
        .map(|n| (n, 0, None))
        .collect();
    while let Some((node, depth, parent)) = stack.pop() {
        if is_transparent(lang, node.kind()) {
            stack.extend(
                named_children(node)
                    .into_iter()
                    .rev()
                    .map(|n| (n, depth, parent)),
            );
            continue;
        }
        let Some(c) = classify(lang, node, src) else {
            continue;
        };
        if out.len() >= max_items {
            return Err(CodeIntelError::TooLarge { limit: max_items });
        }
        let parent_info = parent.and_then(|i| out.get(i));
        let kind = match (c.kind, parent_info) {
            (SymbolKind::Function, Some(p)) if is_type_scope(p.kind) => SymbolKind::Method,
            (k, _) => k,
        };
        let qualified = match parent_info {
            Some(p) => format!("{}::{}", p.qualified, c.name),
            None => c.name.clone(),
        };
        let sig_start = if lang == Lang::Python {
            c.inner.start_byte()
        } else {
            c.outer.start_byte()
        };
        let sig_end = c
            .head_end
            .or_else(|| c.body.map(|b| b.start_byte()))
            .unwrap_or_else(|| c.outer.end_byte())
            .max(sig_start);
        let index = out.len();
        out.push(DeclInfo {
            kind,
            name: c.name.clone(),
            qualified,
            depth,
            span: ByteSpan::new(c.outer.start_byte(), c.outer.end_byte()),
            sig_start,
            sig_end,
            body: c.body.map(|b| ByteSpan::new(b.start_byte(), b.end_byte())),
            is_container: c.members.is_some(),
        });
        if let Some(members) = c.members {
            let child_depth = depth
                .checked_add(1)
                .ok_or(CodeIntelError::ArithmeticInvariant("declaration depth"))?;
            for child in named_children(members).into_iter().rev() {
                stack.push((child, child_depth, Some(index)));
            }
        }
    }
    Ok(out)
}
