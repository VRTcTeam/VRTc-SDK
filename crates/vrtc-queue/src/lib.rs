//! Native VRTc job queue with bounded jobs, worker leases, retries, and recovery.
//!
//! The queue is single-runtime coordination. An optional Storage Engine handle
//! persists job records, but the current lease coordination is not distributed
//! across independent processes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_storage::{StorageEngine, StorageOperation};

const STORAGE_NAMESPACE: &str = "queue";

#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub max_jobs: usize,
    pub max_payload_bytes: usize,
    pub max_job_id_bytes: usize,
    pub max_job_type_bytes: usize,
    pub max_worker_id_bytes: usize,
    pub default_max_attempts: u32,
    pub max_lease_duration_ms: u64,
    pub max_retry_delay_ms: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_jobs: 1_000,
            max_payload_bytes: 1024 * 1024,
            max_job_id_bytes: 256,
            max_job_type_bytes: 128,
            max_worker_id_bytes: 128,
            default_max_attempts: 3,
            max_lease_duration_ms: 15 * 60 * 1000,
            max_retry_delay_ms: 60 * 60 * 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Leased,
    Completed,
    DeadLetter,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub job_type: String,
    pub payload: Vec<u8>,
    pub priority: i32,
    pub status: JobStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub created_at_ms: u64,
    pub not_before_ms: u64,
    pub leased_by: Option<String>,
    pub lease_until_ms: Option<u64>,
    pub lease_generation: u64,
    pub completed_at_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSubmission {
    pub id: String,
    pub job_type: String,
    pub payload: Vec<u8>,
    pub priority: i32,
    pub max_attempts: Option<u32>,
    pub created_at_ms: u64,
    pub not_before_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLease {
    pub job: JobRecord,
    pub worker_id: String,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    Requeued,
    DeadLetter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    pub pending: usize,
    pub leased: usize,
    pub completed: usize,
    pub dead_letter: usize,
    pub cancelled: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecoveryResult {
    pub requeued: usize,
    pub dead_lettered: usize,
}

#[derive(Default)]
struct QueueState {
    jobs: BTreeMap<String, JobRecord>,
}

pub struct JobQueueEngine {
    config: QueueConfig,
    state: Mutex<QueueState>,
    storage: Option<Arc<StorageEngine>>,
}

impl JobQueueEngine {
    pub fn new(
        config: QueueConfig,
        storage: Option<Arc<StorageEngine>>,
    ) -> Result<Self, VrtCError> {
        validate_config(&config)?;
        let mut state = QueueState::default();
        if let Some(storage) = &storage {
            for entry in storage.scan_namespace(STORAGE_NAMESPACE, config.max_jobs)? {
                let record: JobRecord = serde_json::from_slice(&entry.value).map_err(|_| {
                    queue_error(
                        "QUEUE_PERSISTED_RECORD_INVALID",
                        "A persisted queue record could not be decoded",
                        ErrorCategory::Internal,
                        false,
                    )
                })?;
                if entry.key != record.id || state.jobs.contains_key(&record.id) {
                    return Err(queue_error(
                        "QUEUE_PERSISTED_ID_MISMATCH",
                        "A persisted queue key does not match its job ID",
                        ErrorCategory::Internal,
                        false,
                    ));
                }
                state.jobs.insert(record.id.clone(), record);
            }
        }
        Ok(Self {
            config,
            state: Mutex::new(state),
            storage,
        })
    }

    pub fn submit(&self, submission: JobSubmission) -> Result<JobRecord, VrtCError> {
        if submission.id.trim().is_empty()
            || submission.id.len() > self.config.max_job_id_bytes
            || submission.id.chars().any(char::is_control)
            || submission.job_type.trim().is_empty()
            || submission.job_type.chars().any(char::is_control)
            || submission.job_type.len() > self.config.max_job_type_bytes
            || submission.payload.len() > self.config.max_payload_bytes
            || submission.max_attempts == Some(0)
        {
            return Err(queue_error(
                "INVALID_JOB_SUBMISSION",
                "Job ID, type, payload, and attempt limits must be valid",
                ErrorCategory::Validation,
                false,
            ));
        }
        let mut state = self.lock_state()?;
        if state.jobs.contains_key(&submission.id) {
            return Err(queue_error(
                "DUPLICATE_JOB_ID",
                "A job with this identifier already exists",
                ErrorCategory::DuplicateId,
                false,
            ));
        }
        if state.jobs.len() >= self.config.max_jobs {
            return Err(queue_error(
                "QUEUE_CAPACITY_REACHED",
                "The job queue has reached its configured capacity",
                ErrorCategory::ResourceExhausted,
                false,
            ));
        }
        let record = JobRecord {
            id: submission.id,
            job_type: submission.job_type,
            payload: submission.payload,
            priority: submission.priority,
            status: JobStatus::Pending,
            attempt_count: 0,
            max_attempts: submission
                .max_attempts
                .unwrap_or(self.config.default_max_attempts),
            created_at_ms: submission.created_at_ms,
            not_before_ms: submission.not_before_ms,
            leased_by: None,
            lease_until_ms: None,
            lease_generation: 0,
            completed_at_ms: None,
            last_error: None,
        };
        self.persist(&record)?;
        state.jobs.insert(record.id.clone(), record.clone());
        Ok(record)
    }

    /// Lease the highest-priority eligible job with a generation-bound token.
    pub fn lease_next(
        &self,
        worker_id: &str,
        now_ms: u64,
        lease_duration_ms: u64,
    ) -> Result<Option<JobLease>, VrtCError> {
        self.validate_worker(worker_id)?;
        if lease_duration_ms == 0 || lease_duration_ms > self.config.max_lease_duration_ms {
            return Err(queue_error(
                "INVALID_QUEUE_LEASE_DURATION",
                "Lease duration is outside the configured range",
                ErrorCategory::Validation,
                false,
            ));
        }
        let lease_until_ms = now_ms.checked_add(lease_duration_ms).ok_or_else(|| {
            queue_error(
                "QUEUE_TIMESTAMP_OVERFLOW",
                "Lease expiration timestamp overflowed",
                ErrorCategory::Validation,
                false,
            )
        })?;
        let mut state = self.lock_state()?;
        let selected = state
            .jobs
            .values()
            .filter(|job| job.status == JobStatus::Pending && job.not_before_ms <= now_ms)
            .min_by(|left, right| {
                right
                    .priority
                    .cmp(&left.priority)
                    .then(left.created_at_ms.cmp(&right.created_at_ms))
                    .then(left.id.cmp(&right.id))
            })
            .cloned();
        let Some(mut job) = selected else {
            return Ok(None);
        };
        job.status = JobStatus::Leased;
        job.leased_by = Some(worker_id.to_owned());
        job.lease_until_ms = Some(lease_until_ms);
        job.lease_generation = job.lease_generation.checked_add(1).ok_or_else(|| {
            queue_error(
                "QUEUE_LEASE_GENERATION_OVERFLOW",
                "The lease generation counter overflowed",
                ErrorCategory::Internal,
                false,
            )
        })?;
        self.persist(&job)?;
        state.jobs.insert(job.id.clone(), job.clone());
        Ok(Some(JobLease {
            generation: job.lease_generation,
            job,
            worker_id: worker_id.to_owned(),
        }))
    }

    pub fn ack(
        &self,
        job_id: &str,
        worker_id: &str,
        generation: u64,
        now_ms: u64,
    ) -> Result<JobRecord, VrtCError> {
        let mut state = self.lock_state()?;
        let mut job = state
            .jobs
            .get(job_id)
            .cloned()
            .ok_or_else(|| job_not_found(job_id))?;
        validate_lease(&job, worker_id, generation, now_ms)?;
        job.status = JobStatus::Completed;
        job.completed_at_ms = Some(now_ms);
        job.leased_by = None;
        job.lease_until_ms = None;
        self.persist(&job)?;
        state.jobs.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    pub fn retry(
        &self,
        job_id: &str,
        worker_id: &str,
        generation: u64,
        now_ms: u64,
        delay_ms: u64,
        error_message: Option<String>,
    ) -> Result<(RetryOutcome, JobRecord), VrtCError> {
        if delay_ms > self.config.max_retry_delay_ms {
            return Err(queue_error(
                "INVALID_QUEUE_RETRY_DELAY",
                "Retry delay exceeds the configured maximum",
                ErrorCategory::Validation,
                false,
            ));
        }
        let mut state = self.lock_state()?;
        let mut job = state
            .jobs
            .get(job_id)
            .cloned()
            .ok_or_else(|| job_not_found(job_id))?;
        validate_lease(&job, worker_id, generation, now_ms)?;
        job.attempt_count = job.attempt_count.saturating_add(1);
        job.last_error =
            error_message.map(|message| message.chars().take(1024).collect::<String>());
        job.leased_by = None;
        job.lease_until_ms = None;
        let outcome = if job.attempt_count >= job.max_attempts {
            job.status = JobStatus::DeadLetter;
            RetryOutcome::DeadLetter
        } else {
            job.status = JobStatus::Pending;
            job.not_before_ms = now_ms.checked_add(delay_ms).ok_or_else(|| {
                queue_error(
                    "QUEUE_TIMESTAMP_OVERFLOW",
                    "Retry timestamp overflowed",
                    ErrorCategory::Validation,
                    false,
                )
            })?;
            RetryOutcome::Requeued
        };
        self.persist(&job)?;
        state.jobs.insert(job.id.clone(), job.clone());
        Ok((outcome, job))
    }

    /// Recover expired leases after a worker crash or runtime restart.
    pub fn recover_expired(&self, now_ms: u64) -> Result<RecoveryResult, VrtCError> {
        let mut state = self.lock_state()?;
        let mut updated = Vec::new();
        let mut result = RecoveryResult::default();
        for job in state.jobs.values() {
            if job.status != JobStatus::Leased
                || job.lease_until_ms.is_none_or(|expires| expires > now_ms)
            {
                continue;
            }
            let mut recovered = job.clone();
            recovered.attempt_count = recovered.attempt_count.saturating_add(1);
            recovered.leased_by = None;
            recovered.lease_until_ms = None;
            if recovered.attempt_count >= recovered.max_attempts {
                recovered.status = JobStatus::DeadLetter;
                result.dead_lettered += 1;
            } else {
                recovered.status = JobStatus::Pending;
                recovered.not_before_ms = now_ms;
                result.requeued += 1;
            }
            updated.push(recovered);
        }
        self.persist_batch(&updated)?;
        for job in updated {
            state.jobs.insert(job.id.clone(), job);
        }
        Ok(result)
    }

    pub fn cancel(&self, job_id: &str) -> Result<JobRecord, VrtCError> {
        let mut state = self.lock_state()?;
        let mut job = state
            .jobs
            .get(job_id)
            .cloned()
            .ok_or_else(|| job_not_found(job_id))?;
        if matches!(
            job.status,
            JobStatus::Completed | JobStatus::DeadLetter | JobStatus::Cancelled
        ) {
            return Err(queue_error(
                "JOB_NOT_CANCELLABLE",
                "Completed, dead-lettered, or cancelled jobs cannot be cancelled again",
                ErrorCategory::Validation,
                false,
            ));
        }
        job.status = JobStatus::Cancelled;
        job.leased_by = None;
        job.lease_until_ms = None;
        self.persist(&job)?;
        state.jobs.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    pub fn get(&self, job_id: &str) -> Result<Option<JobRecord>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state.jobs.get(job_id).cloned())
    }

    pub fn dead_letters(&self, limit: usize) -> Result<Vec<JobRecord>, VrtCError> {
        if limit == 0 || limit > self.config.max_jobs {
            return Err(queue_error(
                "INVALID_DEAD_LETTER_LIMIT",
                "Dead-letter result limit is outside the configured range",
                ErrorCategory::Validation,
                false,
            ));
        }
        let state = self.lock_state()?;
        Ok(state
            .jobs
            .values()
            .filter(|job| job.status == JobStatus::DeadLetter)
            .take(limit)
            .cloned()
            .collect())
    }

    pub fn stats(&self) -> Result<QueueStats, VrtCError> {
        let state = self.lock_state()?;
        let mut stats = QueueStats {
            total: state.jobs.len(),
            ..QueueStats::default()
        };
        for job in state.jobs.values() {
            match job.status {
                JobStatus::Pending => stats.pending += 1,
                JobStatus::Leased => stats.leased += 1,
                JobStatus::Completed => stats.completed += 1,
                JobStatus::DeadLetter => stats.dead_letter += 1,
                JobStatus::Cancelled => stats.cancelled += 1,
            }
        }
        Ok(stats)
    }

    fn validate_worker(&self, worker_id: &str) -> Result<(), VrtCError> {
        if worker_id.trim().is_empty()
            || worker_id.len() > self.config.max_worker_id_bytes
            || worker_id.chars().any(char::is_control)
        {
            return Err(queue_error(
                "INVALID_WORKER_ID",
                "Worker ID must be non-empty and within the configured limit",
                ErrorCategory::Validation,
                false,
            ));
        }
        Ok(())
    }

    fn persist(&self, job: &JobRecord) -> Result<(), VrtCError> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let value = serde_json::to_vec(job).map_err(|_| {
            queue_error(
                "QUEUE_SERIALIZATION_FAILED",
                "The job record could not be serialized",
                ErrorCategory::Internal,
                false,
            )
        })?;
        storage.put(STORAGE_NAMESPACE, &job.id, value)
    }

    fn persist_batch(&self, jobs: &[JobRecord]) -> Result<(), VrtCError> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let mut operations = Vec::with_capacity(jobs.len());
        for job in jobs {
            let value = serde_json::to_vec(job).map_err(|_| {
                queue_error(
                    "QUEUE_SERIALIZATION_FAILED",
                    "A recovered job record could not be serialized",
                    ErrorCategory::Internal,
                    false,
                )
            })?;
            operations.push(StorageOperation::Put {
                namespace: STORAGE_NAMESPACE.to_owned(),
                key: job.id.clone(),
                value,
            });
        }
        storage.apply_batch(operations)?;
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, QueueState>, VrtCError> {
        self.state.lock().map_err(|_| {
            queue_error(
                "QUEUE_LOCK_POISONED",
                "The job queue state is temporarily unavailable",
                ErrorCategory::Internal,
                true,
            )
        })
    }
}

fn validate_config(config: &QueueConfig) -> Result<(), VrtCError> {
    if config.max_jobs == 0
        || config.max_payload_bytes == 0
        || config.max_job_id_bytes == 0
        || config.max_job_type_bytes == 0
        || config.max_worker_id_bytes == 0
        || config.default_max_attempts == 0
        || config.max_lease_duration_ms == 0
        || config.max_retry_delay_ms == 0
    {
        return Err(queue_error(
            "INVALID_QUEUE_CONFIG",
            "Queue capacity, payload, identity, attempts, and timing limits must be positive",
            ErrorCategory::Validation,
            false,
        ));
    }
    Ok(())
}

fn validate_lease(
    job: &JobRecord,
    worker_id: &str,
    generation: u64,
    now_ms: u64,
) -> Result<(), VrtCError> {
    if job.status != JobStatus::Leased
        || job.leased_by.as_deref() != Some(worker_id)
        || job.lease_generation != generation
        || job.lease_until_ms.is_none_or(|expires| expires <= now_ms)
    {
        return Err(queue_error(
            "STALE_OR_INVALID_JOB_LEASE",
            "The worker does not own the current lease for this job",
            ErrorCategory::Validation,
            false,
        ));
    }
    Ok(())
}

fn job_not_found(job_id: &str) -> VrtCError {
    queue_error(
        "JOB_NOT_FOUND",
        &format!("Job {job_id} does not exist"),
        ErrorCategory::NotFound,
        false,
    )
}

fn queue_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vrtc_storage::{StorageConfig, StorageEngine};

    fn submission(id: &str, priority: i32) -> JobSubmission {
        JobSubmission {
            id: id.to_owned(),
            job_type: "crawl.page".to_owned(),
            payload: b"https://example.org".to_vec(),
            priority,
            max_attempts: Some(2),
            created_at_ms: 1,
            not_before_ms: 1,
        }
    }

    #[test]
    fn leases_by_priority_and_requires_current_worker_token() {
        let queue = JobQueueEngine::new(QueueConfig::default(), None).unwrap();
        queue.submit(submission("low", 1)).unwrap();
        queue.submit(submission("high", 10)).unwrap();
        let lease = queue.lease_next("worker-a", 10, 100).unwrap().unwrap();
        assert_eq!(lease.job.id, "high");
        assert!(queue.ack("high", "worker-b", lease.generation, 20).is_err());
        let completed = queue.ack("high", "worker-a", lease.generation, 20).unwrap();
        assert_eq!(completed.status, JobStatus::Completed);
    }

    #[test]
    fn retries_then_moves_exhausted_jobs_to_dead_letters() {
        let queue = JobQueueEngine::new(QueueConfig::default(), None).unwrap();
        queue.submit(submission("job-1", 1)).unwrap();
        let first = queue.lease_next("worker", 10, 100).unwrap().unwrap();
        let (outcome, _) = queue
            .retry(
                "job-1",
                "worker",
                first.generation,
                20,
                0,
                Some("temporary".to_owned()),
            )
            .unwrap();
        assert_eq!(outcome, RetryOutcome::Requeued);
        let second = queue.lease_next("worker", 20, 100).unwrap().unwrap();
        let (outcome, record) = queue
            .retry(
                "job-1",
                "worker",
                second.generation,
                30,
                0,
                Some("again".to_owned()),
            )
            .unwrap();
        assert_eq!(outcome, RetryOutcome::DeadLetter);
        assert_eq!(record.status, JobStatus::DeadLetter);
        assert_eq!(queue.dead_letters(10).unwrap().len(), 1);
    }

    #[test]
    fn expired_worker_cannot_ack_a_job_before_recovery() {
        let queue = JobQueueEngine::new(QueueConfig::default(), None).unwrap();
        queue.submit(submission("expired-job", 1)).unwrap();
        let lease = queue.lease_next("worker", 10, 10).unwrap().unwrap();
        assert!(queue
            .ack("expired-job", "worker", lease.generation, 20)
            .is_err());
        assert_eq!(queue.recover_expired(20).unwrap().requeued, 1);
    }

    #[test]
    fn recovers_expired_leases_and_persists_job_lifecycle() {
        let storage = Arc::new(StorageEngine::memory(StorageConfig::default()).unwrap());
        let queue =
            JobQueueEngine::new(QueueConfig::default(), Some(Arc::clone(&storage))).unwrap();
        queue.submit(submission("persisted-job", 5)).unwrap();
        queue.lease_next("worker", 10, 10).unwrap().unwrap();
        let recovered = queue.recover_expired(25).unwrap();
        assert_eq!(recovered.requeued, 1);
        drop(queue);

        let reopened = JobQueueEngine::new(QueueConfig::default(), Some(storage)).unwrap();
        assert_eq!(
            reopened.get("persisted-job").unwrap().unwrap().status,
            JobStatus::Pending
        );
    }
}
