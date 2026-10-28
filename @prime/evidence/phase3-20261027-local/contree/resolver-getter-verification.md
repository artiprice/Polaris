# Contree resolver getter verification

Scope: public, read-only resolver inputs needed by injected `ImageRanker` implementations. Evidence recorded 2026-10-27.

## Command

```powershell
cargo test -p polaris-nebius-cloud-sandboxes
cargo clippy -p polaris-nebius-cloud-sandboxes --lib --test environment_resolver -- -D warnings
python -m json.tool specs/descriptors/contree_environment_resolver.json > $null
git diff --check -- crates/polaris-nebius-cloud/crates/sandboxes/src/env_resolver.rs crates/polaris-nebius-cloud/crates/sandboxes/tests/environment_resolver.rs specs/descriptors/contree_environment_resolver.json
```

## Raw output

```text
Compiling polaris-nebius-cloud-sandboxes v0.1.0-alpha (C:\mDev\Polaris\crates\polaris-nebius-cloud\crates\sandboxes)
Finished `test` profile [unoptimized] target(s) in 2.43s
Running unittests src\lib.rs (target\debug\deps\polaris_nebius_cloud_sandboxes-0ba22f0a478ac3a0.exe)

running 6 tests
test tests::contree_rejects_non_loopback_sidecar_and_missing_egress_consent ... ok
test tests::contree_sanitizes_secrets_and_bounds_tool_responses ... ok
test tests::contree_cancellation_joins_local_calls_and_cancels_owned_operations ... ok
test tests::contree_bounds_operations_and_preserves_branch_identity ... ok
test tests::contree_checks_inventory_and_verified_tool_shapes ... ok
test tests::contree_sync_projects_only_approved_sync_plan_paths ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

Running tests\environment_resolver.rs (target\debug\deps\environment_resolver-952dfe9e4edd3c8e.exe)

running 17 tests
test annotation_for_unlisted_image_is_rejected ... ok
test duplicate_catalog_or_annotation_ids_are_rejected ... ok
test ambiguous_candidates_use_ranker ... ok
test exact_commit_match_wins ... ok
test exact_commit_incompatible_toolchain_is_not_selected ... ok
test compatible_metadata_selects_unique_candidate ... ok
test manifest_requires_canonical_repo_and_full_git_oid ... ok
test exact_commit_incompatible_runtime_is_not_selected ... ok
test injected_ranker_can_inspect_manifest_metadata_and_actual_image_record ... ok
test ranker_invalid_score_is_rejected ... ok
test ranker_invented_id_is_rejected ... ok
test resolver_is_deterministic_for_permuted_inputs ... ok
test ranker_duplicate_ids_are_rejected ... ok
test no_match_and_unconstrained_fallback_are_rejected ... ok
test tied_highest_scores_remain_ambiguous ... ok
test ranker_omitted_scores_are_rejected ... ok
test oversized_catalog_is_rejected ... ok

test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

Doc-tests polaris_nebius_cloud_sandboxes

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

Finished `dev` profile [unoptimized] target(s) in 0.70s
descriptor_json=valid
git_diff_check=pass
```

Live Contree/SWE acceptance was not run; no live credentials/catalog were available in this task context.
