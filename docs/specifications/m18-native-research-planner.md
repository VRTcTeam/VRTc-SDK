# M18 — Native Research Planner & Orchestration Graph

**Status:** Initial deterministic planner integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.research`  
**Capability:** `research.plan`

## Research basis

M18 translates established workflow patterns into a VRTc-owned planning contract:

- Temporal's Workflow/Activity separation and event-driven progression inform the distinction between a research plan and execution. Durable replay is not claimed by this initial planner.
- Airflow's DAG/task dependency model informs explicit step dependencies and topological ordering.
- Ray's task/actor distinction reinforces that parallel execution and stateful workers are separate concerns.
- LangGraph's checkpointed interrupt pattern informs the optional human-review gate. M18 only emits that gate; it does not implement a reviewer UI or resumable graph checkpoint.

These patterns are independently modeled. VRTc does not embed or require Temporal, Ray, Airflow, or LangGraph.

## Native planning contract

A request supplies:
- stable research request ID and query;
- maximum source count;
- explicit crawl opt-in and bounded crawl depth;
- optional human-review gate.

The planner creates a bounded graph with Search, optional Crawl/Extract/Index, Retrieve, Evidence Verification, optional Human Review, and Synthesis steps. Each step declares its VRTc capability and dependencies. A deterministic topological order is returned together with parallel groups.

## Execution semantics

- Search and Crawl are independent first-stage branches when crawling is enabled.
- Extract depends on Crawl; Index depends on Extract.
- Retrieve waits for Search and, when enabled, Index.
- Evidence Verification follows retrieval.
- Human Review is an explicit optional gate before Synthesis.
- Invalid limits, duplicate step IDs, missing dependencies, and cycles are rejected.
- The planner does not execute steps, access the network, invoke a model, or infer evidence truth.
- The existing M17 Job Queue is the execution hand-off point. Durable execution, retries, checkpoints, cancellation propagation, and worker pools remain separate runtime work.

## Safety and policy

Crawling is opt-in at request level. The planner only expresses a requested bounded crawl; actual fetching remains subject to the native crawler's robots, origin-scope, public-address, and resource policies. Planning never bypasses access controls or site restrictions.

## Cross-breeding decisions

| Reference pattern | VRTc-owned result |
|---|---|
| Temporal | Separate planning from durable execution and make recovery semantics explicit |
| Airflow | DAG dependencies and deterministic topological order |
| Ray | Parallel branches are metadata; worker execution is not conflated with planning |
| LangGraph | Optional human-review gate as a first-class workflow step |

No reference project's proprietary implementation is copied. Any future adopted library requires version-specific license and security review.
