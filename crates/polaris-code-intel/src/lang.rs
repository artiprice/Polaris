//! Supported languages and language detection.

use std::path::Path;

/// A language with a bundled tree-sitter grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    /// Rust (`tree-sitter-rust`).
    Rust,
    /// Python (`tree-sitter-python`).
    Python,
    /// C++, also used for C sources and headers (`tree-sitter-cpp`).
    Cpp,
    /// CUDA C++ (`tree-sitter-cuda`).
    Cuda,
    /// TypeScript without JSX (`tree-sitter-typescript`, `typescript` dialect).
    TypeScript,
}

impl Lang {
    /// Every supported language, in a fixed order.
    pub const ALL: [Lang; 5] = [
        Lang::Rust,
        Lang::Python,
        Lang::Cpp,
        Lang::Cuda,
        Lang::TypeScript,
    ];

    /// A short lower-case name for logs and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Cpp => "cpp",
            Self::Cuda => "cuda",
            Self::TypeScript => "typescript",
        }
    }

    /// The tree-sitter language ABI version the grammar crate was generated with.
    #[must_use]
    pub fn abi_version(self) -> usize {
        self.grammar().abi_version()
    }

    /// The grammar as a tree-sitter language handle.
    pub(crate) fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Cuda => tree_sitter_cuda::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }
}

/// The UTF-8 byte order mark.
pub(crate) const BOM: &[u8] = b"\xEF\xBB\xBF";

/// How many bytes of the head are inspected for a shebang line.
const SHEBANG_SCAN: usize = 256;

/// Detects the language of a file from its extension, falling back to a shebang line in `head`.
///
/// A known extension always wins over a shebang. `.c` and `.h` map to [`Lang::Cpp`] (the C++
/// grammar parses C closely enough for outlines); `.tsx` is not claimed because the bundled
/// TypeScript grammar cannot parse JSX. The extension match is ASCII case-insensitive and the
/// path is never touched on disk.
///
/// ```
/// use polaris_code_intel::{Lang, detect_lang};
/// use std::path::Path;
/// assert_eq!(detect_lang(Path::new("src/lib.rs"), b""), Some(Lang::Rust));
/// assert_eq!(detect_lang(Path::new("run"), b"#!/usr/bin/env python3\n"), Some(Lang::Python));
/// assert_eq!(detect_lang(Path::new("notes.md"), b""), None);
/// ```
#[must_use]
pub fn detect_lang(path: &Path, head: &[u8]) -> Option<Lang> {
    let by_extension = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(language_for_extension);
    by_extension.or_else(|| language_for_shebang(head))
}

fn language_for_extension(ext: &str) -> Option<Lang> {
    const TABLE: [(&str, Lang); 17] = [
        ("rs", Lang::Rust),
        ("py", Lang::Python),
        ("pyi", Lang::Python),
        ("pyw", Lang::Python),
        ("cpp", Lang::Cpp),
        ("cc", Lang::Cpp),
        ("cxx", Lang::Cpp),
        ("hpp", Lang::Cpp),
        ("hh", Lang::Cpp),
        ("hxx", Lang::Cpp),
        ("c", Lang::Cpp),
        ("h", Lang::Cpp),
        ("cu", Lang::Cuda),
        ("cuh", Lang::Cuda),
        ("ts", Lang::TypeScript),
        ("mts", Lang::TypeScript),
        ("cts", Lang::TypeScript),
    ];
    TABLE
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(ext))
        .map(|(_, lang)| *lang)
}

fn language_for_shebang(head: &[u8]) -> Option<Lang> {
    let head = head.strip_prefix(BOM).unwrap_or(head);
    let rest = head.strip_prefix(b"#!")?;
    let window = rest.get(..rest.len().min(SHEBANG_SCAN)).unwrap_or(rest);
    let line = window.split(|&b| b == b'\n').next().unwrap_or(window);
    let line = String::from_utf8_lossy(line).to_ascii_lowercase();
    line.contains("python").then_some(Lang::Python)
}
