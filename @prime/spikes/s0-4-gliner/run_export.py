"""Run gliner2-rs export_span_v3.py unchanged, with a peak-RSS sampler. Usage: run_export.py <script> <model> <out_dir>"""
import importlib.util, sys, threading, time
from pathlib import Path
import psutil
script, model, out = sys.argv[1:4]
peak = [0]
def sample():
    p = psutil.Process()
    while True:
        peak[0] = max(peak[0], p.memory_info().rss)
        time.sleep(0.5)
threading.Thread(target=sample, daemon=True).start()
spec = importlib.util.spec_from_file_location("exp", script)
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
_orig = m._convert_fp16
def _safe(fp32_path, keep_io_types, out_path=None):
    try:
        return _orig(fp32_path, keep_io_types, out_path)
    except MemoryError:
        print(f"SKIPPED fp16 conversion (MemoryError): {fp32_path} keep_io={keep_io_types}")
m._convert_fp16 = _safe
try:
    m.export_span(model_path=model, out_dir=Path(out), max_count_override=None)
finally:
    print(f"PEAK_RSS_GB={peak[0]/2**30:.2f} avail_GB={psutil.virtual_memory().available/2**30:.2f}")
