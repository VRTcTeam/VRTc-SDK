//! Native VRTc document indexing and lexical retrieval.
//!
//! Tantivy is adopted as the embedded Rust indexing primitive. VRTc owns the
//! document contract, schema, batch/upsert behavior, query boundary, and result
//! model. The index is local to the process; this crate is not a distributed
//! search cluster.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, TantivyDocument, Term};
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub storage_dir: Option<PathBuf>,
    pub writer_heap_size_bytes: usize,
    pub max_batch_documents: usize,
    pub max_document_bytes: usize,
    pub max_query_chars: usize,
    pub max_results: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            storage_dir: None,
            writer_heap_size_bytes: 15_000_000,
            max_batch_documents: 1_000,
            max_document_bytes: 4 * 1024 * 1024,
            max_query_chars: 512,
            max_results: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDocument {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub body: String,
    pub language: String,
    pub published_at: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexBatchResult {
    pub indexed: usize,
    pub committed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexHit {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub excerpt: String,
    pub language: String,
    pub published_at: String,
    pub source: String,
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexStats {
    pub documents: u64,
}

#[derive(Clone, Copy)]
struct IndexFields {
    id: Field,
    url: Field,
    title: Field,
    description: Field,
    body: Field,
    language: Field,
    published_at: Field,
    source: Field,
}

pub struct IndexEngine {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    fields: IndexFields,
    config: IndexConfig,
}

impl IndexEngine {
    pub fn new(config: IndexConfig) -> Result<Self, VrtCError> {
        if config.writer_heap_size_bytes < 15_000_000
            || config.max_batch_documents == 0
            || config.max_document_bytes == 0
            || config.max_query_chars == 0
            || config.max_results == 0
        {
            return Err(index_error(
                "INVALID_INDEX_CONFIG",
                "Index memory and document/query/result limits must be valid",
                ErrorCategory::Validation,
                false,
            ));
        }

        let schema = build_schema();
        let index = if let Some(directory) = &config.storage_dir {
            std::fs::create_dir_all(directory).map_err(|_| {
                index_error(
                    "INDEX_DIRECTORY_CREATE_FAILED",
                    "The index storage directory could not be created",
                    ErrorCategory::Internal,
                    false,
                )
            })?;
            if directory.join("meta.json").exists() {
                Index::open_in_dir(directory).map_err(|_| {
                    index_error(
                        "INDEX_OPEN_FAILED",
                        "The persisted index could not be opened",
                        ErrorCategory::Internal,
                        false,
                    )
                })?
            } else {
                Index::create_in_dir(directory, schema).map_err(|_| {
                    index_error(
                        "INDEX_CREATE_FAILED",
                        "The persisted index could not be created",
                        ErrorCategory::Internal,
                        false,
                    )
                })?
            }
        } else {
            Index::create_in_ram(schema)
        };

        let fields = fields_from_schema(&index.schema())?;
        let writer = index.writer(config.writer_heap_size_bytes).map_err(|_| {
            index_error(
                "INDEX_WRITER_INIT_FAILED",
                "The index writer could not be initialized",
                ErrorCategory::Internal,
                false,
            )
        })?;
        let reader = index.reader().map_err(|_| {
            index_error(
                "INDEX_READER_INIT_FAILED",
                "The index reader could not be initialized",
                ErrorCategory::Internal,
                false,
            )
        })?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            fields,
            config,
        })
    }

    /// Insert or replace a bounded batch of documents and publish one commit.
    pub fn index_batch(&self, documents: &[IndexDocument]) -> Result<IndexBatchResult, VrtCError> {
        if documents.len() > self.config.max_batch_documents {
            return Err(index_error(
                "INDEX_BATCH_TOO_LARGE",
                "The document batch exceeds the configured maximum",
                ErrorCategory::ResourceExhausted,
                false,
            ));
        }
        for document in documents {
            validate_document(document, self.config.max_document_bytes)?;
        }
        if documents.is_empty() {
            return Ok(IndexBatchResult {
                indexed: 0,
                committed: false,
            });
        }

        let mut writer = self.lock_writer()?;
        for document in documents {
            writer.delete_term(Term::from_field_text(self.fields.id, &document.id));
            writer
                .add_document(doc!(
                    self.fields.id => document.id.as_str(),
                    self.fields.url => document.url.as_str(),
                    self.fields.title => document.title.as_str(),
                    self.fields.description => document.description.as_str(),
                    self.fields.body => document.body.as_str(),
                    self.fields.language => document.language.as_str(),
                    self.fields.published_at => document.published_at.as_str(),
                    self.fields.source => document.source.as_str(),
                ))
                .map_err(|_| {
                    index_error(
                        "INDEX_DOCUMENT_FAILED",
                        "A document could not be added to the index",
                        ErrorCategory::Internal,
                        false,
                    )
                })?;
        }
        writer.commit().map_err(|_| {
            index_error(
                "INDEX_COMMIT_FAILED",
                "The document batch could not be committed",
                ErrorCategory::Internal,
                true,
            )
        })?;
        self.reader.reload().map_err(|_| {
            index_error(
                "INDEX_RELOAD_FAILED",
                "The committed index could not be reloaded for search",
                ErrorCategory::Internal,
                true,
            )
        })?;

        Ok(IndexBatchResult {
            indexed: documents.len(),
            committed: true,
        })
    }

    /// Delete one document by its stable VRTc document ID.
    pub fn delete(&self, id: &str) -> Result<(), VrtCError> {
        if id.trim().is_empty() {
            return Err(index_error(
                "INVALID_DOCUMENT_ID",
                "Document ID cannot be empty",
                ErrorCategory::Validation,
                false,
            ));
        }
        let mut writer = self.lock_writer()?;
        writer.delete_term(Term::from_field_text(self.fields.id, id));
        writer.commit().map_err(|_| {
            index_error(
                "INDEX_DELETE_COMMIT_FAILED",
                "The document deletion could not be committed",
                ErrorCategory::Internal,
                true,
            )
        })?;
        self.reader.reload().map_err(|_| {
            index_error(
                "INDEX_RELOAD_FAILED",
                "The committed index could not be reloaded for search",
                ErrorCategory::Internal,
                true,
            )
        })?;
        Ok(())
    }

    /// Run a BM25 lexical query over title, description, and body.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<IndexHit>, VrtCError> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > self.config.max_query_chars {
            return Err(index_error(
                "INVALID_INDEX_QUERY",
                "Search query must be non-empty and within the configured character limit",
                ErrorCategory::Validation,
                false,
            ));
        }
        if limit == 0 || limit > self.config.max_results {
            return Err(index_error(
                "INVALID_RESULT_LIMIT",
                "Search result limit is outside the configured range",
                ErrorCategory::Validation,
                false,
            ));
        }

        let parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.title, self.fields.description, self.fields.body],
        );
        let parsed = parser.parse_query(query).map_err(|_| {
            index_error(
                "INVALID_QUERY_SYNTAX",
                "The search query could not be parsed",
                ErrorCategory::Validation,
                false,
            )
        })?;
        let searcher = self.reader.searcher();
        let top_docs = searcher
            .search(&parsed, &TopDocs::with_limit(limit).order_by_score())
            .map_err(|_| {
                index_error(
                    "INDEX_SEARCH_FAILED",
                    "The index could not execute the query",
                    ErrorCategory::Internal,
                    true,
                )
            })?;

        top_docs
            .into_iter()
            .map(|(score, address)| {
                let document: TantivyDocument = searcher.doc(address).map_err(|_| {
                    index_error(
                        "INDEX_DOCUMENT_READ_FAILED",
                        "A matched document could not be read",
                        ErrorCategory::Internal,
                        false,
                    )
                })?;
                let body = stored_text(&document, self.fields.body);
                Ok(IndexHit {
                    id: stored_text(&document, self.fields.id),
                    url: stored_text(&document, self.fields.url),
                    title: stored_text(&document, self.fields.title),
                    description: stored_text(&document, self.fields.description),
                    excerpt: body.chars().take(320).collect(),
                    language: stored_text(&document, self.fields.language),
                    published_at: stored_text(&document, self.fields.published_at),
                    source: stored_text(&document, self.fields.source),
                    score,
                })
            })
            .collect()
    }

    pub fn stats(&self) -> IndexStats {
        IndexStats {
            documents: self.reader.searcher().num_docs(),
        }
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, IndexWriter>, VrtCError> {
        self.writer.lock().map_err(|_| {
            index_error(
                "INDEX_WRITER_LOCK_POISONED",
                "The index writer is temporarily unavailable",
                ErrorCategory::Internal,
                true,
            )
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("id", STRING | STORED);
    builder.add_text_field("url", STRING | STORED);
    builder.add_text_field("title", TEXT | STORED);
    builder.add_text_field("description", TEXT | STORED);
    builder.add_text_field("body", TEXT | STORED);
    builder.add_text_field("language", STRING | STORED);
    builder.add_text_field("published_at", STRING | STORED);
    builder.add_text_field("source", STRING | STORED);
    builder.build()
}

fn fields_from_schema(schema: &Schema) -> Result<IndexFields, VrtCError> {
    let field = |name: &str| {
        schema.get_field(name).map_err(|_| {
            index_error(
                "INDEX_SCHEMA_MISMATCH",
                "The persisted index does not contain the required VRTc schema",
                ErrorCategory::Validation,
                false,
            )
        })
    };
    Ok(IndexFields {
        id: field("id")?,
        url: field("url")?,
        title: field("title")?,
        description: field("description")?,
        body: field("body")?,
        language: field("language")?,
        published_at: field("published_at")?,
        source: field("source")?,
    })
}

fn validate_document(document: &IndexDocument, max_bytes: usize) -> Result<(), VrtCError> {
    if document.id.trim().is_empty()
        || document.url.trim().is_empty()
        || document.title.trim().is_empty()
    {
        return Err(index_error(
            "INVALID_INDEX_DOCUMENT",
            "Indexed documents require a stable ID, URL, and title",
            ErrorCategory::Validation,
            false,
        ));
    }
    let size = document.id.len()
        + document.url.len()
        + document.title.len()
        + document.description.len()
        + document.body.len()
        + document.language.len()
        + document.published_at.len()
        + document.source.len();
    if size > max_bytes {
        return Err(index_error(
            "INDEX_DOCUMENT_TOO_LARGE",
            "The document exceeds the configured byte limit",
            ErrorCategory::ResourceExhausted,
            false,
        ));
    }
    Ok(())
}

fn stored_text(document: &TantivyDocument, field: Field) -> String {
    document
        .get_first(field)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn index_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: &str, title: &str, body: &str) -> IndexDocument {
        IndexDocument {
            id: id.to_owned(),
            url: format!("https://example.org/{id}"),
            title: title.to_owned(),
            description: String::new(),
            body: body.to_owned(),
            language: "en".to_owned(),
            published_at: String::new(),
            source: "test".to_owned(),
        }
    }

    #[test]
    fn indexes_and_retrieves_documents_with_bm25() {
        let engine = IndexEngine::new(IndexConfig::default()).unwrap();
        let result = engine
            .index_batch(&[
                document(
                    "crawler",
                    "Crawler scheduling",
                    "A crawler uses a URL frontier and scheduler to process pages politely.",
                ),
                document(
                    "database",
                    "Database fundamentals",
                    "Relational tables store structured records and indexes.",
                ),
            ])
            .unwrap();
        assert_eq!(result.indexed, 2);
        assert!(result.committed);
        let hits = engine.search("crawler scheduler", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "crawler");
        assert!(hits[0].score > 0.0);
        assert_eq!(engine.stats().documents, 2);
    }

    #[test]
    fn upsert_replaces_the_existing_document_id() {
        let engine = IndexEngine::new(IndexConfig::default()).unwrap();
        engine
            .index_batch(&[document("doc-1", "Old title", "obsolete content")])
            .unwrap();
        engine
            .index_batch(&[document("doc-1", "New title", "fresh crawler content")])
            .unwrap();
        assert!(engine.search("obsolete", 10).unwrap().is_empty());
        assert_eq!(engine.search("fresh", 10).unwrap()[0].title, "New title");
        assert_eq!(engine.stats().documents, 1);
    }

    #[test]
    fn enforces_batch_document_and_query_limits() {
        let engine = IndexEngine::new(IndexConfig {
            max_batch_documents: 1,
            max_query_chars: 4,
            ..IndexConfig::default()
        })
        .unwrap();
        assert!(engine
            .index_batch(&[
                document("a", "First", "first content"),
                document("b", "Second", "second content"),
            ])
            .is_err());
        assert!(engine.search("query-too-long", 10).is_err());
        assert!(engine.search("valid", 0).is_err());
    }
}
