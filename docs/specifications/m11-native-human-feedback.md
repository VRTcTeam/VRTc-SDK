# M11 — Native Human Feedback & Search Evaluation

**Status:** Initial native engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.feedback`  
**Capability:** `human.review`

## Research basis and adopted patterns

VRTc studied the human-feedback workflows documented by:

- **Argilla:** dataset-centric annotation, response correction, preference/relevance feedback, reviewer workflows, and dataset curation.
- **Label Studio:** configurable annotation tasks, reviewer roles, task assignment concepts, and structured annotation output.
- **Search relevance rating practice:** graded relevance labels, pooled candidate sets, independent ratings, adjudication, and offline ranking evaluation.
- **Human approval workflows:** explicit review gates for exceptional or policy-sensitive operations, rather than hidden approval in every normal query.

VRTc does not embed either product or depend on a hosted annotation service. It implements a compact, native review/evaluation contract and can later expose adapters or a separate reviewer UI.

## Native capabilities

- Create bounded review tasks containing a query, candidate documents, and an immutable rubric-version identifier.
- List tasks with an optional lifecycle-status filter and a bounded page size.
- Read a task snapshot with its current judgments.
- Record independent reviewer judgments without silently overwriting an existing reviewer label.
- Use a graded relevance scale:
  - `0`: not relevant
  - `1`: marginally relevant
  - `2`: relevant
  - `3`: highly relevant
- Detect material disagreement when two or more reviewer labels for a document differ by at least two points.
- Record an adjudicator's resolution while preserving original reviewer judgments.
- Track task status: pending, in progress, needs adjudication, completed.
- Evaluate a ranked document list using Precision@k, Recall@k, MRR, nDCG@k, exact agreement rate, and disagreement count.
- Enforce task, candidate, judgment, note-length, and evaluation page/cutoff bounds.

## Review lifecycle

```text
Create task + rubric version
          |
          v
       Pending
          |
     Reviewer labels
          |
          v
      In progress
          |
   +------+----------------+
   |                       |
No material conflict    Material disagreement
   |                       |
   v                       v
Completed             Needs adjudication
                           |
                     Adjudicator label
                           |
                           v
                       Completed
```

A task is completed when every candidate has at least one reviewer judgment or an adjudication. If material disagreement remains unresolved, the task remains in `NeedsAdjudication`.

## Metric definitions

- **Precision@k:** relevant retrieved items in the first k positions divided by k.
- **Recall@k:** relevant retrieved items in the first k positions divided by all relevant items in the reviewed candidate pool.
- **MRR:** reciprocal rank of the first relevant result for this task's single query.
- **nDCG@k:** discounted cumulative gain using graded relevance (gain (2^r - 1)), normalized by the ideal ordering of the reviewed pool.
- **Exact agreement rate:** fraction of multi-rated documents for which every reviewer assigned the same label. This is a simple descriptive agreement measure, **not Cohen's kappa or Krippendorff's alpha**.
- **Disagreement count:** number of documents with multiple reviewer labels that are not unanimous.

## Important evaluation semantics

- Relevance is aggregated using the adjudicator label when present; otherwise, the arithmetic mean of reviewer labels is used.
- A score of 2 or greater counts as relevant for Precision@k, Recall@k, and MRR.
- Unjudged results are treated as non-relevant, and only the task's pooled candidates contribute to the denominator. Expand the review pool before interpreting metrics as a complete measure of search quality.
- Precision@k uses k as the denominator even when fewer results were supplied.
- Metrics are per-task and must not be compared across different queries or rubric versions without a deliberate evaluation protocol.
- The engine records reviewer IDs as caller-supplied identifiers. Deployments should use pseudonymous IDs, enforce authentication/authorization at the API host, and avoid putting personal information in notes.
- Rubric definitions are versioned by identifier. The host application owns the actual rubric text, reviewer access policy, retention policy, and user interface.

## Storage and boundaries

- The initial implementation uses a bounded in-memory store. Tasks and judgments are lost when the process stops.
- The runtime does not claim durable storage, distributed task assignment, reviewer authentication, reviewer identity verification, or a browser-based annotation UI.
- Human judgments are evidence about relevance, not automatically ground truth. Preserve reviewer provenance and disagreement.
- This engine evaluates ranking quality; it does not automatically train or mutate ranking weights from labels.
- Human review is an explicit workflow for evaluation, disagreement resolution, and approvals. It is not a mandatory pause on every search request.

## Runtime integration

The engine is statically registered by `VrtcRuntime`, exposed through `feedback_engine()`, and declares `human.review` operations for task creation, task listing, judgment submission, adjudication, and ranking evaluation. HTTP routes and language SDK methods are subsequent integration work.
