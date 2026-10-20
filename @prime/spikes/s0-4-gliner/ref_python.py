"""Python gliner2 reference: golden outputs (labels + full prob distributions) + CPU latency/RSS."""
import json, statistics, sys, time
import psutil, torch
torch.set_num_threads(4)
from gliner2 import AutoExtractor
import gliner2, transformers

proc = psutil.Process()
t0 = time.perf_counter()
model = AutoExtractor.from_pretrained("fastino/GLiNER2.5-Decide")
load_s = time.perf_counter() - t0
rss_after_load = proc.memory_info().rss
model.eval()
print("device:", getattr(model, "device", "?"), "load_s=%.2f" % load_s, "rss_load_GB=%.2f" % (rss_after_load / 2**30), flush=True)

# capture the probability vector that classify_text decodes from (public output is still used for top-1)
captured = []
cls = next(c for c in type(model).__mro__ if "_decode_classification_probabilities" in c.__dict__)
orig = cls.__dict__["_decode_classification_probabilities"].__func__
def spy(results, schema_name, cls_config, probs):
    captured.append((schema_name, list(cls_config["labels"]), [float(x) for x in probs.detach().cpu().tolist()]))
    return orig(results, schema_name, cls_config, probs)
cls._decode_classification_probabilities = staticmethod(spy)

items = [json.loads(l) for l in open("golden_inputs.jsonl", encoding="utf-8")]
def call(text, schema):
    captured.clear()
    with torch.inference_mode():
        out = model.classify_text(text, schema)
    return out

with open("golden_python.jsonl", "w", encoding="utf-8", newline="\n") as fh:
    for it in items:
        out = call(it["text"], it["schema"])
        res = {}
        for name, labels, probs in captured:
            res[name] = {"label": out[name], "probs": dict(zip(labels, probs))}
        assert set(res) == set(it["schema"]), (it["id"], res.keys(), out)
        fh.write(json.dumps({"id": it["id"], "kind": it["kind"], "outputs": res}, ensure_ascii=False) + "\n")
print("golden written", flush=True)

def bench(sel, n):
    for it in sel[:3]: call(it["text"], it["schema"])          # warmup
    ts = []
    for i in range(n):
        it = sel[i % len(sel)]; t = time.perf_counter(); call(it["text"], it["schema"]); ts.append((time.perf_counter() - t) * 1000)
    ts.sort()
    return {"n": n, "p50_ms": statistics.median(ts), "p95_ms": ts[int(0.95 * (len(ts) - 1) + 0.5)], "min_ms": ts[0], "max_ms": ts[-1]}
b1 = bench([i for i in items if i["kind"] == "choice"], 40)
b3 = bench([i for i in items if i["kind"] == "multi3"], 30)
peak = proc.memory_info().peak_wset
res = {"gliner2": gliner2.__version__ if hasattr(gliner2, "__version__") else "2.1.0", "torch": torch.__version__, "transformers": transformers.__version__,
       "threads": torch.get_num_threads(), "load_s": load_s, "rss_after_load_GB": rss_after_load / 2**30, "peak_wset_GB": peak / 2**30,
       "latency_1task": b1, "latency_3task": b3}
json.dump(res, open("py_bench.json", "w"), indent=1)
print(json.dumps(res, indent=1))
