# Mouse selection geometry evidence

Date: 2026-10-25T12:18:19.726Z

## Definition of done
Visible transcript drag endpoints clamp to a visible text row and legal grapheme boundary; strict source hits remain source-backed. Composer and questionnaire geometry expose exact source spans without treating padding as text. Prepared, cached, and rendered metadata agree for visible source rows.

## Verified commands

    cargo test --manifest-path crates/polaris-ui/crates/cli/Cargo.toml --test render_contract
    running 37 tests
    test result: ok. 37 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

    cargo test --manifest-path crates/polaris-ui/crates/cli/Cargo.toml --test editor_geometry
    running 4 tests
    test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

    cargo test --manifest-path crates/polaris-ui/crates/cli/Cargo.toml --test editor_render
    running 1 test
    test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

    rustfmt --edition 2024 --check <four owned Rust files>
    exit=0

    git diff --check -- <four owned Rust files>
    exit=0

## Observed coverage
- Transcript hit is strict on source cells; drag endpoint uses nearest visible source row, clamps to legal row endpoints, resolves wide graphemes and tabs, and returns None outside transcript body or over the ToLatest label.
- Selection tint does not affect blank transcript filler.
- prepare_metadata, cached metadata, and rendered metadata contain equivalent source spans and cells.
- Composer source hits distinguish text, tabs, wide graphemes, chip content, and filler while composer_cursor_at preserves previous caret mapping.
- Editor source-cell lookup returns only cells occupied by grapheme, tab, or chip spans.

## Scope
Changed: crates/polaris-ui/crates/cli/src/screen.rs, crates/polaris-ui/crates/cli/src/editor.rs, crates/polaris-ui/crates/cli/tests/render_contract.rs, crates/polaris-ui/crates/cli/tests/editor_geometry.rs.
