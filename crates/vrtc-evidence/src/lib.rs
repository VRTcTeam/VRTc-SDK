//! Native VRTc evidence provenance and claim-verification ledger.
//!
//! This engine validates evidence structure and records explicit assessments.
//! It does not infer semantic entailment or treat source count as proof.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct EvidenceConfig {
    pub max_claims: usize,
    pub max_evidence: usize,
    pub max_evidence_per_claim: usize,
    pub max_statement_bytes: usize,
    pub max_excerpt_bytes: usize,
    pub max_rationale_bytes: usize,
    pub minimum_independent_hosts: usize,
    pub minimum_assessment_confidence: f64,
}

impl Default for EvidenceConfig {
    fn default() -> Self {
        Self {
            max_claims: 10_000,
            max_evidence: 100_000,
            max_evidence_per_claim: 200,
            max_statement_bytes: 16 * 1024,
            max_excerpt_bytes: 8 * 1024,
            max_rationale_bytes: 4 * 1024,
            minimum_independent_hosts: 2,
            minimum_assessment_confidence: 0.65,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: String,
    pub statement: String,
    pub created_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceRecord {
    pub id: String,
    pub claim_id: String,
    pub source_id: String,
    pub source_url: String,
    pub source_host: String,
    pub source_title: String,
    pub excerpt: String,
    pub content_hash: String,
    pub retrieved_at: SystemTime,
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceInput {
    pub id: String,
    pub claim_id: String,
    pub source_id: String,
    pub source_url: String,
    pub source_title: String,
    pub excerpt: String,
    pub content_hash: String,
    pub retrieved_at: SystemTime,
    pub extraction_confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceRelation {
    Supports,
    Contradicts,
    Context,
    Unclear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssessmentMethod {
    Human,
    RuleBased,
    ModelAssisted,
    ExternalSystem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidenceAssessment {
    pub evidence_id: String,
    pub relation: EvidenceRelation,
    pub confidence: f64,
    pub assessor_id: String,
    pub method: AssessmentMethod,
    pub rationale: String,
    pub assessed_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssessmentInput {
    pub evidence_id: String,
    pub relation: EvidenceRelation,
    pub confidence: f64,
    pub assessor_id: String,
    pub method: AssessmentMethod,
    pub rationale: String,
    pub assessed_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    InsufficientEvidence,
    NeedsReview,
    Supported,
    Contradicted,
    Mixed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimVerification {
    pub claim_id: String,
    pub status: VerificationStatus,
    pub evidence_count: usize,
    pub assessed_evidence_count: usize,
    pub unassessed_evidence_count: usize,
    pub supporting_evidence_count: usize,
    pub contradicting_evidence_count: usize,
    pub supporting_host_count: usize,
    pub contradicting_host_count: usize,
    pub mean_assessment_confidence: Option<f64>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceStats {
    pub claims: usize,
    pub evidence_records: usize,
    pub assessments: usize,
}

#[derive(Default)]
struct EvidenceState {
    claims: HashMap<String, Claim>,
    evidence: HashMap<String, EvidenceRecord>,
    by_claim: HashMap<String, Vec<String>>,
    assessments: HashMap<String, EvidenceAssessment>,
}

pub struct EvidenceEngine {
    config: EvidenceConfig,
    state: Mutex<EvidenceState>,
}

impl EvidenceEngine {
    pub fn new(config: EvidenceConfig) -> Result<Self, VrtCError> {
        if config.max_claims == 0
            || config.max_evidence == 0
            || config.max_evidence_per_claim == 0
            || config.max_statement_bytes == 0
            || config.max_excerpt_bytes == 0
            || config.max_rationale_bytes == 0
            || config.minimum_independent_hosts == 0
            || !config.minimum_assessment_confidence.is_finite()
            || !(0.0..=1.0).contains(&config.minimum_assessment_confidence)
        {
            return Err(evidence_error(
                "INVALID_EVIDENCE_CONFIG",
                "Evidence limits and confidence thresholds must be positive and valid",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(EvidenceState::default()),
        })
    }

    pub fn create_claim(
        &self,
        id: &str,
        statement: &str,
        created_at: SystemTime,
    ) -> Result<Claim, VrtCError> {
        if id.trim().is_empty()
            || statement.trim().is_empty()
            || statement.len() > self.config.max_statement_bytes
        {
            return Err(evidence_error(
                "INVALID_CLAIM",
                "Claim ID and statement must be non-empty and within the configured limit",
                ErrorCategory::Validation,
            ));
        }
        let mut state = self.lock_state()?;
        if state.claims.contains_key(id) {
            return Err(evidence_error(
                "DUPLICATE_CLAIM_ID",
                "A claim with this identifier already exists",
                ErrorCategory::DuplicateId,
            ));
        }
        if state.claims.len() >= self.config.max_claims {
            return Err(evidence_error(
                "CLAIM_CAPACITY_REACHED",
                "The evidence engine has reached its claim capacity",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let claim = Claim {
            id: id.to_owned(),
            statement: statement.trim().to_owned(),
            created_at,
        };
        state.claims.insert(id.to_owned(), claim.clone());
        state.by_claim.insert(id.to_owned(), Vec::new());
        Ok(claim)
    }

    pub fn attach_evidence(
        &self,
        input: EvidenceInput,
    ) -> Result<EvidenceRecord, VrtCError> {
        if input.id.trim().is_empty()
            || input.source_id.trim().is_empty()
            || input.source_title.trim().is_empty()
            || input.excerpt.trim().is_empty()
            || input.excerpt.len() > self.config.max_excerpt_bytes
            || !input.extraction_confidence.is_finite()
            || !(0.0..=1.0).contains(&input.extraction_confidence)
            || !is_sha256(&input.content_hash)
        {
            return Err(evidence_error(
                "INVALID_EVIDENCE_RECORD",
                "Evidence identifiers, excerpt, hash, and extraction confidence must be valid",
                ErrorCategory::Validation,
            ));
        }
        let (source_url, source_host) = canonicalize_source_url(&input.source_url)?;
        let mut state = self.lock_state()?;
        if !state.claims.contains_key(&input.claim_id) {
            return Err(evidence_error(
                "CLAIM_NOT_FOUND",
                "Evidence cannot be attached to an unknown claim",
                ErrorCategory::NotFound,
            ));
        }
        if state.evidence.contains_key(&input.id) {
            return Err(evidence_error(
                "DUPLICATE_EVIDENCE_ID",
                "An evidence record with this identifier already exists",
                ErrorCategory::DuplicateId,
            ));
        }
        if state.evidence.len() >= self.config.max_evidence {
            return Err(evidence_error(
                "EVIDENCE_CAPACITY_REACHED",
                "The evidence engine has reached its evidence capacity",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let linked = state.by_claim.get(&input.claim_id).map_or(0, Vec::len);
        if linked >= self.config.max_evidence_per_claim {
            return Err(evidence_error(
                "CLAIM_EVIDENCE_LIMIT_REACHED",
                "The claim has reached its evidence record limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let record = EvidenceRecord {
            id: input.id.clone(),
            claim_id: input.claim_id.clone(),
            source_id: input.source_id,
            source_url,
            source_host,
            source_title: input.source_title,
            excerpt: input.excerpt,
            content_hash: input.content_hash.to_ascii_lowercase(),
            retrieved_at: input.retrieved_at,
            extraction_confidence: input.extraction_confidence,
        };
        state.evidence.insert(input.id.clone(), record.clone());
        state
            .by_claim
            .entry(input.claim_id)
            .or_default()
            .push(input.id);
        Ok(record)
    }

    /// Record one explicit assessment. Semantic support is never inferred here.
    pub fn assess_evidence(
        &self,
        input: AssessmentInput,
    ) -> Result<EvidenceAssessment, VrtCError> {
        if input.assessor_id.trim().is_empty()
            || input.rationale.len() > self.config.max_rationale_bytes
            || !input.confidence.is_finite()
            || !(0.0..=1.0).contains(&input.confidence)
        {
            return Err(evidence_error(
                "INVALID_EVIDENCE_ASSESSMENT",
                "Assessor, rationale length, and confidence must be valid",
                ErrorCategory::Validation,
            ));
        }
        let mut state = self.lock_state()?;
        if !state.evidence.contains_key(&input.evidence_id) {
            return Err(evidence_error(
                "EVIDENCE_NOT_FOUND",
                "The requested evidence record does not exist",
                ErrorCategory::NotFound,
            ));
        }
        if state.assessments.contains_key(&input.evidence_id) {
            return Err(evidence_error(
                "DUPLICATE_EVIDENCE_ASSESSMENT",
                "Evidence already has a final assessment; create a new evidence version to reassess it",
                ErrorCategory::DuplicateId,
            ));
        }
        let assessment = EvidenceAssessment {
            evidence_id: input.evidence_id.clone(),
            relation: input.relation,
            confidence: input.confidence,
            assessor_id: input.assessor_id,
            method: input.method,
            rationale: input.rationale,
            assessed_at: input.assessed_at,
        };
        state
            .assessments
            .insert(input.evidence_id, assessment.clone());
        Ok(assessment)
    }

    pub fn verify_claim(&self, claim_id: &str) -> Result<ClaimVerification, VrtCError> {
        let state = self.lock_state()?;
        if !state.claims.contains_key(claim_id) {
            return Err(evidence_error(
                "CLAIM_NOT_FOUND",
                "The requested claim does not exist",
                ErrorCategory::NotFound,
            ));
        }
        let evidence_ids = state.by_claim.get(claim_id).cloned().unwrap_or_default();
        let mut supporting_hosts = HashSet::new();
        let mut contradicting_hosts = HashSet::new();
        let mut supporting = 0;
        let mut contradicting = 0;
        let mut assessed = 0;
        let mut confidence_sum = 0.0;
        let mut unassessed = 0;

        for id in &evidence_ids {
            let Some(assessment) = state.assessments.get(id) else {
                unassessed += 1;
                continue;
            };
            let Some(record) = state.evidence.get(id) else {
                unassessed += 1;
                continue;
            };
            assessed += 1;
            confidence_sum += assessment.confidence;
            match assessment.relation {
                EvidenceRelation::Supports => {
                    supporting += 1;
                    supporting_hosts.insert(record.source_host.clone());
                }
                EvidenceRelation::Contradicts => {
                    contradicting += 1;
                    contradicting_hosts.insert(record.source_host.clone());
                }
                EvidenceRelation::Context | EvidenceRelation::Unclear => {}
            }
        }

        let status = if evidence_ids.is_empty() {
            VerificationStatus::InsufficientEvidence
        } else if unassessed > 0 {
            VerificationStatus::NeedsReview
        } else if supporting > 0 && contradicting > 0 {
            VerificationStatus::Mixed
        } else if supporting > 0
            && supporting_hosts.len() >= self.config.minimum_independent_hosts
            && mean_confidence(&state, &evidence_ids) >= self.config.minimum_assessment_confidence
        {
            VerificationStatus::Supported
        } else if contradicting > 0
            && contradicting_hosts.len() >= self.config.minimum_independent_hosts
            && mean_confidence(&state, &evidence_ids) >= self.config.minimum_assessment_confidence
        {
            VerificationStatus::Contradicted
        } else {
            VerificationStatus::NeedsReview
        };

        Ok(ClaimVerification {
            claim_id: claim_id.to_owned(),
            status,
            evidence_count: evidence_ids.len(),
            assessed_evidence_count: assessed,
            unassessed_evidence_count: unassessed,
            supporting_evidence_count: supporting,
            contradicting_evidence_count: contradicting,
            supporting_host_count: supporting_hosts.len(),
            contradicting_host_count: contradicting_hosts.len(),
            mean_assessment_confidence: (assessed > 0).then_some(confidence_sum / assessed as f64),
            evidence_ids,
        })
    }

    pub fn evidence(&self, evidence_id: &str) -> Result<Option<EvidenceRecord>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state.evidence.get(evidence_id).cloned())
    }

    pub fn assessment(
        &self,
        evidence_id: &str,
    ) -> Result<Option<EvidenceAssessment>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state.assessments.get(evidence_id).cloned())
    }

    pub fn stats(&self) -> Result<EvidenceStats, VrtCError> {
        let state = self.lock_state()?;
        Ok(EvidenceStats {
            claims: state.claims.len(),
            evidence_records: state.evidence.len(),
            assessments: state.assessments.len(),
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, EvidenceState>, VrtCError> {
        self.state.lock().map_err(|_| {
            evidence_error(
                "EVIDENCE_LOCK_POISONED",
                "Evidence state is temporarily unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn mean_confidence(state: &EvidenceState, evidence_ids: &[String]) -> f64 {
    let values = evidence_ids
        .iter()
        .filter_map(|id| state.assessments.get(id))
        .filter(|assessment| {
            matches!(
                assessment.relation,
                EvidenceRelation::Supports | EvidenceRelation::Contradicts
            )
        })
        .map(|assessment| assessment.confidence)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn canonicalize_source_url(raw_url: &str) -> Result<(String, String), VrtCError> {
    let mut url = Url::parse(raw_url).map_err(|_| {
        evidence_error(
            "INVALID_EVIDENCE_URL",
            "Evidence source URL must be absolute HTTP or HTTPS",
            ErrorCategory::Validation,
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(evidence_error(
            "UNSUPPORTED_EVIDENCE_URL",
            "Evidence sources must use credential-free HTTP or HTTPS URLs",
            ErrorCategory::PolicyDenied,
        ));
    }
    url.set_fragment(None);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    Ok((url.to_string(), host))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn evidence_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn setup() -> EvidenceEngine {
        EvidenceEngine::new(EvidenceConfig::default()).unwrap()
    }

    fn add_evidence(engine: &EvidenceEngine, id: &str, host: &str) {
        engine
            .attach_evidence(
                id,
                "claim-1",
                "crawler",
                &format!("https://{host}/source"),
                "Source title",
                "A quoted excerpt that is relevant to the claim.",
                "a".repeat(64).as_str(),
                at(1),
                0.9,
            )
            .unwrap();
    }

    #[test]
    fn requires_independent_host_assessments_before_supported_status() {
        let engine = setup();
        engine
            .create_claim("claim-1", "The claim under review", at(0))
            .unwrap();
        add_evidence(&engine, "evidence-1", "one.example");
        add_evidence(&engine, "evidence-2", "two.example");
        for id in ["evidence-1", "evidence-2"] {
            engine
                .assess_evidence(
                    id,
                    EvidenceRelation::Supports,
                    0.9,
                    "reviewer-1",
                    AssessmentMethod::Human,
                    "The cited excerpt supports the claim.",
                    at(2),
                )
                .unwrap();
        }
        let verification = engine.verify_claim("claim-1").unwrap();
        assert_eq!(verification.status, VerificationStatus::Supported);
        assert_eq!(verification.supporting_host_count, 2);
    }

    #[test]
    fn mixed_evidence_is_not_collapsed_into_a_positive_result() {
        let engine = setup();
        engine
            .create_claim("claim-1", "The claim under review", at(0))
            .unwrap();
        add_evidence(&engine, "evidence-1", "one.example");
        add_evidence(&engine, "evidence-2", "two.example");
        engine
            .assess_evidence(
                "evidence-1",
                EvidenceRelation::Supports,
                0.9,
                "reviewer-1",
                AssessmentMethod::Human,
                "Supporting excerpt.",
                at(2),
            )
            .unwrap();
        engine
            .assess_evidence(
                "evidence-2",
                EvidenceRelation::Contradicts,
                0.9,
                "reviewer-2",
                AssessmentMethod::Human,
                "Contradicting excerpt.",
                at(2),
            )
            .unwrap();
        assert_eq!(
            engine.verify_claim("claim-1").unwrap().status,
            VerificationStatus::Mixed
        );
    }

    #[test]
    fn unassessed_or_same_host_evidence_requires_review() {
        let engine = setup();
        engine
            .create_claim("claim-1", "The claim under review", at(0))
            .unwrap();
        add_evidence(&engine, "evidence-1", "same.example");
        assert_eq!(
            engine.verify_claim("claim-1").unwrap().status,
            VerificationStatus::NeedsReview
        );
        engine
            .assess_evidence(
                "evidence-1",
                EvidenceRelation::Supports,
                0.95,
                "reviewer-1",
                AssessmentMethod::Human,
                "One source is not enough.",
                at(2),
            )
            .unwrap();
        assert_eq!(
            engine.verify_claim("claim-1").unwrap().status,
            VerificationStatus::NeedsReview
        );
    }

    #[test]
    fn empty_claims_and_invalid_provenance_are_rejected() {
        let engine = setup();
        assert_eq!(
            engine.verify_claim("missing").unwrap_err().code,
            "CLAIM_NOT_FOUND"
        );
        engine
            .create_claim("claim-1", "A valid claim", at(0))
            .unwrap();
        assert!(engine
            .attach_evidence(
                "bad",
                "claim-1",
                "crawler",
                "file:///etc/passwd",
                "Bad source",
                "Excerpt",
                "not-a-sha256",
                at(1),
                0.9,
            )
            .is_err());
    }
}
