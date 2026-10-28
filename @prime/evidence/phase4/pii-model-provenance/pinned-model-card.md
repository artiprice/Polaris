---
library_name: gliner2
license: apache-2.0
base_model: fastino/gliner2-privacy-filter-PII-multi
pipeline_tag: token-classification
tags:
  - token-classification
  - gliner2
  - gliner
  - onnx
  - rust
  - pii
  - ner
  - privacy
  - redaction
  - information-extraction
  - span-extraction
  - iobinding
language:
  - en
  - fr
  - es
  - de
  - it
  - pt
  - nl
---

# GLiNER2 Privacy-Filter PII Multi (ONNX Fragmented & IOBinding)

This repository contains the **ONNX-exported weights** of [`fastino/gliner2-privacy-filter-PII-multi`](https://huggingface.co/fastino/gliner2-privacy-filter-PII-multi),
the multilingual **PII detection model** built on GLiNER2 by Fastino AI.

The model is exported in a **fragmented format** (encoder, token_gather, span_rep, schema_gather, count_pred_argmax, count_lstm_fixed, scorer, classifier) for direct compatibility with [gliner2-rs](https://github.com/SemplificaAI/gliner2-rs), the official **Zero-Python Native Rust inference engine** for GLiNER2.

It supports detection of **42 PII entity types** across **7 languages** (EN, FR, ES, DE, IT, PT, NL).

---

## 🆕 V2 Zero-Copy IOBinding Models

Like the [`gliner2-multi-v1-onnx`](https://huggingface.co/SemplificaAI/gliner2-multi-v1-onnx) base release, this repo ships the **V2 fused IOBinding** variant. `Gather`, `ArgMax`, `MatMul` operations are fused directly into the ONNX graphs so that tensors **never leave the GPU/NPU VRAM**, bypassing the PCIe bus and cutting inference latency by ~30 % on discrete GPUs.

## 📂 Available Variants

| Variant | Use case | Notes |
|---|---|---|
| **`fp16_v2`** *(recommended)* | NVIDIA CUDA · AMD ROCm · Apple CoreML · Qualcomm QNN | Zero-Copy VRAM (IOBinding), full FP16 IO, fused ops |
| **`fp32_v2`** | CPU (AVX2 / XNNPACK / ARM NEON) | High precision V2 fusions for CPU |
| **`fp16`** *(standard)* | Legacy compatible, all EPs | FP32 IO (CoreML-compatible), slower on CUDA due to PCIe round-trips |
| **`fp32`** *(standard)* | Universal fallback | Legacy Float32 |

Each variant ships 8 fragments:

```
encoder_{precision}.onnx          ~530–1060 MB
token_gather_{precision}.onnx     ~ <1 MB
span_rep_{precision}.onnx         ~32–63 MB
schema_gather_{precision}.onnx    ~ <1 MB
count_pred_argmax_{precision}.onnx ~2–5 MB
count_lstm_fixed_{precision}.onnx ~20–41 MB
scorer_{precision}.onnx           ~ <1 MB
classifier_{precision}.onnx       ~2–5 MB
```

Total: **~590 MB (FP16)** or **~1.17 GB (FP32)** per variant.

---

## 🎯 Supported PII Labels (42 types)

### Person / Names (6 labels)
`person`, `full_name`, `first_name`, `middle_name`, `last_name`, `date_of_birth`

### Contact / Address (8 labels)
`email`, `phone_number`, `address`, `street_address`, `city`, `state_or_region`, `postal_code`, `country`

### Government / Tax IDs (7 labels)
`government_id`, `national_id_number`, `passport_number`, `drivers_license_number`, `license_number`, `tax_id`, `tax_number`

### Banking / Payment (8 labels)
`bank_account`, `account_number`, `routing_number`, `iban`, `payment_card`, `card_number`, `card_expiry`, `card_cvv`

### Digital Identity (4 labels)
`username`, `ip_address`, `account_id`, `sensitive_account_id`

### Secrets / Credentials (5 labels)
`password`, `secret`, `api_key`, `access_token`, `recovery_code`

### Sensitive Dates (4 labels)
`sensitive_date`, `document_date`, `expiration_date`, `transaction_date`

---

## 🚀 Usage in Rust (`gliner2-rs`)

```rust
use gliner2_inference::{Gliner2Engine, ModelType, SchemaTask};

// Auto-downloads the V2 FP16 fragments from this HuggingFace repo
// and switches to the high-performance IOBinding engine.
let engine = Gliner2Engine::from_pretrained(
    "SemplificaAI/gliner2-privacy-filter-PII-multi",
    Some("fp16_v2"),
    ModelType::HuggingFace,
)?;

let text = "Please contact Maria Jensen at maria.jensen@example.dk or +45 20 12 34 56.";
let tasks = vec![
    SchemaTask::Entities(vec![
        "person".into(), "email".into(), "phone_number".into(),
    ])
];

let (entities, _, _) = engine.extract(text, &tasks)?;
```

Requires **`gliner2-rs >= 0.4.1`** for automatic V2 detection / IOBinding routing.

## 🐍 Usage in Python (`onnxruntime`)

Run the 8-fragment pipeline manually (no Python `gliner2` dependency needed):

```python
import onnxruntime as ort

# Per fragment (example for the encoder, CUDA backend)
encoder = ort.InferenceSession(
    "encoder_fp16_iobinding.onnx",
    providers=["CUDAExecutionProvider"],
)
# ...load the other 7 fragments analogously...

# Chain them via IOBinding (see validate_onnx_v2.py for a full reference impl)
```

For a simpler entry point you can keep using the original PyTorch model via the `gliner2` Python package on `fastino/gliner2-privacy-filter-PII-multi`; this ONNX repo is optimised for **production deployment without Python**.

---

## 🛠 Pipeline Wiring (IOBinding chain)

```
encoder_fp16_iobinding.onnx
    │
    ├─ token_gather_fp16_iobinding.onnx
    │       └─ span_rep_fp16_iobinding.onnx
    │
    └─ schema_gather_fp16_iobinding.onnx
            ├─ count_pred_argmax_fp16_iobinding.onnx  →  pred_count (int64)
            └─ count_lstm_fixed_fp16_iobinding.onnx
                    └─ scorer_fp16_iobinding.onnx     →  entity_scores

classifier_fp16_iobinding.onnx (only for classification tasks)
```

---

## ⚙️ Technical Notes

- **opset 17** (ONNX 1.14+) for maximum execution-provider compatibility.
- `count_lstm_fixed` exports the GRU **unrolled to 20 fixed steps** at tracing time → compatible with execution providers that don't support dynamic loops (Apple CoreML, Qualcomm QNN).
- `scorer` uses **fused Reshape + MatMul + Transpose** instead of `Einsum` for compatibility with QNN/CoreML FP16.
- **INT8 not supported**: the DeBERTa-v3 disentangled-attention activations contain extreme outliers that saturate 8-bit ranges (the same limitation called out by the GLiNER2 maintainers). FP16 remains the optimal compression target.
- **Encoder size**: ~1.06 GB FP32 → ~530 MB FP16. Larger than the multi-v1 base because of the wider classification head (42 PII labels) and per-language fine-tuning.

## 🪪 License

Apache 2.0 — same as the upstream model.

## 🙏 Acknowledgements

- Upstream model: [`fastino/gliner2-privacy-filter-PII-multi`](https://huggingface.co/fastino/gliner2-privacy-filter-PII-multi) by Fastino AI.
- GLiNER2 paper: Zaratiana et al., *GLiNER2: Schema-Driven Multi-Task Learning for Structured Information Extraction*, EMNLP 2025.
- ONNX fragmentation + IOBinding strategy: Semplifica s.r.l., as used in [`gliner2-multi-v1-onnx`](https://huggingface.co/SemplificaAI/gliner2-multi-v1-onnx).

## 📚 Citation

```bibtex
@misc{fastino2026gliner2pii,
  title   = {GLiNER2-PII: Multilingual PII Extraction via Synthetic Fine-Tuning},
  author  = {{Fastino AI Team}},
  year    = {2026},
  url     = {https://huggingface.co/fastino/gliner2-privacy-filter-PII-multi}
}

@inproceedings{zaratiana-etal-2025-gliner2,
  title     = {GLiNER2: Schema-Driven Multi-Task Learning for Structured Information Extraction},
  author    = {Zaratiana, Urchade and Pasternak, Gil and Boyd, Oliver and Hurn-Maloney, George and Lewis, Ash},
  booktitle = {Proceedings of EMNLP 2025: System Demonstrations},
  year      = {2025}
}
```
