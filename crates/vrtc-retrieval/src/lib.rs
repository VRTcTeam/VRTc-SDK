//! Native hybrid retrieval and reranking for VRTc.
//!
//! Combines VRTc lexical index results with optional semantic candidate lists
//! using weighted Reciprocal Rank Fusion, then applies a transparent term
//! coverage tie-breaker. Semantic candidates may come from a future local
//! vector engine; no hosted search API is required.

use std::collections::{BTreeMap, BTreeSet};

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_index::{IndexEngine, IndexHit};

#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    pub max_candidates_per_channel: usize,
    pub max_results: usize,
    pub rrf_constant: u32,
    pub lexical_weight: f32,
    pub semantic_weight: f32,
    pub term_match_weight: f32,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            max_candidates_per_channel: 100,
            max_results: 100,
            rrf_constant: 60,
            lexical_weight: 0.5,
            semantic_weight: 0.5,
            term_match_weight: 0.002,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticCandidate {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub excerpt: String,
    pub source: String,
    /// Semantic similarity normalized to [0, 1]; used for validation/traceability.
    pub similarity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetrievalChannel {
    Lexical,
    Semantic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalResult {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub excerpt: String,
    pub source: String,
    pub fusion_score: f32,
    pub term_coverage: f32,
    pub rank: usize,
    pub channels: Vec<RetrievalChannel>,
    pub lexical_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
    pub lexical_score: Option<f32>,
    pub semantic_similarity: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrievalStats {
    pub lexical_candidates: usize,
    pub semantic_candidates: usize,
    pub fused_candidates: usize,
    pub returned: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetrievalResponse {
    pub query: String,
    pub results: Vec<RetrievalResult>,
    pub stats: RetrievalStats,
}

#[derive(Default)]
struct CandidateAggregate {
    id: String,
    url: String,
    title: String,
    description: String,
    excerpt: String,
    source: String,
    score: f32,
    term_coverage: f32,
    channels: BTreeSet<RetrievalChannel>,
    lexical_rank: Option<usize>,
    semantic_rank: Option<usize>,
    lexical_score: Option<f32>,
    semantic_similarity: Option<f32>,
}

pub struct RetrievalEngine {
    index: std::sync::Arc<IndexEngine>,
    config: RetrievalConfig,
}

impl RetrievalEngine {
    pub fn new(
        index: std::sync::Arc<IndexEngine>,
        mut config: RetrievalConfig,
    ) -> Result<Self, VrtCError> {
        if config.max_candidates_per_channel == 0
            || config.max_results == 0
            || config.rrf_constant == 0
            || !valid_weight(config.lexical_weight)
            || !valid_weight(config.semantic_weight)
            || !valid_weight(config.term_match_weight)
            || config.lexical_weight + config.semantic_weight <= 0.0
        {
            return Err(retrieval_error(
                "INVALID_RETRIEVAL_CONFIG",
                "Retrieval limits, RRF constant, and weights must be valid",
                ErrorCategory::Validation,
            ));
        }
        let weight_total = config.lexical_weight + config.semantic_weight;
        config.lexical_weight /= weight_total;
        config.semantic_weight /= weight_total;
        Ok(Self { index, config })
    }

    /// Fuse local lexical results with optional semantic candidates.
    pub fn retrieve(
        &self,
        query: &str,
        semantic_candidates: &[SemanticCandidate],
        limit: usize,
    ) -> Result<RetrievalResponse, VrtCError> {
        if limit == 0 || limit > self.config.max_results {
            return Err(retrieval_error(
                "INVALID_RETRIEVAL_LIMIT",
                "Requested result limit is outside the configured range",
                ErrorCategory::Validation,
            ));
        }
        if semantic_candidates.len() > self.config.max_candidates_per_channel {
            return Err(retrieval_error(
                "TOO_MANY_SEMANTIC_CANDIDATES",
                "Semantic candidate list exceeds the configured maximum",
                ErrorCategory::ResourceExhausted,
            ));
        }
        if semantic_candidates
            .iter()
            .any(|candidate| !candidate.similarity.is_finite() || !(0.0..=1.0).contains(&candidate.similarity))
        {
            return Err(retrieval_error(
                "INVALID_SEMANTIC_SCORE",
                "Semantic similarity values must be finite and normalized to [0, 1]",
                ErrorCategory::Validation,
            ));
        }

        let lexical = self.index.search(query, self.config.max_candidates_per_channel)?;
        let lexical_count = lexical.len();
        let semantic_count = semantic_candidates.len();
        let mut candidates = BTreeMap::<String, CandidateAggregate>::new();

        for (offset, hit) in lexical.into_iter().enumerate() {
            let rank = offset + 1;
            let key = canonical_key(&hit.url, &hit.id);
            let aggregate = candidates.entry(key).or_default();
            merge_lexical(aggregate, hit, rank, &self.config);
        }

        for (offset, candidate) in semantic_candidates.iter().enumerate() {
            let rank = offset + 1;
            let key = canonical_key(&candidate.url, &candidate.id);
            let aggregate = candidates.entry(key).or_default();
            merge_semantic(aggregate, candidate, rank, &self.config);
        }

        let fused_count = candidates.len();
        let mut results = candidates
            .into_values()
            .map(|mut candidate| {
                candidate.term_coverage = term_coverage(
                    query,
                    &candidate.title,
                    &candidate.description,
                    &candidate.excerpt,
                );
                candidate.score += candidate.term_coverage * self.config.term_match_weight;
                RetrievalResult {
                    id: candidate.id,
                    url: candidate.url,
                    title: candidate.title,
                    description: candidate.description,
                    excerpt: candidate.excerpt,
                    source: candidate.source,
                    fusion_score: candidate.score,
                    term_coverage: candidate.term_coverage,
                    rank: 0,
                    channels: candidate.channels.into_iter().collect(),
                    lexical_rank: candidate.lexical_rank,
                    semantic_rank: candidate.semantic_rank,
                    lexical_score: candidate.lexical_score,
                    semantic_similarity: candidate.semantic_similarity,
                }
            })
            .collect::<Vec<_>>();

        results.sort_by(|left, right| {
            right
                .fusion_score
                .total_cmp(&left.fusion_score)
                .then_with(|| left.url.cmp(&right.url))
        });
        results.truncate(limit);
        for (index, result) in results.iter_mut().enumerate() {
            result.rank = index + 1;
        }

        let stats = RetrievalStats {
            lexical_candidates: lexical_count,
            semantic_candidates: semantic_count,
            fused_candidates: fused_count,
            returned: results.len(),
        };
        Ok(RetrievalResponse {
            query: query.to_owned(),
            results,
            stats,
        })
    }
}

fn merge_lexical(
    aggregate: &mut CandidateAggregate,
    hit: IndexHit,
    rank: usize,
    config: &RetrievalConfig,
) {
    if aggregate.id.is_empty() {
        aggregate.id = hit.id;
        aggregate.url = hit.url;
        aggregate.title = hit.title;
        aggregate.description = hit.description;
        aggregate.excerpt = hit.excerpt;
        aggregate.source = hit.source;
    }
    aggregate.score += config.lexical_weight / (config.rrf_constant as f32 + rank as f32);
    aggregate.lexical_score = Some(hit.score);
    aggregate.channels.insert(RetrievalChannel::Lexical);
    aggregate.lexical_rank = Some(rank);
}

fn merge_semantic(
    aggregate: &mut CandidateAggregate,
    candidate: &SemanticCandidate,
    rank: usize,
    config: &RetrievalConfig,
) {
    if aggregate.id.is_empty() {
        aggregate.id = candidate.id.clone();
        aggregate.url = candidate.url.clone();
        aggregate.title = candidate.title.clone();
        aggregate.description = candidate.description.clone();
        aggregate.excerpt = candidate.excerpt.clone();
        aggregate.source = candidate.source.clone();
    } else {
        if aggregate.title.is_empty() {
            aggregate.title = candidate.title.clone();
        }
        if aggregate.description.is_empty() {
            aggregate.description = candidate.description.clone();
        }
        if aggregate.excerpt.is_empty() {
            aggregate.excerpt = candidate.excerpt.clone();
        }
        if aggregate.source.is_empty() {
            aggregate.source = candidate.source.clone();
        }
    }
    aggregate.score += config.semantic_weight / (config.rrf_constant as f32 + rank as f32);
    aggregate.semantic_similarity = Some(candidate.similarity);
    aggregate.channels.insert(RetrievalChannel::Semantic);
    aggregate.semantic_rank = Some(rank);
}

fn canonical_key(url: &str, id: &str) -> String {
    Url::parse(url)
        .ok()
        .map(|mut parsed| {
            parsed.set_fragment(None);
            parsed.to_string()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("id:{id}"))
}

fn term_coverage(query: &str, title: &str, description: &str, excerpt: &str) -> f32 {
    let query_terms = query
        .to_ascii_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() > 1)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if query_terms.is_empty() {
        return 0.0;
    }
    let content_terms = format!("{title} {description} {excerpt}")
        .to_ascii_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.len() > 1)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let matched = query_terms.intersection(&content_terms).count();
    matched as f32 / query_terms.len() as f32
}

fn valid_weight(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn retrieval_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrtc_index::{IndexConfig, IndexDocument};

    fn indexed_document(id: &str, url: &str, title: &str, body: &str) -> IndexDocument {
        IndexDocument {
            id: id.to_owned(),
            url: url.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            body: body.to_owned(),
            language: "en".to_owned(),
            published_at: String::new(),
            source: "local-index".to_owned(),
        }
    }

    #[test]
    fn fuses_lexical_and_semantic_results_and_deduplicates_urls() {
        let index = std::sync::Arc::new(IndexEngine::new(IndexConfig::default()).unwrap());
        index
            .index_batch(&[
                indexed_document(
                    "lexical",
                    "https://example.org/lexical",
                    "Crawler frontier scheduling",
                    "URL frontier scheduling keeps a crawler polite.",
                ),
                indexed_document(
                    "shared",
                    "https://example.org/shared#old",
                    "Search system design",
                    "Search and retrieval systems use ranked documents.",
                ),
            ])
            .unwrap();
        let engine = RetrievalEngine::new(index, RetrievalConfig::default()).unwrap();
        let semantic = vec![
            SemanticCandidate {
                id: "semantic".to_owned(),
                url: "https://example.org/semantic".to_owned(),
                title: "Crawler scheduling".to_owned(),
                description: String::new(),
                excerpt: "A crawler frontier schedules URLs.".to_owned(),
                source: "semantic-index".to_owned(),
                similarity: 0.94,
            },
            SemanticCandidate {
                id: "shared-copy".to_owned(),
                url: "https://example.org/shared".to_owned(),
                title: "Search system design".to_owned(),
                description: String::new(),
                excerpt: "Hybrid retrieval ranks documents.".to_owned(),
                source: "semantic-index".to_owned(),
                similarity: 0.88,
            },
        ];
        let response = engine.retrieve("crawler scheduling", &semantic, 10).unwrap();
        assert_eq!(response.stats.semantic_candidates, 2);
        assert!(response.results.iter().any(|result| result.url == "https://example.org/shared#old"));
        assert_eq!(
            response
                .results
                .iter()
                .filter(|result| result.url.contains("/shared"))
                .count(),
            1
        );
        assert_eq!(response.results[0].rank, 1);
        assert_eq!(response.results[0].url, "https://example.org/shared#old");
    }

    #[test]
    fn rejects_out_of_range_semantic_scores() {
        let index = std::sync::Arc::new(IndexEngine::new(IndexConfig::default()).unwrap());
        let engine = RetrievalEngine::new(index, RetrievalConfig::default()).unwrap();
        let candidate = SemanticCandidate {
            id: "bad".to_owned(),
            url: "https://example.org/bad".to_owned(),
            title: "Bad score".to_owned(),
            description: String::new(),
            excerpt: String::new(),
            source: "test".to_owned(),
            similarity: 1.5,
        };
        assert!(engine.retrieve("query", &[candidate], 5).is_err());
    }
}
