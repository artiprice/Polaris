# Spike S0-3: NemoClaw custom agent / blueprint

RESULT: FAIL -> native blueprint applier

Short answer: NemoClaw has no supported out-of-tree custom-agent path. Agents are discovered only from in-tree directories `agents/<name>/manifest.yaml` of the NVIDIA/NemoClaw repo, and the sandbox images for those agents are built and published by NVIDIA CI. The one documented custom path, `nemoclaw onboard --from <Dockerfile>`, is a custom *image* path, not agent registration, and is not qualified on native Podman. Polaris should implement its own blueprint applier over OpenShell and may reuse NemoClaw's formats and images.

Labels: VERIFIED = read in the repo or docs during this spike. LIKELY = inferred from code or docs but not run. UNVERIFIED = not checked.

## Evidence

### 1. Agent discovery is in-tree only (VERIFIED)
- `src/lib/agent/defs.ts`: `export const AGENTS_DIR = path.join(ROOT, "agents");` and `listAgents()` scans `agents/` for directories containing `manifest.yaml`. `ROOT = REPOSITORY_ROOT` (`src/lib/runner.ts`). No env override for the agents directory was found.
- Repo `agents/` contains: `hermes`, `langchain-deepagents-code`, `nemocua`, `openclaw`, `pi`.
- `src/lib/onboard/managed-image/contract.ts`:
  - `SHIPPED_MANAGED_IMAGE_AGENTS = ["openclaw", "hermes", "langchain-deepagents-code"]`
  - `CANDIDATE_MANAGED_IMAGE_AGENTS = ["pi"]`
- `src/lib/agent/candidate.ts`: candidate agents are gated by `NEMOCLAW_CANDIDATE_AGENTS=1` plus a qualification receipt. `src/lib/agent/candidate-authority.ts` holds a hardcoded list of accepted receipt SHA-256 digests for `pi`, and says "a caller can neither mint an accepted digest nor replace one through environment configuration."
- Conclusion: a Polaris agent would have to be a PR to NVIDIA/NemoClaw adding `agents/polaris/manifest.yaml`, plus image publication through CI (`.github/workflows/base-image.yaml`). That is not a supported external extension point.

### 2. Agent manifest schema (VERIFIED, `agents/openclaw/manifest.yaml`)
Fields include `name`, `display_name`, `description`, `version_constraint`, `language`, `install_method`, `binary_path`, `version_command`, `expected_version`, `gateway_command`, `runtime.interactive_command`, `health_probe`, `forward_ports`, `config` (`dir`, `config_file`, `format`), `skills`, `user_managed_files`, `device_pairing`, `web_auth_method`, `inference`, `mcp`, `phone_home_hosts`, `package_registry`. The file is annotated: "This documents the current (default) agent — artifacts still live at root level for backward compatibility. Phase 3 of the multi-agent plan will move them here."

### 3. Blueprint YAML schema (VERIFIED, `nemoclaw-blueprint/blueprint.yaml` at HEAD)
Top-level keys: `version`, `min_openshell_version`, `max_openshell_version`, `min_openclaw_version`, `digest`, `profiles` (list), `description`, `components`.
- `components.sandbox`: `image` (`ghcr.io/nvidia/openshell-community/sandboxes/openclaw@sha256:...`), `name`, `forward_ports`, `resource_profiles`.
- `components.inference.profiles.<name>`: `provider_type`, `provider_name`, `endpoint`, `model`, `credential_env`, `credential_default`, `timeout_secs`, `dynamic_endpoint`.
- `components.router`: `enabled`, `port`, `pool_config_path`.
- `components.policy.base`: policy path.
- Version pin mismatch: `min_openshell_version: "0.0.116"` and `max_openshell_version: "0.0.116"`. Polaris targets OpenShell v0.1.4, so this blueprint is not compatible with the Polaris target as written.

### 4. Blueprint verification (mixed)
- The `digest` field is documented as a mirror of the `components.sandbox.image` digest, so that release tooling can check both fields agree (VERIFIED from the YAML comment, "Lets a downstream consumer ... verify the blueprint declares a specific sandbox image"). It is not a signature.
- `nemoclaw/src/blueprint/runner.ts`: `loadBlueprint()` reads `blueprint.yaml` and `isBlueprint()` checks an allowlist of keys and nested shapes (VERIFIED, read in code).
- Docs claim: "The integration layer checks the artifact digest against the expected value" (`https://docs.nvidia.com/nemoclaw/latest/user-guide/openclaw/reference/architecture.md`, Blueprint Lifecycle step 2). I did not confirm this check in code (UNVERIFIED).
- No signature verification was found in the runner or the base-image code (LIKELY absent, not exhaustively searched).

### 5. Base image (VERIFIED from CI and code)
- `.github/workflows/base-image.yaml`: `REGISTRY: ghcr.io`; the OpenClaw base is built as image `nvidia/nemoclaw/sandbox-base`. Other bases: `nvidia/nemoclaw/hermes-sandbox-base`, `nvidia/nemoclaw/langchain-deepagents-code-sandbox-base`, `nvidia/nemoclaw/pi-sandbox-base`.
- `src/lib/agent/base-image.ts`: `HERMES_OFFICIAL_BASE_DIGEST_REF = /^ghcr\.io\/nvidia\/nemoclaw\/hermes-sandbox-base@sha256:[0-9a-f]{64}$/`, so full refs have the form `ghcr.io/nvidia/nemoclaw/<agent>-sandbox-base@sha256:<digest>`.
- The docs do not name the base image; the architecture page says only "selects an immutable managed image for the installed release and host architecture."

### 6. Onboard flags (VERIFIED, `docs/reference/commands.mdx`, line ~423)
`nemoclaw onboard [--profile <name>] [--non-interactive] [--resume | --fresh] [--recreate-sandbox] [--from <Dockerfile> | --from-image <repository>@sha256:<digest>] [--name <sandbox>] ... [--agent <name>] [--agents <agents.yaml>] [--tool-disclosure <progressive|direct>] ... [--yes-i-accept-third-party-software]`

- `--from <Dockerfile>`: "build the sandbox from a user-supplied Dockerfile instead of the default NemoClaw image" (VERIFIED). Docs say it "remains a separate complete custom-image path" (`docs/reference/architecture.mdx` line 335).
- `docs/reference/platform-support.mdx`, native rootless Podman row: "Custom `--from` Dockerfiles and read-only host mounts are not qualified." Docker-backed custom images are the only supported `--from` route.
- `--agents <agents.yaml>` (declarative secondary agents for the OpenClaw sandbox, see `https://docs.nvidia.com/nemoclaw/latest/user-guide/openclaw/configure-agents/declarative-agents-manifest.md`) adds secondary agents to the OpenClaw roster. It does not add new agent types, and the docs say only the canonical `main` agent is always baked in.

### 7. Documented support (VERIFIED)
`docs/reference/platform-support.mdx` lists Hermes as "First-class agent" and Pi as a candidate. The agent set matches the code: OpenClaw, Hermes, Deep Agents Code (shipped), plus Pi (candidate).

## What Polaris can reuse
- Blueprint format: the `blueprint.yaml` shape (`profiles`, `components.sandbox/inference/router/policy`) is a reasonable template for Polaris's own blueprint. Parts to copy: the digest-mirror pattern, `min_/max_openshell_version`, and per-profile inference blocks.
- Agent manifest format: `agents/*/manifest.yaml` is a clean, readable integration contract (binary path, version command, health probe, config dir, MCP support, phone-home hosts). Polaris can define an equivalent `polaris` manifest.
- Base images: `ghcr.io/nvidia/nemoclaw/*-sandbox-base` images are public references. Whether Polaris may reuse them (license, pinning, and whether they include Polaris's runtime) is UNVERIFIED and needs a decision.
- Policy presets: `nemoclaw-blueprint/policies/presets/*.yaml` (for example `weather.yaml`) and the `apply-policy-presets` docs. Reusable as policy examples over OpenShell (UNVERIFIED for compatibility with OpenShell v0.1.4 policy schema).
- Verification ideas: pin images by `@sha256:` digest and check the digest mirror field, as NemoClaw does.

## What Polaris must implement itself
- Agent registration: Polaris needs its own registry (a manifest directory or config) and its own onboarding command. NemoClaw's registry is hardcoded to the repo's `agents/` tree.
- Blueprint applier: parse the blueprint, check OpenShell version compatibility against v0.1.4, create the sandbox via OpenShell, apply the policy, configure inference routes.
- Image build and digest pinning for the Polaris runtime, on top of OpenShell.
- Any signature or provenance checks. NemoClaw's digest field is not a signature.

## Open questions
1. Does NVIDIA accept third-party agents in-tree (a PR to add `agents/polaris`)? `AGENTS.md#product-scope-gate` (not read in this spike) may decide this. UNVERIFIED.
2. Does the blueprint runner enforce `min/max_openshell_version`, and does it enforce the image digest? Code reading was partial. UNVERIFIED.
3. Licensing and terms for reusing `ghcr.io/nvidia/nemoclaw/*-sandbox-base` images. UNVERIFIED.
4. The `blueprint.yaml` pins OpenShell 0.0.116, while Polaris targets v0.1.4. Whether NemoClaw has a newer blueprint for v0.1.4 is UNVERIFIED (checked HEAD only, not a v0.1.4 tag).

## Sources
- https://docs.nvidia.com/nemoclaw/latest/llms.txt (index only)
- https://docs.nvidia.com/nemoclaw/latest/user-guide/openclaw/reference/architecture.md
- https://docs.nvidia.com/nemoclaw/latest/user-guide/openclaw/configure-agents/declarative-agents-manifest.md
- GitHub NVIDIA/NemoClaw (HEAD, read via `gh api`): `src/lib/agent/defs.ts`, `src/lib/agent/candidate.ts`, `src/lib/agent/candidate-authority.ts`, `src/lib/onboard/managed-image/contract.ts`, `src/lib/agent/base-image.ts`, `nemoclaw-blueprint/blueprint.yaml`, `agents/openclaw/manifest.yaml`, `nemoclaw/src/blueprint/runner.ts`, `.github/workflows/base-image.yaml`, `docs/reference/commands.mdx`, `docs/reference/architecture.mdx`, `docs/reference/platform-support.mdx`
