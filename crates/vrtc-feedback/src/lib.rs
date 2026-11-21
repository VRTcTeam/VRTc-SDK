//! Native human-feedback, review, and search-quality evaluation engine.
//!
//! This is VRTc's own human-in-the-loop workflow: review tasks, versioned
//! relevance judgments, disagreement detection, adjudication, and offline
//! ranking metrics. Storage is in-memory in this initial implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct FeedbackConfig {
    pub max_tasks: usize,
    pub max_candidates_per_task: usize,
    pub max_judgments_per_task: usize,
    pub max_notes_chars: usize,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            max_tasks: 10_000,
            max_candidates_per_task: 200,
            max_judgments_per_task: 2_000,
            max_notes_chars: 2_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCandidate {
    pub document_id: String,
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewTaskStatus {
    Pending,
    InProgress,
    NeedsAdjudication,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTask {
    pub id: String,
    pub query: String,
    pub rubric_version: String,
    pub candidates: Vec<ReviewCandidate>,
    pub status: ReviewTaskStatus,
    pub created_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgmentRole {
    Reviewer,
    Adjudicator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelevanceJudgment {
    pub task_id: String,
    pub document_id: String,
    pub reviewer_id: String,
    /// Graded relevance: 0 = not relevant, 1 = marginal, 2 = relevant, 3 = highly relevant.
    pub relevance: u8,
    pub rubric_version: String,
    pub role: JudgmentRole,
    pub notes: Option<String>,
    pub submitted_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTaskSnapshot {
    pub task: ReviewTask,
    pub judgments: Vec<RelevanceJudgment>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankingMetrics {
    pub cutoff: usize,
    pub precision_at_k: f64,
    pub recall_at_k: f64,
    pub mean_reciprocal_rank: f64,
    pub ndcg_at_k: f64,
    pub total_relevant: usize,
    pub retrieved_relevant: usize,
    pub inter_rater_agreement: f64,
    pub disagreement_count: usize,
}

#[derive(Default)]
struct FeedbackState {
    next_task_id: u64,
    tasks: BTreeMap<String, ReviewTask>,
    judgments: BTreeMap<String, Vec<RelevanceJudgment>>,
}

pub struct FeedbackEngine {
    config: FeedbackConfig,
    state: Mutex<FeedbackState>,
}

impl FeedbackEngine {
    pub fn new(config: FeedbackConfig) -> Result<Self, VrtCError> {
        if config.max_tasks == 0
            || config.max_candidates_per_task == 0
            || config.max_judgments_per_task == 0
            || config.max_notes_chars == 0
        {
            return Err(feedback_error(
                "INVALID_FEEDBACK_CONFIG",
                "Task, candidate, judgment, and note limits must be positive",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(FeedbackState::default()),
        })
    }

    /// Create a bounded review task with a versioned relevance rubric.
    pub fn create_task(
        &self,
        query: &str,
        rubric_version: &str,
        candidates: Vec<ReviewCandidate>,
    ) -> Result<ReviewTask, VrtCError> {
        if query.trim().is_empty() || rubric_version.trim().is_empty() {
            return Err(feedback_error(
                "INVALID_REVIEW_TASK",
                "Review query and rubric version are required",
                ErrorCategory::Validation,
            ));
        }
        if candidates.is_empty() || candidates.len() > self.config.max_candidates_per_task {
            return Err(feedback_error(
                "INVALID_REVIEW_CANDIDATE_COUNT",
                "Review task candidate count is outside the configured range",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let mut ids = BTreeSet::new();
        for candidate in &candidates {
            if candidate.document_id.trim().is_empty() || !ids.insert(candidate.document_id.clone())
            {
                return Err(feedback_error(
                    "INVALID_REVIEW_CANDIDATE",
                    "Review candidates require unique, non-empty document IDs",
                    ErrorCategory::Validation,
                ));
            }
        }

        let mut state = self.lock_state()?;
        if state.tasks.len() >= self.config.max_tasks {
            return Err(feedback_error(
                "REVIEW_TASK_CAPACITY_REACHED",
                "The review task store has reached its configured capacity",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let id = format!("review-task-{}", state.next_task_id);
        state.next_task_id = state.next_task_id.saturating_add(1);
        let task = ReviewTask {
            id: id.clone(),
            query: query.to_owned(),
            rubric_version: rubric_version.to_owned(),
            candidates,
            status: ReviewTaskStatus::Pending,
            created_at: SystemTime::now(),
        };
        state.tasks.insert(id.clone(), task.clone());
        state.judgments.insert(id, Vec::new());
        Ok(task)
    }

    /// List a bounded page of review tasks, optionally filtered by lifecycle status.
    ///
    /// The returned tasks are snapshots; use `snapshot` to inspect judgments.
    pub fn list_tasks(
        &self,
        status: Option<ReviewTaskStatus>,
        limit: usize,
    ) -> Result<Vec<ReviewTask>, VrtCError> {
        if limit == 0 || limit > self.config.max_tasks {
            return Err(feedback_error(
                "INVALID_REVIEW_TASK_LIMIT",
                "Review task page size is outside the configured range",
                ErrorCategory::Validation,
            ));
        }
        let state = self.lock_state()?;
        Ok(state
            .tasks
            .values()
            .filter(|task| status.is_none_or(|expected| task.status == expected))
            .take(limit)
            .cloned()
            .collect())
    }

    /// Add one reviewer judgment. A reviewer cannot silently overwrite their own label.
    pub fn submit_judgment(
        &self,
        task_id: &str,
        document_id: &str,
        reviewer_id: &str,
        relevance: u8,
        rubric_version: &str,
        notes: Option<String>,
    ) -> Result<RelevanceJudgment, VrtCError> {
        self.validate_judgment(reviewer_id, relevance, rubric_version, notes.as_deref())?;
        let mut state = self.lock_state()?;
        let task = state
            .tasks
            .get(task_id)
            .ok_or_else(|| task_not_found(task_id))?;
        if task.rubric_version != rubric_version {
            return Err(feedback_error(
                "RUBRIC_VERSION_MISMATCH",
                "Judgment rubric version does not match the review task",
                ErrorCategory::Validation,
            ));
        }
        if !task
            .candidates
            .iter()
            .any(|candidate| candidate.document_id == document_id)
        {
            return Err(feedback_error(
                "DOCUMENT_NOT_IN_REVIEW_TASK",
                "The document is not part of this review task",
                ErrorCategory::Validation,
            ));
        }
        let judgments = state
            .judgments
            .get(task_id)
            .expect("task judgment list exists");
        if judgments.len() >= self.config.max_judgments_per_task {
            return Err(feedback_error(
                "REVIEW_JUDGMENT_CAPACITY_REACHED",
                "The review task has reached its judgment limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        if judgments.iter().any(|judgment| {
            judgment.document_id == document_id
                && judgment.reviewer_id == reviewer_id
                && judgment.role == JudgmentRole::Reviewer
        }) {
            return Err(feedback_error(
                "DUPLICATE_REVIEWER_JUDGMENT",
                "This reviewer has already judged this document in the task",
                ErrorCategory::Validation,
            ));
        }
        let judgment = RelevanceJudgment {
            task_id: task_id.to_owned(),
            document_id: document_id.to_owned(),
            reviewer_id: reviewer_id.to_owned(),
            relevance,
            rubric_version: rubric_version.to_owned(),
            role: JudgmentRole::Reviewer,
            notes,
            submitted_at: SystemTime::now(),
        };
        state
            .judgments
            .get_mut(task_id)
            .expect("task judgment list exists")
            .push(judgment.clone());
        refresh_task_status(&mut state, task_id)?;
        Ok(judgment)
    }

    /// Resolve a documented disagreement without deleting the original judgments.
    pub fn adjudicate(
        &self,
        task_id: &str,
        document_id: &str,
        adjudicator_id: &str,
        relevance: u8,
        rubric_version: &str,
        notes: Option<String>,
    ) -> Result<RelevanceJudgment, VrtCError> {
        self.validate_judgment(adjudicator_id, relevance, rubric_version, notes.as_deref())?;
        let mut state = self.lock_state()?;
        let task = state
            .tasks
            .get(task_id)
            .ok_or_else(|| task_not_found(task_id))?;
        if task.rubric_version != rubric_version {
            return Err(feedback_error(
                "RUBRIC_VERSION_MISMATCH",
                "Adjudication rubric version does not match the review task",
                ErrorCategory::Validation,
            ));
        }
        if !task
            .candidates
            .iter()
            .any(|candidate| candidate.document_id == document_id)
        {
            return Err(feedback_error(
                "DOCUMENT_NOT_IN_REVIEW_TASK",
                "The document is not part of this review task",
                ErrorCategory::Validation,
            ));
        }
        let judgments = state
            .judgments
            .get(task_id)
            .expect("task judgment list exists");
        let labels = judgments
            .iter()
            .filter(|judgment| {
                judgment.document_id == document_id && judgment.role == JudgmentRole::Reviewer
            })
            .map(|judgment| judgment.relevance)
            .collect::<Vec<_>>();
        if !has_material_disagreement(&labels) {
            return Err(feedback_error(
                "ADJUDICATION_NOT_REQUIRED",
                "Adjudication requires at least two reviewer judgments with a material disagreement",
                ErrorCategory::Conflict,
            ));
        }
        if judgments.iter().any(|judgment| {
            judgment.document_id == document_id && judgment.role == JudgmentRole::Adjudicator
        }) {
            return Err(feedback_error(
                "DUPLICATE_ADJUDICATION",
                "This document already has an adjudication",
                ErrorCategory::Conflict,
            ));
        }
        if judgments.len() >= self.config.max_judgments_per_task {
            return Err(feedback_error(
                "REVIEW_JUDGMENT_CAPACITY_REACHED",
                "The review task has reached its judgment limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let judgment = RelevanceJudgment {
            task_id: task_id.to_owned(),
            document_id: document_id.to_owned(),
            reviewer_id: adjudicator_id.to_owned(),
            relevance,
            rubric_version: rubric_version.to_owned(),
            role: JudgmentRole::Adjudicator,
            notes,
            submitted_at: SystemTime::now(),
        };
        state
            .judgments
            .get_mut(task_id)
            .expect("task judgment list exists")
            .push(judgment.clone());
        refresh_task_status(&mut state, task_id)?;
        Ok(judgment)
    }

    pub fn snapshot(&self, task_id: &str) -> Result<ReviewTaskSnapshot, VrtCError> {
        let state = self.lock_state()?;
        let task = state
            .tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| task_not_found(task_id))?;
        let judgments = state.judgments.get(task_id).cloned().unwrap_or_default();
        Ok(ReviewTaskSnapshot { task, judgments })
    }

    /// Evaluate a ranking against the task's human judgments.
    ///
    /// Unjudged results are treated as non-relevant for this pooled evaluation;
    /// callers should expand the review pool before treating the metrics as final.
    pub fn evaluate_ranking(
        &self,
        task_id: &str,
        ranked_document_ids: &[String],
        cutoff: usize,
    ) -> Result<RankingMetrics, VrtCError> {
        if cutoff == 0 {
            return Err(feedback_error(
                "INVALID_EVALUATION_CUTOFF",
                "Evaluation cutoff must be positive",
                ErrorCategory::Validation,
            ));
        }
        let snapshot = self.snapshot(task_id)?;
        let relevance = aggregate_relevance(&snapshot.judgments);
        let total_relevant = relevance.values().filter(|score| **score >= 2.0).count();
        let mut seen = BTreeSet::new();
        let ranked = ranked_document_ids
            .iter()
            .filter(|id| seen.insert((*id).clone()))
            .take(cutoff)
            .cloned()
            .collect::<Vec<_>>();
        let retrieved_relevant = ranked
            .iter()
            .filter(|id| relevance.get(*id).copied().unwrap_or_default() >= 2.0)
            .count();
        let precision_at_k = retrieved_relevant as f64 / cutoff as f64;
        let recall_at_k = if total_relevant == 0 {
            0.0
        } else {
            retrieved_relevant as f64 / total_relevant as f64
        };
        let mean_reciprocal_rank = ranked
            .iter()
            .position(|id| relevance.get(id).copied().unwrap_or_default() >= 2.0)
            .map(|position| 1.0 / (position + 1) as f64)
            .unwrap_or_default();
        let ndcg_at_k = calculate_ndcg(&ranked, &relevance, cutoff);
        let (inter_rater_agreement, disagreement_count) = agreement_metrics(&snapshot.judgments);

        Ok(RankingMetrics {
            cutoff,
            precision_at_k,
            recall_at_k,
            mean_reciprocal_rank,
            ndcg_at_k,
            total_relevant,
            retrieved_relevant,
            inter_rater_agreement,
            disagreement_count,
        })
    }

    fn validate_judgment(
        &self,
        reviewer_id: &str,
        relevance: u8,
        rubric_version: &str,
        notes: Option<&str>,
    ) -> Result<(), VrtCError> {
        if reviewer_id.trim().is_empty() || rubric_version.trim().is_empty() || relevance > 3 {
            return Err(feedback_error(
                "INVALID_RELEVANCE_JUDGMENT",
                "Reviewer ID, rubric version, and a relevance label from 0 to 3 are required",
                ErrorCategory::Validation,
            ));
        }
        if notes.is_some_and(|value| value.chars().count() > self.config.max_notes_chars) {
            return Err(feedback_error(
                "REVIEW_NOTES_TOO_LONG",
                "Reviewer notes exceed the configured character limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, FeedbackState>, VrtCError> {
        self.state.lock().map_err(|_| {
            feedback_error(
                "FEEDBACK_STORE_LOCK_POISONED",
                "The human-feedback store is temporarily unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn refresh_task_status(state: &mut FeedbackState, task_id: &str) -> Result<(), VrtCError> {
    let task = state
        .tasks
        .get(task_id)
        .cloned()
        .ok_or_else(|| task_not_found(task_id))?;
    let judgments = state.judgments.get(task_id).cloned().unwrap_or_default();
    let mut complete = true;
    let mut needs_adjudication = false;

    for candidate in &task.candidates {
        let reviewer_labels = judgments
            .iter()
            .filter(|judgment| {
                judgment.document_id == candidate.document_id
                    && judgment.role == JudgmentRole::Reviewer
            })
            .map(|judgment| judgment.relevance)
            .collect::<Vec<_>>();
        let adjudicated = judgments.iter().any(|judgment| {
            judgment.document_id == candidate.document_id
                && judgment.role == JudgmentRole::Adjudicator
        });
        if reviewer_labels.is_empty() && !adjudicated {
            complete = false;
        }
        if !adjudicated && has_material_disagreement(&reviewer_labels) {
            needs_adjudication = true;
        }
    }

    let status = if needs_adjudication {
        ReviewTaskStatus::NeedsAdjudication
    } else if complete {
        ReviewTaskStatus::Completed
    } else if judgments.is_empty() {
        ReviewTaskStatus::Pending
    } else {
        ReviewTaskStatus::InProgress
    };
    if let Some(task) = state.tasks.get_mut(task_id) {
        task.status = status;
    }
    Ok(())
}

fn has_material_disagreement(labels: &[u8]) -> bool {
    if labels.len() < 2 {
        return false;
    }
    let highest = labels.iter().copied().max().unwrap_or_default();
    let lowest = labels.iter().copied().min().unwrap_or_default();
    highest.saturating_sub(lowest) >= 2
}

fn aggregate_relevance(judgments: &[RelevanceJudgment]) -> BTreeMap<String, f64> {
    let mut reviewer_labels = BTreeMap::<String, Vec<u8>>::new();
    let mut adjudicated = BTreeMap::<String, u8>::new();
    for judgment in judgments {
        match judgment.role {
            JudgmentRole::Reviewer => reviewer_labels
                .entry(judgment.document_id.clone())
                .or_default()
                .push(judgment.relevance),
            JudgmentRole::Adjudicator => {
                adjudicated.insert(judgment.document_id.clone(), judgment.relevance);
            }
        }
    }
    let mut output = BTreeMap::new();
    for (document_id, labels) in reviewer_labels {
        let score = adjudicated
            .get(&document_id)
            .map(|value| f64::from(*value))
            .unwrap_or_else(|| {
                labels.iter().map(|value| f64::from(*value)).sum::<f64>() / labels.len() as f64
            });
        output.insert(document_id, score);
    }
    for (document_id, label) in adjudicated {
        output.entry(document_id).or_insert(f64::from(label));
    }
    output
}

fn calculate_ndcg(ranked: &[String], relevance: &BTreeMap<String, f64>, cutoff: usize) -> f64 {
    let dcg = ranked
        .iter()
        .take(cutoff)
        .enumerate()
        .map(|(index, id)| {
            let score = relevance.get(id).copied().unwrap_or_default();
            (2_f64.powf(score) - 1.0) / ((index + 2) as f64).log2()
        })
        .sum::<f64>();
    let mut ideal = relevance.values().copied().collect::<Vec<_>>();
    ideal.sort_by(|left, right| right.total_cmp(left));
    let ideal_dcg = ideal
        .iter()
        .take(cutoff)
        .enumerate()
        .map(|(index, score)| (2_f64.powf(*score) - 1.0) / ((index + 2) as f64).log2())
        .sum::<f64>();
    if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg / ideal_dcg
    }
}

fn agreement_metrics(judgments: &[RelevanceJudgment]) -> (f64, usize) {
    let mut ratings = BTreeMap::<&str, Vec<u8>>::new();
    for judgment in judgments
        .iter()
        .filter(|judgment| judgment.role == JudgmentRole::Reviewer)
    {
        ratings
            .entry(&judgment.document_id)
            .or_default()
            .push(judgment.relevance);
    }
    let multi_rated = ratings
        .values()
        .filter(|labels| labels.len() >= 2)
        .collect::<Vec<_>>();
    if multi_rated.is_empty() {
        return (0.0, 0);
    }
    let agreements = multi_rated
        .iter()
        .filter(|labels| labels.iter().all(|label| *label == labels[0]))
        .count();
    let disagreements = multi_rated.len() - agreements;
    (agreements as f64 / multi_rated.len() as f64, disagreements)
}

fn task_not_found(task_id: &str) -> VrtCError {
    VrtCError::new(
        "REVIEW_TASK_NOT_FOUND",
        "The requested review task does not exist",
        ErrorCategory::NotFound,
        false,
    )
    .for_component(task_id)
}

fn feedback_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> ReviewCandidate {
        ReviewCandidate {
            document_id: id.to_owned(),
            url: format!("https://example.org/{id}"),
            title: format!("Document {id}"),
            snippet: "A test search result".to_owned(),
            source: "test".to_owned(),
        }
    }

    #[test]
    fn records_disagreement_adjudicates_and_evaluates_ranking() {
        let engine = FeedbackEngine::new(FeedbackConfig::default()).unwrap();
        let task = engine
            .create_task(
                "crawler scheduler",
                "rubric-v1",
                vec![candidate("a"), candidate("b")],
            )
            .unwrap();
        engine
            .submit_judgment(&task.id, "a", "reviewer-1", 3, "rubric-v1", None)
            .unwrap();
        engine
            .submit_judgment(&task.id, "a", "reviewer-2", 1, "rubric-v1", None)
            .unwrap();
        engine
            .submit_judgment(&task.id, "b", "reviewer-1", 1, "rubric-v1", None)
            .unwrap();
        engine
            .submit_judgment(&task.id, "b", "reviewer-2", 1, "rubric-v1", None)
            .unwrap();
        assert_eq!(
            engine.snapshot(&task.id).unwrap().task.status,
            ReviewTaskStatus::NeedsAdjudication
        );

        engine
            .adjudicate(
                &task.id,
                "a",
                "adjudicator-1",
                2,
                "rubric-v1",
                Some("Evidence supports relevance".to_owned()),
            )
            .unwrap();
        assert_eq!(
            engine.snapshot(&task.id).unwrap().task.status,
            ReviewTaskStatus::Completed
        );

        let metrics = engine
            .evaluate_ranking(&task.id, &["b".to_owned(), "a".to_owned()], 2)
            .unwrap();
        assert_eq!(metrics.total_relevant, 1);
        assert_eq!(metrics.retrieved_relevant, 1);
        assert_eq!(metrics.precision_at_k, 0.5);
        assert_eq!(metrics.recall_at_k, 1.0);
        assert_eq!(metrics.mean_reciprocal_rank, 0.5);
        assert_eq!(metrics.disagreement_count, 1);
        assert_eq!(metrics.inter_rater_agreement, 0.5);
    }

    #[test]
    fn lists_tasks_by_status_with_a_bounded_page_size() {
        let engine = FeedbackEngine::new(FeedbackConfig::default()).unwrap();
        let first = engine
            .create_task("first query", "rubric-v1", vec![candidate("a")])
            .unwrap();
        let second = engine
            .create_task("second query", "rubric-v1", vec![candidate("b")])
            .unwrap();
        engine
            .submit_judgment(&second.id, "b", "reviewer-1", 2, "rubric-v1", None)
            .unwrap();

        let pending = engine
            .list_tasks(Some(ReviewTaskStatus::Pending), 10)
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, first.id);
        let all = engine.list_tasks(None, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert!(engine.list_tasks(None, 0).is_err());
    }

    #[test]
    fn prevents_duplicate_judgments_and_invalid_labels() {
        let engine = FeedbackEngine::new(FeedbackConfig::default()).unwrap();
        let task = engine
            .create_task("search query", "rubric-v1", vec![candidate("doc")])
            .unwrap();
        engine
            .submit_judgment(&task.id, "doc", "reviewer", 2, "rubric-v1", None)
            .unwrap();
        assert!(engine
            .submit_judgment(&task.id, "doc", "reviewer", 3, "rubric-v1", None)
            .is_err());
        assert!(engine
            .submit_judgment(&task.id, "doc", "reviewer-2", 4, "rubric-v1", None)
            .is_err());
    }
}
