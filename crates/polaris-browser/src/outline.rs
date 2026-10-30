//! Accessibility trees as compact outlines: one indented line per meaningful node, a `ref` for
//! every node a tool can act on, and a hard byte budget.

use serde::Deserialize;

use crate::BrowserError;

/// Largest accessibility tree accepted, in bytes.
pub const MAX_TREE_BYTES: usize = 16 * 1024 * 1024;
/// Deepest nesting walked; deeper nodes are reported as truncated.
const MAX_DEPTH: usize = 64;
/// Longest name or value kept per node, in characters.
const MAX_LABEL_CHARS: usize = 120;

/// Roles a tool can act on; each gets a `ref`.
const INTERACTIVE: &[&str] = &[
    "button",
    "checkbox",
    "combobox",
    "link",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "textbox",
    "treeitem",
];

/// Roles that only group other nodes; without a name they are folded into their children.
const STRUCTURAL: &[&str] = &[
    "generic",
    "none",
    "presentation",
    "group",
    "section",
    "div",
    "LayoutTable",
    "LayoutTableRow",
    "LayoutTableCell",
];

/// One node as Chromium/Playwright accessibility snapshots describe it.
#[derive(Debug, Clone, Deserialize)]
pub struct A11yNode {
    /// ARIA role.
    #[serde(default)]
    pub role: String,
    /// Accessible name.
    #[serde(default)]
    pub name: String,
    /// Current value of an input, if any.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    /// Checked state of a checkbox, radio or switch.
    #[serde(default)]
    pub checked: Option<serde_json::Value>,
    /// Whether the node is disabled.
    #[serde(default)]
    pub disabled: bool,
    /// Whether the node has focus.
    #[serde(default)]
    pub focused: bool,
    /// Heading level.
    #[serde(default)]
    pub level: Option<u32>,
    /// Child nodes in document order.
    #[serde(default)]
    pub children: Vec<A11yNode>,
}

/// Limits of one outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutlineBudget {
    /// Largest outline text, in bytes.
    pub max_bytes: usize,
    /// Most lines emitted.
    pub max_lines: usize,
}

impl Default for OutlineBudget {
    fn default() -> Self {
        Self {
            max_bytes: 12 * 1024,
            max_lines: 400,
        }
    }
}

/// A node a tool can target by its `ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionTarget {
    /// `e1`, `e2`, … in document order.
    pub reference: String,
    /// ARIA role.
    pub role: String,
    /// Accessible name.
    pub name: String,
}

/// The compressed page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    /// Indented lines such as `- button "Save" [ref=e3]`.
    pub text: String,
    /// Nodes with a `ref`, in document order.
    pub targets: Vec<ActionTarget>,
    /// Nodes in the source tree.
    pub source_nodes: usize,
    /// Whether the budget or the depth limit cut the outline short.
    pub truncated: bool,
}

/// Parse a JSON accessibility tree (one root node) and compress it within `budget`.
/// # Errors
/// An oversized or malformed tree.
pub fn a11y_outline(tree: &[u8], budget: OutlineBudget) -> Result<Outline, BrowserError> {
    if tree.len() > MAX_TREE_BYTES {
        return Err(BrowserError::TooLarge {
            what: "accessibility tree",
            limit: MAX_TREE_BYTES,
        });
    }
    let root: A11yNode =
        serde_json::from_slice(tree).map_err(|_| BrowserError::Malformed("accessibility tree"))?;
    Ok(outline(&root, budget))
}

/// Compress an already parsed tree within `budget`.
#[must_use]
pub fn outline(root: &A11yNode, budget: OutlineBudget) -> Outline {
    let mut writer = Writer {
        budget,
        text: String::new(),
        lines: 0,
        targets: Vec::new(),
        source_nodes: 0,
        truncated: false,
    };
    writer.node(root, 0, 0);
    Outline {
        text: writer.text,
        targets: writer.targets,
        source_nodes: writer.source_nodes,
        truncated: writer.truncated,
    }
}

struct Writer {
    budget: OutlineBudget,
    text: String,
    lines: usize,
    targets: Vec<ActionTarget>,
    source_nodes: usize,
    truncated: bool,
}

impl Writer {
    fn node(&mut self, node: &A11yNode, indent: usize, depth: usize) {
        self.source_nodes = self.source_nodes.saturating_add(1);
        if depth >= MAX_DEPTH {
            self.truncated = true;
            return;
        }
        let name = label(&node.name);
        let interactive = INTERACTIVE.contains(&node.role.as_str());
        let structural = node.role.is_empty() || STRUCTURAL.contains(&node.role.as_str());
        // Plain text whose only content is its parent's name adds nothing.
        let redundant_text = matches!(node.role.as_str(), "text" | "StaticText") && name.is_empty();
        let shown = !redundant_text && !(structural && name.is_empty() && !interactive);
        let child_indent = if shown {
            self.line(node, &name, interactive, indent);
            indent.saturating_add(1)
        } else {
            indent
        };
        for child in &node.children {
            self.node(child, child_indent, depth.saturating_add(1));
        }
    }

    fn line(&mut self, node: &A11yNode, name: &str, interactive: bool, indent: usize) {
        let mut line = format!("{}- {}", "  ".repeat(indent), node.role);
        if !name.is_empty() {
            line.push_str(&format!(" \"{name}\""));
        }
        if let Some(level) = node.level {
            line.push_str(&format!(" [level={level}]"));
        }
        if let Some(value) = node.value.as_ref().and_then(scalar) {
            line.push_str(&format!(" [value=\"{}\"]", label(&value)));
        }
        if let Some(checked) = node.checked.as_ref().and_then(scalar) {
            line.push_str(&format!(" [checked={checked}]"));
        }
        if node.disabled {
            line.push_str(" [disabled]");
        }
        if node.focused {
            line.push_str(" [focused]");
        }
        let reference = interactive.then(|| format!("e{}", self.targets.len().saturating_add(1)));
        if let Some(reference) = &reference {
            line.push_str(&format!(" [ref={reference}]"));
        }
        line.push('\n');
        if self.lines >= self.budget.max_lines
            || self.text.len().saturating_add(line.len()) > self.budget.max_bytes
        {
            self.truncated = true;
            return;
        }
        self.text.push_str(&line);
        self.lines = self.lines.saturating_add(1);
        if let Some(reference) = reference {
            self.targets.push(ActionTarget {
                reference,
                role: node.role.clone(),
                name: name.to_owned(),
            });
        }
    }
}

/// Whitespace collapsed, quotes escaped, cut to [`MAX_LABEL_CHARS`].
fn label(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed
        .chars()
        .take(MAX_LABEL_CHARS)
        .collect::<String>()
        .replace('"', "\\\"");
    if collapsed.chars().count() > MAX_LABEL_CHARS {
        out.push('…');
    }
    out
}

fn scalar(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.is_empty() => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}
