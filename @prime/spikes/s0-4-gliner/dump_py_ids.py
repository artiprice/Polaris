"""Dump the Python processor's input_ids (and decoded tokens) for chosen golden ids, to diff against Rust."""
import json, sys
import torch
from gliner2 import AutoExtractor
ids = sys.argv[1:]
model = AutoExtractor.from_pretrained("fastino/GLiNER2.5-Decide"); model.eval()
items = {r["id"]: r for r in map(json.loads, open("golden_inputs.jsonl", encoding="utf-8"))}
proc = model.processor
out = {}
for i in ids:
    it = items[i]
    schema = model._classification_schema(it["schema"])
    b = proc.collate_fn_inference([(it["text"], schema)])
    toks = b.input_ids[0].tolist()
    out[i] = {"input_ids": toks, "pieces": proc.tokenizer.convert_ids_to_tokens(toks)}
json.dump(out, open("artifacts/py_ids.json", "w"))
for i in ids:
    print(i, len(out[i]["input_ids"]), " ".join(out[i]["pieces"]))
