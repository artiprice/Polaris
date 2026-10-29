//! Descriptor: specs/descriptors/codeintel_language_detection.json

use polaris_code_intel::{Lang, detect_lang};
use std::path::Path;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn extension_maps_each_language() -> TestResult {
    let cases = [
        ("src/lib.rs", Lang::Rust),
        ("tool.py", Lang::Python),
        ("stubs/mod.pyi", Lang::Python),
        ("a.cpp", Lang::Cpp),
        ("a.cc", Lang::Cpp),
        ("a.cxx", Lang::Cpp),
        ("a.hpp", Lang::Cpp),
        ("a.hh", Lang::Cpp),
        ("a.hxx", Lang::Cpp),
        ("kernel.cu", Lang::Cuda),
        ("kernel.cuh", Lang::Cuda),
        ("index.ts", Lang::TypeScript),
        ("index.mts", Lang::TypeScript),
        ("index.cts", Lang::TypeScript),
    ];
    for (path, want) in cases {
        assert_eq!(detect_lang(Path::new(path), b""), Some(want), "{path}");
    }
    Ok(())
}

#[test]
fn c_sources_and_headers_use_the_cpp_grammar() -> TestResult {
    assert_eq!(detect_lang(Path::new("x.c"), b""), Some(Lang::Cpp));
    assert_eq!(detect_lang(Path::new("x.h"), b""), Some(Lang::Cpp));
    Ok(())
}

#[test]
fn extension_is_case_insensitive_and_path_agnostic() -> TestResult {
    assert_eq!(
        detect_lang(
            Path::new("dir with spaces/\u{444}\u{430}\u{439}\u{43b}.RS"),
            b""
        ),
        Some(Lang::Rust)
    );
    assert_eq!(
        detect_lang(Path::new("C:\\proj dir\\K.CU"), b""),
        Some(Lang::Cuda)
    );
    Ok(())
}

#[test]
fn shebang_detects_python_without_extension() -> TestResult {
    assert_eq!(
        detect_lang(
            Path::new("scripts/run"),
            b"#!/usr/bin/env python3\nprint(1)\n"
        ),
        Some(Lang::Python)
    );
    assert_eq!(
        detect_lang(Path::new("run"), b"#!/usr/bin/python\n"),
        Some(Lang::Python)
    );
    Ok(())
}

#[test]
fn shebang_survives_a_utf8_bom() -> TestResult {
    let head = b"\xEF\xBB\xBF#!/usr/bin/env python\n";
    assert_eq!(detect_lang(Path::new("run"), head), Some(Lang::Python));
    Ok(())
}

#[test]
fn known_extension_wins_over_shebang() -> TestResult {
    assert_eq!(
        detect_lang(Path::new("odd.rs"), b"#!/usr/bin/env python\n"),
        Some(Lang::Rust)
    );
    Ok(())
}

#[test]
fn unknown_files_yield_none() -> TestResult {
    assert_eq!(detect_lang(Path::new("README.md"), b"# title\n"), None);
    assert_eq!(detect_lang(Path::new("Makefile"), b"all:\n"), None);
    assert_eq!(detect_lang(Path::new("run"), b"#!/bin/sh\n"), None);
    assert_eq!(detect_lang(Path::new(""), b""), None);
    Ok(())
}

#[test]
fn tsx_is_not_claimed_by_the_typescript_grammar() -> TestResult {
    assert_eq!(detect_lang(Path::new("view.tsx"), b""), None);
    Ok(())
}
