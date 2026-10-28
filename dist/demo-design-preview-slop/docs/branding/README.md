# Polaris branding and migration

The product, Cargo package/library and executable are now Polaris / polaris. Version remains 0.1.0-alpha (displayed v0.1.0alpha). The entry point is dist/polaris.exe; no nvvi compatibility executable is shipped in active package locations. cargo run -- --preview --interactive remains valid.

The real workspace directory remains C:\mDev\nvvi-code. Paths must reflect the actual directory; vendor/model names such as NVIDIA NIM are unchanged. Historical verification records and legacy backups retain their original Nvvi names and executable paths.

## Logo source

The user-provided a-green-star-in-low-resolution-pixel-art--built-fr (1).png is the primary shape reference. The other supplied image is a secondary highlight reference. The root upload files are not required at runtime. Versioned reference copies are [star-primary.png](../../assets/branding/references/star-primary.png) and [star-secondary.png](../../assets/branding/references/star-secondary.png); their SHA256 values are in [SHA256SUMS.txt](../../assets/branding/references/SHA256SUMS.txt).

Those references informed the first Polaris pixel-star design, preserved in [pixel-star](pixel-star/README.md). The active CLI mark is a minimal North Star: an unfilled nine-by-five cell welcome mark and a single-cell app-bar emblem, with a compact five-by-three fallback. It is embedded in src/logo.rs without runtime image dependencies. See the [active palette](north-star-palette.json) and [header specification](../header-logo.md). The web welcome design and its chip-derived icon remain unchanged.

The preceding microcircuit hybrid, its specification and artwork tests are archived in [microcircuit-north-star](microcircuit-north-star/). Its [hybrid palette](hybrid-palette.json) remains a historical reference rather than the active CLI palette.

## Rollback

Before the rename, tracked source was clean at commit d0a419195b05d541798b05922ce9b847bdd0b71c. No Git history was rewritten. [legacy-nvvi](legacy-nvvi/README.md) contains the verbatim theme/logo sources, original header specification and palette roles.

The previously distributed Nvvi executable, SHA256SUMS.txt and build-info.json are preserved in dist/legacy-nvvi/4bceec800502/primary. Its SHA256 is 4bceec800502cb8a34dce6e6a4cd0b3cd6d12d9687222b80ccb1bb6ceac40eb9. Previous alternate packages are archived under their own SHA256 prefix and location label before replacement; the archive inventory is dist/legacy-nvvi/archive.json. Those are backups, not active compatibility aliases.

Both earlier renderers and their specifications are preserved independently of the Polaris name. The original Nvvi chip is in legacy-nvvi; the first Polaris pixel star and its header tests are in pixel-star. Its previous executable and matching manifests are in dist/legacy-polaris-pixel/68bf2a4ca77c, with SHA256 68bf2a4ca77cc8ce4a6a15c1df39249f0cead1c2fd21e094f68f30108a2e4b7d. Restore a renderer together with its artwork-specific tests; retain current layout and behavior regressions.

The full rebrand is verified by target/polaris-verification.json, with executable checksums/imports in dist/build-info.json and source/reference comparisons in target/polaris-package.json.

The thin microcircuit hybrid is verified separately in target/polaris-hybrid-verification.json, with release output in target/polaris-hybrid-release.json and exact package/startup evidence in target/polaris-hybrid-package.json. The earlier rebrand record above remains historical.

Default Windows color selection now uses VT output capability rather than requiring terminal environment hints. That historical color-policy revision left its then-current palette and artwork unchanged. See [color policy](../color-policy.md) and target/color-consistency-verification.json for that revision's package checks. The prior hybrid executable is preserved under dist/legacy-color-policy/e48c4cb8c351.

The current CLI redesign evidence is in target/cli-redesign-review/verification.json. All active source/header/control documentation uses the minimal North Star; archived revisions retain their original artwork.
