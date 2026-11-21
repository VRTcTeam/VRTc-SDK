//! Bounded native WARC-compatible response archive for VRTc.
//!
//! The initial engine keeps records in process memory and exports standards-shaped
//! WARC/1.1 response records. It never performs network requests itself.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct ArchiveConfig {
    pub max_records: usize,
    pub max_total_bytes: usize,
    pub max_record_bytes: usize,
}

impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            max_records: 1_000,
            max_total_bytes: 128 * 1024 * 1024,
            max_record_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCapture {
    /// UUID value without angle brackets or the urn prefix.
    pub record_id: String,
    pub target_uri: String,
    /// Canonical UTC WARC-Date: YYYY-MM-DDTHH:MM:SSZ.
    pub captured_at: String,
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub record_id: String,
    pub target_uri: String,
    pub captured_at: String,
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    serialized_bytes: usize,
}

impl ArchiveRecord {
    /// Serialize this capture as a WARC/1.1 response record.
    #[must_use]
    pub fn to_warc_bytes(&self) -> Vec<u8> {
        let mut http = Vec::new();
        let reason = reason_phrase(self.status);
        http.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", self.status, reason).as_bytes());
        http.extend_from_slice(format!("Content-Type: {}\r\n", self.content_type).as_bytes());
        http.extend_from_slice(format!("Content-Length: {}\r\n", self.body.len()).as_bytes());
        for (name, value) in &self.headers {
            http.extend_from_slice(name.as_bytes());
            http.extend_from_slice(b": ");
            http.extend_from_slice(value.as_bytes());
            http.extend_from_slice(b"\r\n");
        }
        http.extend_from_slice(b"\r\n");
        http.extend_from_slice(&self.body);

        let mut warc = Vec::new();
        warc.extend_from_slice(b"WARC/1.1\r\n");
        warc.extend_from_slice(b"WARC-Type: response\r\n");
        warc.extend_from_slice(format!("WARC-Record-ID: <urn:uuid:{}>\r\n", self.record_id).as_bytes());
        warc.extend_from_slice(format!("WARC-Date: {}\r\n", self.captured_at).as_bytes());
        warc.extend_from_slice(format!("WARC-Target-URI: {}\r\n", self.target_uri).as_bytes());
        warc.extend_from_slice(b"Content-Type: application/http; msgtype=response\r\n");
        warc.extend_from_slice(format!("Content-Length: {}\r\n", http.len()).as_bytes());
        warc.extend_from_slice(b"\r\n");
        warc.extend_from_slice(&http);
        warc.extend_from_slice(b"\r\n\r\n");
        warc
    }

    #[must_use]
    pub fn serialized_len(&self) -> usize {
        self.serialized_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveStats {
    pub records_retained: usize,
    pub bytes_retained: usize,
    pub total_records_captured: u64,
    pub records_evicted: u64,
}

#[derive(Debug, Default)]
struct ArchiveState {
    records: VecDeque<ArchiveRecord>,
    bytes_retained: usize,
    total_records_captured: u64,
    records_evicted: u64,
}

pub struct ArchiveEngine {
    config: ArchiveConfig,
    state: Mutex<ArchiveState>,
}

impl ArchiveEngine {
    pub fn new(config: ArchiveConfig) -> Result<Self, VrtCError> {
        if config.max_records == 0
            || config.max_total_bytes == 0
            || config.max_record_bytes == 0
            || config.max_record_bytes > config.max_total_bytes
        {
            return Err(archive_error(
                "INVALID_ARCHIVE_CONFIG",
                "Archive capacities must be positive and record capacity cannot exceed total capacity",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(ArchiveState::default()),
        })
    }

    pub fn capture(&self, capture: ArchiveCapture) -> Result<ArchiveRecord, VrtCError> {
        if capture.body.len() > self.config.max_record_bytes {
            return Err(archive_error(
                "ARCHIVE_RECORD_TOO_LARGE",
                "Response body exceeds the configured record byte limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let mut record = normalize_capture(capture)?;
        let serialized_bytes = record.to_warc_bytes().len();
        if serialized_bytes > self.config.max_record_bytes
            || serialized_bytes > self.config.max_total_bytes
        {
            return Err(archive_error(
                "ARCHIVE_RECORD_TOO_LARGE",
                "Serialized WARC record exceeds the configured record or total byte limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        record.serialized_bytes = serialized_bytes;

        let mut state = self.lock_state()?;
        if state.records.iter().any(|existing| existing.record_id == record.record_id) {
            return Err(archive_error(
                "DUPLICATE_ARCHIVE_RECORD_ID",
                "WARC record IDs must be unique within the retained archive",
                ErrorCategory::Validation,
            ));
        }
        while state.records.len() >= self.config.max_records
            || state.bytes_retained.saturating_add(serialized_bytes) > self.config.max_total_bytes
        {
            if let Some(evicted) = state.records.pop_front() {
                state.bytes_retained = state.bytes_retained.saturating_sub(evicted.serialized_bytes);
                state.records_evicted = state.records_evicted.saturating_add(1);
            } else {
                return Err(archive_error(
                    "ARCHIVE_CAPACITY_UNAVAILABLE",
                    "Archive capacity is unavailable for this record",
                    ErrorCategory::ResourceExhausted,
                ));
            }
        }
        state.bytes_retained = state.bytes_retained.saturating_add(serialized_bytes);
        state.total_records_captured = state.total_records_captured.saturating_add(1);
        state.records.push_back(record.clone());
        Ok(record)
    }

    pub fn get(&self, record_id: &str) -> Result<Option<ArchiveRecord>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state.records.iter().find(|record| record.record_id == record_id).cloned())
    }

    pub fn latest_for_target(&self, target_uri: &str) -> Result<Option<ArchiveRecord>, VrtCError> {
        let normalized = normalize_target(target_uri)?;
        let state = self.lock_state()?;
        Ok(state
            .records
            .iter()
            .rev()
            .find(|record| record.target_uri == normalized)
            .cloned())
    }

    pub fn records(&self, limit: usize) -> Result<Vec<ArchiveRecord>, VrtCError> {
        if limit > self.config.max_records {
            return Err(archive_error(
                "ARCHIVE_QUERY_LIMIT",
                "Archive query limit exceeds configured record capacity",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let state = self.lock_state()?;
        Ok(state.records.iter().rev().take(limit).cloned().collect())
    }

    pub fn export_warc(&self) -> Result<Vec<u8>, VrtCError> {
        let state = self.lock_state()?;
        let mut output = Vec::with_capacity(state.bytes_retained);
        for record in &state.records {
            output.extend_from_slice(&record.to_warc_bytes());
        }
        Ok(output)
    }

    pub fn stats(&self) -> Result<ArchiveStats, VrtCError> {
        let state = self.lock_state()?;
        Ok(ArchiveStats {
            records_retained: state.records.len(),
            bytes_retained: state.bytes_retained,
            total_records_captured: state.total_records_captured,
            records_evicted: state.records_evicted,
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, ArchiveState>, VrtCError> {
        self.state.lock().map_err(|_| {
            archive_error(
                "ARCHIVE_LOCK_POISONED",
                "Archive state is temporarily unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn normalize_capture(capture: ArchiveCapture) -> Result<ArchiveRecord, VrtCError> {
    if !valid_uuid(&capture.record_id) {
        return Err(archive_error(
            "INVALID_ARCHIVE_RECORD_ID",
            "WARC record ID must be a canonical UUID",
            ErrorCategory::Validation,
        ));
    }
    if !valid_warc_date(&capture.captured_at) {
        return Err(archive_error(
            "INVALID_WARC_DATE",
            "Capture date must use canonical UTC YYYY-MM-DDTHH:MM:SSZ format",
            ErrorCategory::Validation,
        ));
    }
    if !(100..=599).contains(&capture.status)
        || capture.content_type.trim().is_empty()
        || capture.content_type.len() > 255
        || capture.content_type.chars().any(char::is_control)
    {
        return Err(archive_error(
            "INVALID_ARCHIVE_RESPONSE_METADATA",
            "Response status and content type must be valid and bounded",
            ErrorCategory::Validation,
        ));
    }

    let target_uri = normalize_target(&capture.target_uri)?;
    let safe_headers = capture
        .headers
        .into_iter()
        .filter(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "content-language" | "last-modified" | "etag" | "cache-control" | "expires"
            ) && valid_header_name(name)
                && value.len() <= 2048
                && !value.chars().any(char::is_control)
        })
        .collect();

    Ok(ArchiveRecord {
        record_id: capture.record_id.to_ascii_lowercase(),
        target_uri,
        captured_at: capture.captured_at,
        status: capture.status,
        content_type: capture.content_type,
        headers: safe_headers,
        body: capture.body,
        serialized_bytes: 0,
    })
}

fn normalize_target(raw: &str) -> Result<String, VrtCError> {
    let mut url = Url::parse(raw).map_err(|_| {
        archive_error(
            "INVALID_ARCHIVE_TARGET",
            "Archive target must be an absolute HTTP or HTTPS URL",
            ErrorCategory::Validation,
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(archive_error(
            "INVALID_ARCHIVE_TARGET",
            "Archive target must be a credential-free HTTP or HTTPS URL",
            ErrorCategory::PolicyDenied,
        ));
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

fn valid_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit()
        })
}

fn valid_warc_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [4, 7, 10, 13, 16, 19].contains(&index) || byte.is_ascii_digit())
        && value[5..7].parse::<u8>().is_ok_and(|month| (1..=12).contains(&month))
        && value[8..10].parse::<u8>().is_ok_and(|day| (1..=31).contains(&day))
        && value[11..13].parse::<u8>().is_ok_and(|hour| hour <= 23)
        && value[14..16].parse::<u8>().is_ok_and(|minute| minute <= 59)
        && value[17..19].parse::<u8>().is_ok_and(|second| second <= 60)
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_|~".contains(&byte))
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Recorded Response",
    }
}

fn archive_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false).for_component("engine.vrtc.archive")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(id: &str, target: &str, body: &[u8]) -> ArchiveCapture {
        ArchiveCapture {
            record_id: id.to_owned(),
            target_uri: target.to_owned(),
            captured_at: "2026-11-21T15:00:00Z".to_owned(),
            status: 200,
            content_type: "text/html; charset=utf-8".to_owned(),
            headers: vec![
                ("ETag".to_owned(), "\"v1\"".to_owned()),
                ("Set-Cookie".to_owned(), "session=private".to_owned()),
            ],
            body: body.to_vec(),
        }
    }

    #[test]
    fn captures_and_exports_warc_response_records() {
        let engine = ArchiveEngine::new(ArchiveConfig::default()).unwrap();
        let record = engine
            .capture(capture(
                "550e8400-e29b-41d4-a716-446655440000",
                "https://example.org/page#section",
                b"<html>archive</html>",
            ))
            .unwrap();
        let warc = record.to_warc_bytes();
        let text = String::from_utf8_lossy(&warc);
        assert!(text.starts_with("WARC/1.1\r\n"));
        assert!(text.contains("WARC-Type: response"));
        assert!(text.contains("WARC-Target-URI: https://example.org/page"));
        assert!(text.contains("ETag: \"v1\""));
        assert!(!text.contains("Set-Cookie"));
        assert!(text.ends_with("<html>archive</html>\r\n\r\n"));
        assert_eq!(engine.export_warc().unwrap(), warc);
    }

    #[test]
    fn bounded_archive_evicts_oldest_records() {
        let engine = ArchiveEngine::new(ArchiveConfig {
            max_records: 1,
            ..ArchiveConfig::default()
        })
        .unwrap();
        engine
            .capture(capture(
                "550e8400-e29b-41d4-a716-446655440000",
                "https://example.org/one",
                b"one",
            ))
            .unwrap();
        engine
            .capture(capture(
                "550e8400-e29b-41d4-a716-446655440001",
                "https://example.org/two",
                b"two",
            ))
            .unwrap();
        let stats = engine.stats().unwrap();
        assert_eq!(stats.records_retained, 1);
        assert_eq!(stats.records_evicted, 1);
        assert_eq!(
            engine.records(1).unwrap()[0].target_uri,
            "https://example.org/two"
        );
    }

    #[test]
    fn rejects_invalid_targets_dates_and_record_ids() {
        let engine = ArchiveEngine::new(ArchiveConfig::default()).unwrap();
        assert_eq!(
            engine
                .capture(capture("not-a-uuid", "https://example.org", b"x"))
                .unwrap_err()
                .code,
            "INVALID_ARCHIVE_RECORD_ID"
        );
        let invalid = capture(
            "550e8400-e29b-41d4-a716-446655440000",
            "file:///etc/passwd",
            b"x",
        );
        assert_eq!(
            engine.capture(invalid).unwrap_err().code,
            "INVALID_ARCHIVE_TARGET"
        );
    }

    #[test]
    fn refuses_records_that_exceed_the_byte_budget() {
        let engine = ArchiveEngine::new(ArchiveConfig {
            max_record_bytes: 64,
            max_total_bytes: 64,
            ..ArchiveConfig::default()
        })
        .unwrap();
        assert_eq!(
            engine
                .capture(capture(
                    "550e8400-e29b-41d4-a716-446655440000",
                    "https://example.org",
                    b"large",
                ))
                .unwrap_err()
                .code,
            "ARCHIVE_RECORD_TOO_LARGE"
        );
    }
}
