//! hacienda — xberg + PII/Compliance Distribution
//!
//! `use hacienda::*;` gives you:
//! - All of xberg: extract, enrich, chunk, embed, rerank, caption, CLI, API, bindings
//! - PII pipeline: detect, redact, explain, batch
//! - Compliance: DPIA, Model Card, DORA, AI Act checklists
//! - Audit: hash-chained log, CSV/JSON export
//! - Review: human-in-the-loop queue
//! - Glossary: entity linking, markdown links

// =========================================================================
// 1. RE-EXPORT ALL OF XBERG (features mirror xberg's feature flags)
// =========================================================================
pub use xberg::{
    // Core extraction
    extract, extract_batch, ExtractInput, ExtractionConfig, ExtractionResult,
    ExtractedDocument, Entity, EntityCategory, Chunk, Table, Metadata,

    // Enrichment
    enrich, EnrichedResult, EnrichmentConfig,
    NerEnrichmentConfig, ClassificationEnrichmentConfig, CaptioningEnrichmentConfig,

    // Chunking / Embeddings / Reranking / Captioning
    chunk_text, ChunkingResult,
    embed_texts, embed_texts_async, EmbeddingModelType,
    rerank, rerank_async, RerankedDocument,
    caption_image, caption_image_file, caption_images,

    // Configuration types
    OcrConfig, ChunkingConfig, EmbeddingConfig, RerankerConfig,
    NerConfig, NerBackendKind, RedactionConfig, RedactionStrategy,
    PiiCategory, CaptioningConfig, SummarizationConfig,
    ClassificationConfig, TranslationConfig,

    // Plugins
    register_post_processor, register_ocr_backend,
    register_document_extractor, register_embedding_backend,
    register_reranker_backend, register_tokenizer_backend,
    register_validator, register_renderer,
    PostProcessor, OcrBackend, DocumentExtractor,
    EmbeddingBackend, RerankerBackend, TokenizerBackend,
    Validator, Renderer, ProcessingStage,

    // Types & Errors
    OutputFormat, ExtractionErrorItem, ExtractionSummary,
    Result, XbergError,

    // Feature-gated re-exports
    #[cfg(feature = "ner")] detect_entities,
    #[cfg(feature = "ner")] NerBackend,
    #[cfg(feature = "redaction")] TokenCounter,
    #[cfg(feature = "redaction")] RedactionStrategy,
    #[cfg(feature = "presets")] { Preset, Registry, resolve },
};

// =========================================================================
// 2. HACIENDA PII EXTENSIONS
// =========================================================================
pub use hacienda_core::{
    // PII Pipeline
    PiiPipeline, PipelineConfig, PipelineResult, PipelineEntity,
    PipelineAuditEntry, PipelineMetrics,

    // Redaction
    RedactionMode, RedactionConfig as HaciendaRedactionConfig,
    RedactionProfile, PciProfile, HipaaProfile, CustomProfile,

    // Compliance
    ComplianceGenerator, ComplianceReport, ModelCard, DoraReport,
    ComplianceChecklist, ChecklistItem, AiActRiskLevel,

    // Audit
    AuditChain, FileSink, AuditEntry, AuditSink, EntitySource,
    AuditExporter, ExportFormat as AuditExportFormat,

    // Review Queue
    ReviewQueue, ReviewQueueItem, ReviewRequest, ReviewDecision,
    ReviewStatus, Priority, QueueStats, ReviewConfig,

    // Glossary / Entity Linking
    EntityGlossary, GlossaryEntry, generate_markdown_links,
    GlossaryConfig, LinkStyle,

    // Config
    HaciendaConfig, HaciendaFacadeConfig,
};

// =========================================================================
// 3. UNIFIED FACADE (single entry point for extract + PII + compliance)
// =========================================================================
pub use hacienda_core::facade::HaciendaFacade;

// =========================================================================
// 4. CLI & API EXTENSIONS
// =========================================================================
pub mod cli;
pub mod api;

/// Common imports for hacienda users
pub mod prelude {
    pub use crate::{
        extract, extract_batch, ExtractInput, ExtractionConfig, ExtractionResult,
        ExtractedDocument, Entity, EntityCategory, Chunk, Table, Metadata,
        enrich, EnrichedResult, EnrichmentConfig,
        chunk_text, ChunkingResult,
        embed_texts, embed_texts_async, EmbeddingModelType,
        rerank, rerank_async, RerankedDocument,
        caption_image, caption_image_file, caption_images,
        HaciendaFacade, HaciendaFacadeConfig, HaciendaResult,
        PiiPipeline, PipelineConfig, PipelineResult,
        ComplianceGenerator, ComplianceReport, ModelCard,
        AuditChain, FileSink, ReviewQueue, ReviewQueueItem,
        EntityGlossary, generate_markdown_links,
        Result, HaciendaError,
    };
}