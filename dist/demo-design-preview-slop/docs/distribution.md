# Standalone Windows executable

The packaged application is dist/polaris.exe, with SHA256SUMS.txt and build-info.json describing its exact bytes. Identical copies are available in dist/word-selection, dist/message-presentation and dist/overlay-surfaces. Copy that one file to the destination computer and launch it in a terminal:

    .\polaris.exe --preview --interactive

No Cargo, Rust installation, project files, GCC installation, or separately shipped DLLs are needed at runtime. This was verified on the current Windows 11 x64 machine with a PATH containing only Windows System32, from a directory containing only polaris.exe.

## Runtime boundary

Rust dependencies and MinGW support libraries are linked statically. Windows system DLLs remain imports, including Windows' built-in Universal C Runtime (UCRT). The release does not import VCRUNTIME140.dll, MSVCP140.dll, libgcc_s_seh-1.dll, or libwinpthread-1.dll.

The C runtime is therefore not completely embedded. The available MSVC installation lacks its static CRT libraries; the portable release uses the installed GNU/UCRT toolchain. Other Windows versions and clean virtual machines have not been verified.

## Rebuild and package

Build prerequisites are Rust with the x86_64-pc-windows-gnu target, a Windows GNU compiler/linker named gcc on PATH, and Python 3. The local build used MSYS2 UCRT64 GCC.

    rustup target add x86_64-pc-windows-gnu
    python scripts/build_static.py --test

Add --offline when Cargo dependencies are cached:

    python scripts/build_static.py --offline --test

The verified offline wrapper works from any current directory, temporarily sets RUST_TEST_THREADS=1, restores the caller's environment/location, and propagates build or test failure:

    .\scripts\build_verified.ps1

The script builds, optionally runs the release test suite, checks the exact resulting executable's PE imports, then copies that executable to dist/polaris.exe. It also writes dist/SHA256SUMS.txt and dist/build-info.json. Unknown non-system DLL imports and delay imports fail packaging. The manifest and checksum describe the packaged bytes.

cargo static builds the executable without packaging it:

    cargo static --offline

The GNU target flags and Cargo alias are in .cargo/config.toml. They do not change the development build target. Release symbols are stripped through Cargo.toml.

## Verification

    python -m unittest discover -s scripts -p test_static_binary.py
    Ran 5 tests
    OK

The gate accepts system DLLs and OS UCRT contracts; rejects Visual C++, GCC, pthread, and unknown runtime DLLs; rejects delay imports; and rejects malformed/wrong-architecture input. The previous MSVC debug executable was also checked and rejected with:

    ValueError: Non-system DLL dependencies: VCRUNTIME140.dll

The Enter-to-apply GNU release build passed 267 top-level tests, including twenty-two real ConPTY input/mouse smoke tests. Its two ignored child entry points are executed by parent tests in isolated consoles.

Raw output: [static build and tests](../target/settings-enter-to-apply/release-build.log). Packaged executable imports and checksum: [build information](../dist/build-info.json). See [Settings completion evidence](settings-enter-to-apply-verification.md).

After packaging, the exact distributed file was copied to an otherwise empty directory and launched with only Windows System32 on PATH. All thirteen version/help, welcome, Settings and Usage dump checks exited 0, including Settings at 60x20, 80x24, 120x40 and 160x60, ASCII style previews, terminal-native/ANSI Settings and no-color Usage. Startup did not write preferences. Raw results: [package and standalone checks](../target/settings-enter-to-apply/verification.json).

## Previous branding

The active executable and Cargo package are polaris, without an nvvi compatibility alias. Prior executable bytes and matching manifests are preserved under dist/legacy-nvvi; archive.json lists the hashes and original locations. The physical source directory remains C:\mDev\nvvi-code. See [branding and rollback](branding/README.md).

The first Polaris pixel-star executable and matching manifests are preserved in dist/legacy-polaris-pixel/68bf2a4ca77c. The active CLI package contains the minimal North Star artwork. The preceding microcircuit renderer and tests are archived in docs/branding/microcircuit-north-star.

The previous hybrid executable and manifests are preserved under dist/legacy-color-policy/e48c4cb8c351. Default Windows dark sessions now use the RGB palette without requiring terminal environment hints. See [color policy](color-policy.md).

The cursor-presentation fix preserves the accepted color palette and does not alter fonts or cursor shape. The previous executable and matching manifests are preserved under dist/legacy-cursor/34e115e9e61f. See [composer cursor behavior](composer.md#cursor-presentation).

The current CLI redesign is packaged in all four active distribution locations. The preceding cursor-fix build and matching manifests are preserved under dist/legacy-selection/80fd0513bd7d. The selection change preserves the accepted palette elsewhere, logo geometry, fonts and cursor presentation.

The stronger 30% selection build replaces the initial faint 10% tint. That initial selection build is preserved under dist/legacy-selection/b9594d381b5f with matching manifests.

The prior active copies and matching manifests are preserved under dist/legacy-cli-redesign, grouped by SHA256 prefix and original location. Current copy hashes and archive locations are recorded in target/cli-redesign-review/distribution-sync.json. The redesign does not change runtime DLL requirements.

The enclosed-composer package is preserved in dist/legacy-composer-open-sides/eb3919e56c30. The open-sided source preview is built and verified at target/composer-open-sides/standalone/polaris.exe, with evidence in target/composer-open-sides/verification.json. That revision could not replace a running distribution instance; the later full-width trial successfully updated all four distribution locations. The preview keeps horizontal rules with open sides; layout, growth and input coordinates are unchanged.

The full-width trial preserves two-cell outer margins and has no 100-cell width cap. Source/test/doc snapshots and the preceding centered/open-sided executable are in target/full-width-trial/before and target/full-width-trial/centered-polaris.exe. Distribution copy hashes and archive paths are in target/full-width-trial/distribution-sync.json.

The three-style Usage release and its package checks are recorded in target/usage-redesign/verification.json. Previous executables and matching manifests are preserved under dist/legacy-usage, grouped by SHA256 prefix and original location. Source/test/doc snapshots are in target/usage-redesign/before. No live metrics source or provider quota policy is connected.

## Agent work view release

The agent UI preview release passed 287 top-level release tests, including the new ConPTY agent navigation test, and 25 standalone launch/dump checks. All four active distribution copies share the packaged SHA-256 in dist/build-info.json. Previous packages and matching manifests are preserved in dist/legacy-agents/5081032d89de. See [agent verification](agents-verification.md) for commands, raw logs, capture links and the UI-only boundary.

## Stats rebuild release

The rebuilt Stats release passed 304 top-level release tests and 35 standalone checks. All four active package locations share the hash in dist/build-info.json. The previous agent-view build and matching manifests are preserved under dist/legacy-stats/d7c037263ad4. See [Stats verification](stats-verification.md) for the exact commands, captures, raw output and preview-only boundary.

## Solid calendar release

The touching activity-calendar release passed 309 top-level release tests and 14 standalone launch checks. All four active distribution directories contain identical executable bytes with matching checksum and build manifests. The previous packages are preserved under dist/legacy-calendar-fill/0f1039c73805. See [calendar verification](calendar-fill-verification.md) for raw output, captures, fallback coverage and runtime limitations.

## Agent symbol release

The symbol-only release uses ● for Main and ◯ for working subagents. It passed 310 top-level release tests and 10 standalone checks; all four distribution copies and manifests match. Previous packages are archived under dist/legacy-agent-symbols/d9738bd21edf. See [agent symbol verification](agent-symbols-verification.md).

## Rendering performance release

The rendering refactor passed 322 top-level release tests and 42 standalone launch checks. All four active package locations contain identical tested bytes with matching manifests. Previous packages are preserved under dist/legacy-rendering/46a64f7c8cef. See [rendering verification](rendering-verification.md) for raw results, before/after timings, screen comparisons and terminal limitations.
