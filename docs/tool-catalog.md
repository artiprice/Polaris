# Registered tools

Generated from P1 and P2 builtin schemas plus typed tool specs; checked by the facade's catalog test.

| Name | Effects | Tier | Phase | Description |
|---|---|---|---|---|
| `apply_patch` | workspace proposal | E0 | P2 | Validate and propose a single-file unified patch against the current workspace file; include its full base CID. No host file is written. |
| `classify_command` | pure / persisted context read | E0 | P2 | Classify an explicit argv command as heavy, destructive, or unknown without executing it. |
| `diff_apply` | pure / persisted context read | E0 | P1 | Apply a unified patch to caller-provided text as a pure operation; no files are changed. |
| `edit_file` | workspace proposal | E0 | P2 | Propose a text replacement in a workspace file; include the current full base CID. One match is required unless replace_all is true. Exact text is preferred; whole-line whitespace and typographic punctuation differences are tolerated, and a miss reports the closest lines. |
| `glob` | workspace read | E0 | P1 | Find workspace paths using a glob and repository ignore rules. |
| `grep` | workspace read | E0 | P1 | Search workspace text using a size-guarded regular expression. |
| `json_query` | pure / persisted context read | E0 | P1 | Select a value from caller-provided JSON using JSON Pointer. |
| `list_dir` | workspace read | E0 | P1 | List entries under the workspace while excluding symlinks and secret paths. |
| `polaris_expand` | pure / persisted context read | E0 | P1 | Expand a full or collision-checked short content reference. |
| `polaris_recall` | pure / persisted context read | E0 | P1 | Retrieve persisted relevant context through the configured recall backend. |
| `read_file` | workspace read | E0 | P1 | Read a bounded workspace file; secret-bearing paths are denied. |
| `regex_query` | pure / persisted context read | E0 | P1 | Find bounded regular-expression matches in caller-provided text. |
| `repo_map` | workspace read | E0 | P1 | Build a bounded symbol map from supported source files. |
| `shell` | sandbox execution | E0 | P2 | Run explicit argv in the runtime-owned sandbox only. Output is returned in full within the configured capture bound; it is not run on the host. |
| `skeleton` | workspace read | E0 | P1 | Extract signatures from one supported source file. |
| `write_file` | workspace proposal | E0 | P2 | Propose a complete replacement for an existing UTF-8 workspace file; include its current full base CID. This submits for verification and never writes the file directly. |
| `math` | pure | E0 | P3 | Deterministic math: fixed-scale decimal arithmetic (28–29 significant digits, not arbitrary precision; rational fractions such as 1/3 are rounded to this scale), finite interpolation/statistics, and first-order GPU occupancy from a bundled curated NVIDIA table with per-row citations. Operations have bounded inputs and return typed validation errors; occupancy excludes compiler allocation granularity and spills. |

Network tools and remote MCP integration are future-phase work. The math dispatcher is a pure P3 tool.
