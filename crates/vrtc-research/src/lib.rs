//! Native, deterministic research planning for VRTc.
//!
//! This crate creates bounded dependency graphs. It does not execute network
//! requests or claim durable workflow execution; execution is delegated to the
//! runtime's queue and the corresponding engine capabilities.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct ResearchConfig {
    pub max_query_bytes: usize,
    pub max_sources: usize,
    pub max_crawl_depth: u8,
    pub max_steps: usize,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            max_query_bytes: 8 * 1024,
            max_sources: 100,
            max_crawl_depth: 5,
            max_steps: 16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchRequest {
    pub id: String,
    pub query: String,
    pub max_sources: usize,
    pub crawl: bool,
    pub max_crawl_depth: u8,
    pub human_review: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchStepKind {
    Search,
    Crawl,
    Extract,
    Index,
    Retrieve,
    VerifyEvidence,
    HumanReview,
    Synthesize,
}

impl ResearchStepKind {
    fn capability(self) -> &'static str {
        match self {
            Self::Search => "search.query",
            Self::Crawl => "crawl.execute",
            Self::Extract => "content.extract",
            Self::Index => "index.documents",
            Self::Retrieve => "retrieval.fuse",
            Self::VerifyEvidence => "evidence.verify",
            Self::HumanReview => "human.review",
            Self::Synthesize => "research.synthesize",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchStep {
    pub id: String,
    pub kind: ResearchStepKind,
    pub capability: String,
    pub depends_on: Vec<String>,
    /// Steps sharing a non-zero group may be scheduled concurrently.
    pub parallel_group: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchPlan {
    pub request_id: String,
    pub query: String,
    pub max_sources: usize,
    pub max_crawl_depth: Option<u8>,
    pub steps: Vec<ResearchStep>,
    pub execution_order: Vec<String>,
    pub parallel_groups: BTreeMap<u8, Vec<String>>,
}

#[derive(Debug)]
pub struct ResearchPlanner {
    config: ResearchConfig,
}

impl ResearchPlanner {
    pub fn new(config: ResearchConfig) -> Result<Self, VrtCError> {
        if config.max_query_bytes == 0
            || config.max_sources == 0
            || config.max_crawl_depth == 0
            || config.max_steps < 3
        {
            return Err(planner_error(
                "INVALID_RESEARCH_CONFIG",
                "Research limits must be positive and max_steps must be at least three",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self { config })
    }

    pub fn plan(&self, request: ResearchRequest) -> Result<ResearchPlan, VrtCError> {
        if request.id.trim().is_empty()
            || request.id.len() > 256
            || request.id.chars().any(char::is_control)
            || request.query.trim().is_empty()
            || request.query.len() > self.config.max_query_bytes
            || request.max_sources == 0
            || request.max_sources > self.config.max_sources
            || (request.crawl
                && (request.max_crawl_depth == 0
                    || request.max_crawl_depth > self.config.max_crawl_depth))
        {
            return Err(planner_error(
                "INVALID_RESEARCH_REQUEST",
                "Research ID, query, source limit, or crawl depth is invalid",
                ErrorCategory::Validation,
            ));
        }

        let mut steps = vec![step("search", ResearchStepKind::Search, vec![], 1)];
        if request.crawl {
            steps.push(step("crawl", ResearchStepKind::Crawl, vec![], 1));
            steps.push(step("extract", ResearchStepKind::Extract, vec!["crawl"], 2));
            steps.push(step("index", ResearchStepKind::Index, vec!["extract"], 3));
        }
        let retrieval_dependencies = if request.crawl {
            vec!["search", "index"]
        } else {
            vec!["search"]
        };
        steps.push(step(
            "retrieve",
            ResearchStepKind::Retrieve,
            retrieval_dependencies,
            4,
        ));
        steps.push(step(
            "verify",
            ResearchStepKind::VerifyEvidence,
            vec!["retrieve"],
            5,
        ));
        if request.human_review {
            steps.push(step(
                "human-review",
                ResearchStepKind::HumanReview,
                vec!["verify"],
                6,
            ));
        }
        let synthesis_dependency = if request.human_review {
            "human-review"
        } else {
            "verify"
        };
        steps.push(step(
            "synthesize",
            ResearchStepKind::Synthesize,
            vec![synthesis_dependency],
            7,
        ));

        if steps.len() > self.config.max_steps {
            return Err(planner_error(
                "RESEARCH_PLAN_TOO_LARGE",
                "The generated plan exceeds the configured step limit",
                ErrorCategory::ResourceExhausted,
            ));
        }

        let execution_order = topological_order(&steps)?;
        let mut parallel_groups: BTreeMap<u8, Vec<String>> = BTreeMap::new();
        for item in &steps {
            if item.parallel_group > 0 {
                parallel_groups
                    .entry(item.parallel_group)
                    .or_default()
                    .push(item.id.clone());
            }
        }

        Ok(ResearchPlan {
            request_id: request.id,
            query: request.query.trim().to_owned(),
            max_sources: request.max_sources,
            max_crawl_depth: request.crawl.then_some(request.max_crawl_depth),
            steps,
            execution_order,
            parallel_groups,
        })
    }
}

fn step(
    id: &str,
    kind: ResearchStepKind,
    dependencies: Vec<&str>,
    parallel_group: u8,
) -> ResearchStep {
    ResearchStep {
        id: id.to_owned(),
        kind,
        capability: kind.capability().to_owned(),
        depends_on: dependencies.into_iter().map(str::to_owned).collect(),
        parallel_group,
    }
}

fn topological_order(steps: &[ResearchStep]) -> Result<Vec<String>, VrtCError> {
    let ids: BTreeSet<&str> = steps.iter().map(|item| item.id.as_str()).collect();
    if ids.len() != steps.len()
        || steps.iter().any(|item| {
            item.depends_on
                .iter()
                .any(|dependency| !ids.contains(dependency.as_str()))
        })
    {
        return Err(planner_error(
            "INVALID_RESEARCH_GRAPH",
            "Research plan contains duplicate IDs or missing dependencies",
            ErrorCategory::Validation,
        ));
    }

    let mut indegree: BTreeMap<&str, usize> = steps
        .iter()
        .map(|item| (item.id.as_str(), item.depends_on.len()))
        .collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for item in steps {
        for dependency in &item.depends_on {
            dependents
                .entry(dependency.as_str())
                .or_default()
                .push(item.id.as_str());
        }
    }

    let mut ready: VecDeque<&str> = steps
        .iter()
        .filter(|item| indegree[item.id.as_str()] == 0)
        .map(|item| item.id.as_str())
        .collect();
    let mut order = Vec::with_capacity(steps.len());
    while let Some(id) = ready.pop_front() {
        order.push(id.to_owned());
        if let Some(children) = dependents.get(id) {
            for child in children {
                let degree = indegree.get_mut(*child).expect("validated node");
                *degree -= 1;
                if *degree == 0 {
                    ready.push_back(*child);
                }
            }
        }
    }
    if order.len() != steps.len() {
        return Err(planner_error(
            "RESEARCH_GRAPH_CYCLE",
            "Research plan dependencies contain a cycle",
            ErrorCategory::Validation,
        ));
    }
    Ok(order)
}

fn planner_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false).for_component("engine.vrtc.research")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(crawl: bool, human_review: bool) -> ResearchRequest {
        ResearchRequest {
            id: "research-1".to_owned(),
            query: "WCAG 2.2 accessibility".to_owned(),
            max_sources: 20,
            crawl,
            max_crawl_depth: 2,
            human_review,
        }
    }

    #[test]
    fn plans_search_retrieval_verification_and_synthesis() {
        let planner = ResearchPlanner::new(ResearchConfig::default()).unwrap();
        let plan = planner.plan(request(false, false)).unwrap();
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(
            plan.execution_order,
            vec!["search", "retrieve", "verify", "synthesize"]
        );
    }

    #[test]
    fn crawl_and_search_are_independent_parallel_branches() {
        let planner = ResearchPlanner::new(ResearchConfig::default()).unwrap();
        let plan = planner.plan(request(true, true)).unwrap();
        assert_eq!(plan.parallel_groups.get(&1).unwrap().len(), 2);
        assert!(plan.execution_order.iter().position(|id| id == "search").unwrap()
            < plan.execution_order.iter().position(|id| id == "retrieve").unwrap());
        assert!(plan.steps.iter().any(|item| item.id == "human-review"));
    }

    #[test]
    fn rejects_invalid_source_and_depth_limits() {
        let planner = ResearchPlanner::new(ResearchConfig::default()).unwrap();
        let mut invalid = request(true, false);
        invalid.max_sources = 0;
        assert_eq!(
            planner.plan(invalid).unwrap_err().code,
            "INVALID_RESEARCH_REQUEST"
        );
    }

    #[test]
    fn rejects_zero_configuration_limits() {
        let config = ResearchConfig {
            max_steps: 0,
            ..ResearchConfig::default()
        };
        assert_eq!(
            ResearchPlanner::new(config).unwrap_err().code,
            "INVALID_RESEARCH_CONFIG"
        );
    }
}
