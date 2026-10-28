# Settings Enter-to-apply verification

Verified on 2026-10-21 against the current workspace and exact packaged GNU release.

## Implemented behavior

Settings keeps its highlighted candidate separate from applied preferences. Arrows and mouse clicks only highlight; Enter applies exactly that choice and saves persistent fields. Space does nothing. Right clamps at the final variant, and Left from the first returns to categories. Changing rows, leaving controls, or closing Settings discards unconfirmed candidates. Confirmed values remain active.

Usage retains the three labeled 42% sample rows and the shared Usage renderer. Its applied [x] marker stays independent of candidate focus. Ordinary controls use applied markers and an explicit > candidate marker, including no-color mode. Enter retries failed saves and explicitly confirms launch overrides without saving unrelated overrides. Interaction mode remains session-only; Manage providers opens only on Enter.

The Settings sheet reserves the protected header rows while retaining its shortest-window minimum. The four modifier-sensitive overlay Return fixtures use explicit unmodified CSI-u Enter; production legacy Shift recovery remains intact. Two additional ConPTY text assertions now use the existing terminal-screen decoder because cursor-motion sequences can split visible words in raw transport bytes. The visible-text, screen, modifier and model assertions remain in place.

The build wrapper resolves the repository from its own location, runs offline release tests with RUST_TEST_THREADS=1, propagates failures, and restores the caller's environment and location. It was also invoked successfully from docs/ rather than the repository root.

## Observed verification

Commands and observed results (suite totals aggregate their per-target results):

```text
cargo fmt --all -- --check
(exit 0)

cargo clippy --offline --locked --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo]

cargo test --offline --locked -- --test-threads=1
267 top-level passed; 0 failed; 2 ignored child entry points

scripts/build_verified.ps1
267 top-level release tests passed; 0 failed; 2 ignored child entry points

scripts/test_build_verified.ps1
PASS: wrapper arguments, serial tests, success/failure exit codes, environment/location restoration, launch exception

python -m unittest discover -s scripts -p test_static_binary.py
Ran 5 tests
OK

python target/settings-enter-to-apply/verify_release.py
"active_copies": 4
"startup_checks": 13
"top_level_passed": 267
```

The two ignored entries are child-fixture entry points exercised by their parent tests. Including those child runs, each full suite recorded 314 passes. The 23 Settings/persistence tests cover unchanged intermediate values and files, exact confirmation, candidate discard, mouse behavior, no-op confirmations, launch overrides, failed-save retry, session-only actions and restart behavior. Rendering checks cover 60x20, 80x24, 120x40 and 160x60 with Dark/terminal-native, color/no-color and ASCII/Unicode variants. Narrow/wide pending-choice and save-error PNGs were inspected as rendered Ratatui buffers.

Full evidence:

- [Development suite](../target/settings-enter-to-apply/development-tests.log)
- [Static release build and suite](../target/settings-enter-to-apply/release-build.log)
- [Wrapper verification](../target/settings-enter-to-apply/wrapper-tests.log)
- [Machine-readable package/startup checks](../target/settings-enter-to-apply/verification.json)
- [Narrow pending choice](../target/settings-enter-to-apply/pending-theme-60x20.png), [wide Usage candidate](../target/settings-enter-to-apply/pending-usage-120x40.png), [save error](../target/settings-enter-to-apply/save-error-80x24.png)

## Package and consistency

dist/polaris.exe matches the exact release-build executable byte for byte. Its SHA256 is:

```text
5081032d89de0ccf44005ca41e8bb4cb5e5799d1dbbde11dfaec93e6bfbb1467
```

The executable, build-info.json and SHA256SUMS.txt were synchronized and checked in dist/, dist/word-selection/, dist/message-presentation/ and dist/overlay-surfaces/. Legacy archive directories were left intact. The PE gate accepts only Windows system imports, including OS UCRT contracts. The isolated startup directory contains only polaris.exe; all 13 version/help/Settings/Usage dumps passed with only Windows System32 on PATH and without writing preferences.

README, Settings, Usage, UI-design and distribution documentation and in-app Help agree with Enter-to-apply. Existing unrelated workspace changes were preserved.

## Remaining limits

Physical Alacritty keyboard/font/mouse appearance was not observed. Other Windows versions and clean virtual machines were not checked. This remains a UI preview without live inference, authentication, provider connections or real usage sources.
