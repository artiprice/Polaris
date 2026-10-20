---
library_name: transformers
license: apache-2.0
language:
- en
pipeline_tag: text-classification
tags:
- gliner2
- Text classification
- Intent classification
- Sentiment Analysis
- Topic classification
- Named Entity Recognition
- decision-model
base_model: fastino/gliner2-large-v1
---
<div align="center">
  <a href="https://agent.fastino.ai" target="_blank" rel="noopener noreferrer">
    <img src="GLiNER-2.5-Decision-HF-Banner.png" alt="Fastino Agent - Fine-tune GLiNER with a single prompt" width="100%"/>
  </a>
</div>

<div style="display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 16px;">
  <a href="https://agent.fastino.ai" target="_blank" rel="noreferrer" style="text-decoration:none;">
    <img src="https://img.shields.io/badge/Finetune-GLiNER2.5-EA4335" alt="Fine-tune and Deploy GLiNER2.5 with Fastino Agent" style="vertical-align:middle;">
  </a>
  <a href="https://arxiv.org/abs/2507.18546" target="_blank" rel="noreferrer" style="text-decoration:none;">
    <img src="https://img.shields.io/badge/arXiv-2507.18546-b31b1b.svg?logo=arxiv" alt="arXiv Paper" style="vertical-align:middle;">
  </a>
  <a href="https://github.com/fastino-ai/GLiNER2" target="_blank" rel="noreferrer" style="text-decoration:none;">
    <img src="https://img.shields.io/badge/GitHub-GLiNER2-black?logo=github" alt="GitHub" style="vertical-align:middle;">
  </a>
  <a href="https://x.com/fastinoAI" target="_blank" rel="noreferrer" style="text-decoration:none;">
    <img src="https://img.shields.io/twitter/follow/:fastinoAI" alt="Follow @fastinoAI" style="vertical-align:middle;">
  </a>
</div>

# GLiNER2.5-Decide

**The 340M English classification model in the GLiNER2.5 family.** Pass any label set at call time: intent, routing, sentiment, priority, policy, and multi-label tags, in a single forward pass. No prompt template. No generated tokens. Load it with `AutoExtractor` and ship it locally.

A single call can score several heads at once. Single-label tasks return one string. Multi-label tasks return every label above the threshold.

## Benchmark

Exact-match accuracy on [`fastino/fast-decisions`](https://huggingface.co/datasets/fastino/fast-decisions): 17 domains, 300 held-out examples each, with the same text and candidate labels for every model.

| Model | Avg |
|---|---:|
| **GLiNER2.5-Decide (340M)** | **60.2%** |
| [GLiNER2.5-Decide-1B](https://huggingface.co/fastino/GLiNER2.5-Decide-1B) | 59.6% |
| JevK5 | 57.6% |
| [GLiNER2.5-multi-Decide (287M)](https://huggingface.co/fastino/GLiNER2.5-multi-Decide) | 56.7% |
| SemIf (Qwen3.5-4B) | 56.4% |
| GLiFormer large-v1 | 49.0% |
| Laya Router | 46.6% |

The suite is English. Use [`GLiNER2.5-multi-Decide`](https://huggingface.co/fastino/GLiNER2.5-multi-Decide) when the input is multilingual. Domain and label details are on the [dataset card](https://huggingface.co/datasets/fastino/fast-decisions).

This release is not a general-purpose model. It does not reason, explain, or answer open questions. It is a specialist for operational decisions: customer and banking intent, travel and clinic requests, review sentiment, document type, email and ticket routing, human handoff, agent completion, moderation, severity, urgency, and spam. The same call also answers a question about a passage, classifies a book, uses labels that carry a description, and scores an ordinal scale.

Outputs below are potential results for these inputs. They show the shape `classify_text` returns.

## Install

```bash
pip install "gliner2[local]"
```

```python
from gliner2 import AutoExtractor

model = AutoExtractor.from_pretrained("fastino/GLiNER2.5-Decide")
```

## Examples

### Customer support intent

Route a live message before a human ever sees it. Refunds, cancellations, login failures, and shipping delays share one inbox and one label set. Decide picks the action the queue should take, so the right workflow starts on the first turn.

```python
model.classify_text(
    "My subscription renewed on April 15 for ¥5,400 after the service was already down. Can I get that charge refunded?",
    {"intent": [
        "order_status", "refund_request", "cancel_subscription", "update_payment",
        "login_problem", "shipping_delay", "bug_report", "speak_to_human", "other",
    ]},
)
```

Potential output:

```text
{"intent": "refund_request"}
```

### Banking request

A single customer message often mixes a failed payment, a beneficiary change, and a fee question. The label set is the product catalog: transfers, cards, fraud, mortgages. The model maps the utterance onto the operation the core system should open.

```python
model.classify_text(
    "The transfer I sent this morning is still pending, and I think I used the wrong sort code. Can you stop it and add Emily as the beneficiary instead?",
    {"intent": [
        "transfer_pending", "transfer_cancel", "beneficiary_add", "card_lost",
        "balance_inquiry", "fraud_report", "mortgage_application", "fee_explanation",
    ]},
)
```

Potential output:

```text
{"intent": "transfer_cancel"}
```

### Travel request

Booking, changing, cancelling, and seat requests look similar in free text and trigger different inventory calls. Use this when a chat or email has to become a structured booking action without a form.

```python
model.classify_text(
    "I need to move my Friday flight to Paris to Saturday morning, same cabin, and keep the aisle seat if you can.",
    {"request": ["book", "change", "cancel", "status", "seat_change", "refund", "baggage"]},
)
```

Potential output:

```text
{"request": "change"}
```

### Clinic request

Patients describe symptoms and the thing they want in the same sentence: an appointment, a refill, a result, a referral. Front-desk routing needs that distinction before anyone is put on a schedule.

```python
model.classify_text(
    "The rash came back after the antibiotics finished. Can I get a same-week appointment with dermatology, or should I just refill the cream?",
    {"request": [
        "book_appointment", "refill_prescription", "test_results",
        "referral", "billing_question", "cancel_appointment",
    ]},
)
```

Potential output:

```text
{"request": "book_appointment"}
```

### Review sentiment

Star ratings hide mixed reviews. A product can be praised and rejected in one paragraph. A four-way sentiment label is what a dashboard, a reply policy, or a ranking feature actually needs.

```python
model.classify_text(
    "Battery dies before lunch, but the keyboard and the screen are the best I have used on a laptop.",
    {"sentiment": ["positive", "negative", "mixed", "neutral"]},
)
```

Potential output:

```text
{"sentiment": "mixed"}
```

### Product aspects

Sentiment says the review is mixed. Aspects say why: battery, keyboard, screen. Several labels apply at once, so this head is multi-label. That is the input to aspect-level analytics and to a reply that mentions the right part of the product.

```python
model.classify_text(
    "Battery dies before lunch, but the keyboard and the screen are the best I have used on a laptop.",
    {"aspects": {
        "labels": ["battery", "keyboard", "screen", "camera", "price", "support"],
        "multi_label": True,
        "cls_threshold": 0.4,
    }},
)
```

Potential output:

```text
{"aspects": ["battery", "keyboard", "screen"]}
```

### News topic

Wire copy, alerts, and scraped headlines have to land in a section before they are ranked or summarized. The label set is the section list of the product, not a fixed taxonomy baked into the model.

```python
model.classify_text(
    "The central bank held rates and said inflation is still above target, pushing bank stocks lower in afternoon trading.",
    {"topic": ["politics", "business", "sports", "science", "entertainment", "world"]},
)
```

Potential output:

```text
{"topic": "business"}
```

### Document type

Inboxes and shared drives mix invoices, contracts, resumes, and notes. Classifying the document is the gate in front of extraction: an invoice goes to payables, a contract goes to review, a resume goes to screening.

```python
model.classify_text(
    "INVOICE 1842\nBill to: Northstar QA\nAmount due: 2,400 USD\nDue: 30 April 2026\nWire instructions are on page 2.",
    {"document_type": ["invoice", "receipt", "contract", "resume", "support_email", "meeting_notes"]},
)
```

Potential output:

```text
{"document_type": "invoice"}
```

### Email triage

A shared mailbox needs three answers before a message is filed: what the sender wants, how soon it matters, and which team owns it. One call scores all three heads on the same text, so the router does not run the model three times.

```python
model.classify_text(
    "From: compliance@group.example\nSubject: Protocol update — action required today\n\nPlease confirm the new retention rule is applied before Friday's audit.",
    {
        "intent": ["fyi", "request", "approval", "complaint", "newsletter", "security_alert"],
        "urgency": ["low", "normal", "high", "critical"],
        "route": ["support", "billing", "legal", "security", "finance", "archive"],
    },
)
```

Potential output:

```text
{"intent": "request", "urgency": "high", "route": "legal"}
```

### Ticket routing

Employees describe a problem, not a department. Payroll, benefits, IT access, and facilities share the same portal. The queue label is the assignment, so the ticket opens in the right team instead of bouncing through a dispatcher.

```python
model.classify_text(
    "[subject] 401k deduction missing from this paystub\n[body] Last month's contribution posted. This month the line is gone and HR told me to open a ticket.",
    {"queue": [
        "payroll", "benefits", "it_access", "facilities",
        "expense_reimbursement", "manager_approval",
    ]},
)
```

Potential output:

```text
{"queue": "benefits"}
```

### Handoff to a person

Most turns should stay automated. A repeated complaint, an explicit request for a human, or a case the bot cannot close should leave the flow. This is the yes/no gate in front of an agent queue.

```python
model.classify_text(
    "This is the third time I have explained the same missing refund. Stop the bot and get me a person.",
    {"handoff": ["yes", "no"]},
)
```

Potential output:

```text
{"handoff": "yes"}
```

### Did the agent finish?

A trace can look busy and still be incomplete: a draft saved, a button disabled, a required field empty. Supervisors and eval harnesses need a finish decision from the goal plus the last state, not from whether the model stopped talking.

```python
model.classify_text(
    "Goal: email the Q4 summary to every partner.\nLast action: draft saved in the hub.\nSend button is still disabled because two partners have no address.",
    {"finished": ["yes", "no"]},
)
```

Potential output:

```text
{"finished": "no"}
```

### Moderation

User content and model output both need a policy decision before they are shown or acted on. The labels are the policy, from allow through personal data, harassment, scam, and spam, so a filter can block, redact, or escalate.

```python
model.classify_text(
    "Post the customer's home address in the public thread so everyone can see where the package actually went.",
    {"policy": ["allow", "personal_data", "harassment", "scam", "violence", "spam"]},
)
```

Potential output:

```text
{"policy": "personal_data"}
```

### Incident severity

Pages and deploys produce more notes than pages. Severity is what decides whether this wakes someone up. Staging-only tag drift is not a production checkout failure, and the label should say so.

```python
model.classify_text(
    "The deploy left resource tags inconsistent across staging. Production checkout is unaffected. No customer reports yet.",
    {"severity": ["info", "low", "medium", "high", "critical"]},
)
```

Potential output:

```text
{"severity": "low"}
```

### Urgency score

Some queues want a rank, not a bucket. Pass `"0"` through `"5"` as ordinary strings. A payroll cutoff before 5pm and a typo in a wiki page should not receive the same score, and the label is what the SLA system reads.

```python
model.classify_text(
    "Payroll file has to be corrected before the 5pm cutoff or the whole company is paid late.",
    {"urgency": ["0", "1", "2", "3", "4", "5"]},
)
```

Potential output:

```text
{"urgency": "5"}
```

### Spam or not

The first filter on an inbox or a comment stream. Phishing and mailbox-full lures should never reach the intent router. A two-label decision keeps that check cheap enough to run on every message.

```python
model.classify_text(
    "Your mailbox is almost full. Click here in the next hour or we will delete every message.",
    {"label": ["spam", "ham"]},
)
```

Potential output:

```text
{"label": "spam"}
```

### Several decisions at once

Real requests carry more than one fact. A guest can need a room move, a billing release, and a maintenance ticket in the same call. Intent, priority, a human gate, and multi-label topics are scored together, which is how a property system opens the right work orders without a chain of prompts.

```python
model.classify_text(
    "Guest in room 1408 says the AC has been out since yesterday and they want to move tonight or leave. They also asked for the incidentals hold to be released.",
    {
        "intent": ["maintenance", "room_change", "checkout", "billing", "complaint", "amenity_request"],
        "priority": ["low", "normal", "high", "urgent"],
        "needs_human": ["yes", "no"],
        "topics": {
            "labels": ["hvac", "billing", "housekeeping", "noise", "safety"],
            "multi_label": True,
            "cls_threshold": 0.4,
        },
    },
)
```

Potential output:

```text
{
  "intent": "room_change",
  "priority": "high",
  "needs_human": "yes",
  "topics": ["hvac", "billing"]
}
```

### Question over a passage

The text is the source. The question is the task. Yes or no is the whole answer: no chain of thought, no extracted sentence, just the decision a checker or a form needs.

```python
model.classify_text(
    "The treaty was signed in Paris in 1992. It entered into force the following year, after the last signatory ratified it.",
    {"answer": {
        "labels": ["yes", "no"],
        "prompt": "Did the treaty enter into force in 1992?",
    }},
)
```

Potential output:

```text
{"answer": "no"}
```

### Book

A catalog, a slush pile, or a library inbox needs a genre before anyone writes a blurb. The passage is enough. The label set is the shelf list.

```python
model.classify_text(
    "She closed the ledger, blew out the lamp, and listened for the stair. The house had been empty since the winter the river took the bridge.",
    {"genre": ["mystery", "romance", "history", "science_fiction", "literary_fiction", "cookbook"]},
)
```

Potential output:

```text
{"genre": "literary_fiction"}
```

### Labels with a description

When the name of a label is not enough, pass a short description with it. The description is part of the decision, which is how a private taxonomy stays precise without a bigger model.

```python
model.classify_text(
    "Please reset the card PIN. The new one never arrived and the old one is locked after three tries.",
    {"intent": {
        "labels": {
            "card_pin_change": "The customer wants a new PIN or the current PIN replaced",
            "card_lost": "The physical card is missing",
            "balance_inquiry": "The customer wants the current balance",
        },
    }},
)
```

Potential output:

```text
{"intent": "card_pin_change"}
```

### Ordinal score

Some products want a rank, not a class. Pass the scale as ordinary strings, from `"0"` to `"10"`. A review, a rubric, or a satisfaction form can read the label directly.

```python
model.classify_text(
    "I finished it in two nights. The ending is earned, the middle drags, and I would still hand it to a friend.",
    {"rating": ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]},
)
```

Potential output:

```text
{"rating": "7"}
```

## Fine-tuning

Every schema on this card has a training form. One JSONL row holds the text and the correct answer for each decision. Each object in `classifications` is one decision, with the same `task`, `labels`, `prompt` and `multi_label` you pass to `classify_text`.

| On this card | In a training row |
| --- | --- |
| `{"intent": [...]}` | `{"task": "intent", "labels": [...], "true_label": ["refund_request"]}` |
| `"multi_label": True` | `"multi_label": true`, with every correct label in `true_label`. `cls_threshold` is inference-only |
| `"prompt": "Did …?"` | the same `"prompt"` |
| `"labels": {name: description}` | `"labels": [names]` plus `"label_descriptions": {name: description}` |
| ordinal `["0", …, "10"]` | an ordinary single-label row, for example `"true_label": ["7"]` |
| several decisions in one call | several objects in `classifications` |

```jsonl
{"input": "My subscription renewed after the service was already down. Can I get that charge refunded?", "output": {"classifications": [{"task": "intent", "labels": ["order_status", "refund_request", "cancel_subscription", "other"], "true_label": ["refund_request"]}]}}
{"input": "Battery dies before lunch, but the keyboard and the screen are great.", "output": {"classifications": [{"task": "aspects", "labels": ["battery", "keyboard", "screen", "camera", "price"], "true_label": ["battery", "keyboard", "screen"], "multi_label": true}]}}
{"input": "The treaty was signed in Paris in 1992 and entered into force the following year.", "output": {"classifications": [{"task": "answer", "labels": ["yes", "no"], "prompt": "Did the treaty enter into force in 1992?", "true_label": ["no"]}]}}
{"input": "Please reset the card PIN. The old one is locked after three tries.", "output": {"classifications": [{"task": "intent", "labels": ["card_pin_change", "card_lost", "balance_inquiry"], "label_descriptions": {"card_pin_change": "The customer wants a new PIN or the current PIN replaced", "card_lost": "The physical card is missing", "balance_inquiry": "The customer wants the current balance"}, "true_label": ["card_pin_change"]}]}}
{"input": "I finished it in two nights. The ending is earned and the middle drags.", "output": {"classifications": [{"task": "rating", "labels": ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"], "true_label": ["7"]}]}}
```

```python
# pip install "gliner2[train]"
from gliner2 import AutoExtractor
from gliner2.training.trainer import ExtractorTrainer, TrainingConfig

model = AutoExtractor.from_pretrained("fastino/GLiNER2.5-Decide")
config = TrainingConfig(output_dir="out", num_epochs=3, batch_size=8)
ExtractorTrainer(model, config).train(train_data="train.jsonl")
tuned = AutoExtractor.from_pretrained("out/final")
```

An ordinal scale trains as plain classes. Each label is scored on its own, and the loss does not know that `"6"` is closer to `"7"` than `"0"` is. Measure an ordinal head with mean absolute error as well as accuracy.

More row formats are in the [GLiNER2 training data guide](https://github.com/fastino-ai/GLiNER2/blob/main/tutorial/8-train_data.md). Or describe the task to [Fastino Agent](https://agent.fastino.ai) and it builds the data and runs the fine-tune for you.

## Details

- **What it is:** a specialist classifier for operational decisions, including questions over a passage, books, described labels, and ordinal scores
- **What it is not:** a general-purpose model. No reasoning, no explanations, no open-ended answers.
- **Encoder:** DeBERTa-v3-large
- **Parameters:** 340M
- **Runs on:** CPU or GPU, through `gliner2`
- **License:** Apache 2.0

## Citation

```bibtex
@misc{zaratiana2025gliner2efficientmultitaskinformation,
      title={GLiNER2: An Efficient Multi-Task Information Extraction System with Schema-Driven Interface},
      author={Urchade Zaratiana and Gil Pasternak and Oliver Boyd and George Hurn-Maloney and Ash Lewis},
      year={2025},
      eprint={2507.18546},
      archivePrefix={arXiv},
      primaryClass={cs.CL},
      url={https://arxiv.org/abs/2507.18546},
}
```
