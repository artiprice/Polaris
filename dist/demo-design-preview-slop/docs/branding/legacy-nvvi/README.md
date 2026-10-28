# Pre-Polaris branding backup

Baseline commit: d0a419195b05d541798b05922ce9b847bdd0b71c. The tracked source was clean before the rename.

The verbatim theme and logo sources and palette.json preserve the old appearance. Uploaded originals were left intact; versioned copies are in assets/branding/references. The previous executable and matching manifests are preserved in dist/legacy-nvvi/4bceec800502/primary.

To restore only the original color family while retaining Polaris and the star, set the star body to #76B900 and highlight to #B6E35B. The application palette remains unchanged by the rebrand. To recover the exact old chip, consult logo.rs.txt and header-logo.md; its geometry is a separate design from the star. Do not overwrite the new renderer with the legacy source without also restoring its geometry tests.

Git can inspect the original files with git show d0a419195b05d541798b05922ce9b847bdd0b71c:src/theme.rs and git show d0a419195b05d541798b05922ce9b847bdd0b71c:src/logo.rs. No history has been rewritten.
