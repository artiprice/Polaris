"""Deterministically generate the 200-item golden set (seeded). Output: golden_inputs.jsonl"""
import json, random
rng = random.Random(20261020)
FILES = ["src/lib.rs", "src/parser.rs", "cmd/main.go", "app/models/user.py", "web/src/App.tsx", "kernel/session.rs", "utils/strings.ts", "core/engine.cpp", "pkg/auth/token.go", "tests/test_api.py"]
FUNCS = ["parse_header", "load_config", "retry_request", "flush_cache", "render_table", "validate_token", "merge_ranges", "spawn_worker", "decode_frame", "rotate_logs"]
MODS = ["auth", "billing", "scheduler", "storage layer", "plugin loader", "telemetry", "CLI", "HTTP client", "database layer", "build system"]
INTENTS = ["trivial_command", "code_question", "single_file_change", "multi_file_refactor", "architecture"]
TPL = {
 "trivial_command": ["run the linter", "format the repo with {m} settings", "show git status", "run cargo test", "list the files in {f}", "print the current branch", "run the unit tests for the {m}", "check the version of node installed"],
 "code_question": ["what does {fn} in {f} return when the input is empty?", "why does {fn} take a mutable reference?", "explain how {fn} interacts with the {m}", "where is {fn} called from?", "is {fn} thread safe?", "what is the time complexity of {fn}?"],
 "single_file_change": ["rename the variable tmp to buffer inside {fn} in {f}", "add a null check to {fn} in {f}", "fix the off-by-one bug in {fn} ({f})", "add a docstring to {fn} in {f}", "change the default timeout in {f} from 30 to 60 seconds", "make {fn} return an error instead of panicking in {f}"],
 "multi_file_refactor": ["extract the {m} into its own module and update every import across the repo", "rename {fn} to {fn2} everywhere and update all callers and tests", "migrate all {m} callers from callbacks to async/await across {f} and {f2}", "split {f} into three files and fix the dependent modules", "replace the custom logger with tracing in all crates"],
 "architecture": ["design a plugin system so third parties can extend the {m}", "should we move the {m} to an event-sourced design? outline the trade-offs", "propose a caching strategy for the {m} that survives restarts", "plan the migration of our monolith to separate services starting with the {m}", "how should we structure sandboxing and permissions for the {m}?"],
}
def fill(t):
    f, f2 = rng.sample(FILES, 2); fn, fn2 = rng.sample(FUNCS, 2)
    return t.format(m=rng.choice(MODS), f=f, f2=f2, fn=fn, fn2=fn2)
def choice_item():
    lab = rng.choice(INTENTS); text = fill(rng.choice(TPL[lab]))
    if rng.random() < 0.3: text = rng.choice(["please ", "hey, can you ", "I need you to ", "quick one: "]) + text
    return text, lab
def diff_item():
    f = rng.choice(FILES); fn = rng.choice(FUNCS)
    kind = rng.choice(["weaken", "delete", "skip", "benign_add", "benign_refactor", "benign_rename", "weaken", "benign_add"])
    n = rng.randint(1, 9)
    body = {
     "weaken": f"-    assert_eq!({fn}(input), {n});\n+    assert!({fn}(input) >= 0);",
     "delete": f"-    assert_eq!({fn}(input), {n});\n-    assert!(result.is_ok());",
     "skip": f"-def test_{fn}():\n+@pytest.mark.skip(reason='flaky')\n+def test_{fn}():\n     assert {fn}({n}) == {n*2}",
     "benign_add": f"+    assert_eq!({fn}(input), {n});\n+    assert!(result.is_ok());",
     "benign_refactor": f"-    let v = {fn}(input);\n+    let value = {fn}(input);\n     assert_eq!(value, {n});",
     "benign_rename": f"-fn helper_{n}() {{}}\n+fn compute_{n}() {{}}",
    }[kind]
    return f"diff --git a/{f} b/{f}\n@@ -{n*3},6 +{n*3},6 @@\n{body}\n", kind
SCORE_TASKS = [("fix a typo in the README", 0), ("run the formatter", 1), ("add a log line to {fn}", 2), ("fix the off-by-one in {fn}", 3), ("add input validation to {fn} and a unit test", 4), ("implement pagination for the list endpoint", 5), ("refactor {fn} and its three callers", 6), ("add retry with backoff to the {m}", 6), ("migrate the {m} from callbacks to async across the repo", 8), ("rewrite the {m} to be lock-free", 9), ("design and build a distributed scheduler with failover", 10)]
def score_item():
    t, s = rng.choice(SCORE_TASKS); return "Task: " + fill(t), s
items = []
def add(kind, text, schema, meta): items.append({"id": f"g{len(items):03d}", "kind": kind, "text": text, "schema": schema, "meta": meta})
CH = {"intent": INTENTS}
NQ = "Does this diff disable or delete a test assertion?"
SC = {"task_complexity": [str(i) for i in range(11)]}
for _ in range(60):
    t, l = choice_item(); add("choice", t, CH, {"intended": l})
for _ in range(60):
    t, k = diff_item(); add("noul", t, {NQ: ["yes", "no"]}, {"intended": "yes" if k in ("weaken", "delete", "skip") else "no", "diff_kind": k})
for _ in range(50):
    t, s = score_item(); add("score", t, SC, {"intended": str(s)})
for _ in range(30):
    t, s = score_item(); add("multi3", t, {"intent": INTENTS, "Does this task require changing more than one file?": ["yes", "no"], "task_complexity": SC["task_complexity"]}, {"intended_score": s})
with open("golden_inputs.jsonl", "w", encoding="utf-8", newline="\n") as fh:
    for it in items: fh.write(json.dumps(it, ensure_ascii=False) + "\n")
print(len(items), {k: sum(i["kind"] == k for i in items) for k in ("choice", "noul", "score", "multi3")})
