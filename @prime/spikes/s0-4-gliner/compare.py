"""Compare Python reference vs Rust outputs: top-1 agreement, max |prob diff|, per-schema breakdown."""
import json, sys, collections
rust_path = sys.argv[1]
load = lambda p: {r["id"]: r for r in map(json.loads, open(p, encoding="utf-8"))}
py, rs = load("golden_python.jsonl"), load(rust_path)
ins = {r["id"]: r for r in map(json.loads, open("golden_inputs.jsonl", encoding="utf-8"))}
tot = collections.Counter(); agree = collections.Counter(); maxd = collections.defaultdict(float); mism = []; margins = []
item_ok = 0
for i, p in py.items():
    ok = True
    for task, po in p["outputs"].items():
        ro = rs[i]["outputs"][task]
        ptop = max(po["probs"], key=po["probs"].get); rtop = max(ro["probs"], key=ro["probs"].get)
        key = ins[i]["kind"] + ("/" + task if ins[i]["kind"] == "multi3" else "")
        d = max(abs(po["probs"][l] - ro["probs"][l]) for l in po["probs"])
        maxd[key] = max(maxd[key], d); maxd["ALL"] = max(maxd["ALL"], d)
        tot[key] += 1; tot["ALL"] += 1
        s = sorted(po["probs"].values(), reverse=True); margins.append((s[0] - s[1], i, task))
        if ptop == rtop and po["label"] == ro["label"]:
            agree[key] += 1; agree["ALL"] += 1
        else:
            ok = False; mism.append((i, task, ptop, rtop, round(s[0] - s[1], 4), round(d, 4)))
    item_ok += ok
for k in sorted(tot):
    print(f"{k:55s} top1 {agree[k]}/{tot[k]} = {100*agree[k]/tot[k]:.2f}%  max|dp|={maxd[k]:.6f}")
print(f"item-level (all tasks of the item agree): {item_ok}/{len(py)}")
print("mismatches (id, task, py_top, rust_top, py_margin, max|dp|):", mism)
margins.sort(); print("smallest python top1-top2 margins:", [(round(m, 5), i) for m, i, t in margins[:5]])
