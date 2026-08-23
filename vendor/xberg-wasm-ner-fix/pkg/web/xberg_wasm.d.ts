/* tslint:disable */
/* eslint-disable */

/**
 * A GLiNER2 model loaded into WASM memory, detecting entities locally.
 *
 * Construct with [`NerModel::load`], call [`NerModel::detect`] as many times as
 * needed, and call `free()` from JS when finished to release the weights.
 *
 * ```js
 * const model = await NerModel.load({ weights, tokenizer, encoderConfig });
 * const entities = await model.detect("Alice works at Acme Corp", {
 *   categories: ["person", "organization"],
 * });
 * model.free();
 * ```
 */
export class NerModel {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Detect entities in `text`, running inference inside the binary.
     *
     * `opts` may contain `categories`, an array of category names; unknown
     * names become custom zero-shot labels. Omitting it uses the backend's
     * default label set.
     *
     * Resolves to an array of `{ category, text, start, end, confidence }`,
     * with `start` and `end` as byte offsets into `text`.
     *
     * # Errors
     *
     * Returns a JS error if inference fails.
     */
    detect(text: string, opts: any): Promise<any>;
    /**
     * Load a GLiNER2 model from bytes the host has already fetched.
     *
     * `options` takes three byte buffers, each a `Uint8Array` or `ArrayBuffer`:
     * `weights` (`model.safetensors`), `tokenizer` (`tokenizer.json`), and
     * `encoderConfig` (the encoder `config.json`). Named rather than positional
     * so a transposition cannot be silent.
     *
     * Weights are never embedded in the `.wasm`; the host fetches them.
     *
     * # Errors
     *
     * Returns a JS error if a field is missing, is not a byte buffer, is
     * empty, or if the bytes do not parse as a GLiNER2 model.
     */
    static load(options: any): Promise<NerModel>;
}

/**
 * Hardware acceleration configuration for ONNX Runtime models.
 *
 * Controls which execution provider (CPU, CoreML, CUDA, TensorRT) is used
 * for inference in layout detection and embedding generation.
 *
 * # Example
 */
export class WasmAccelerationConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmAccelerationConfig;
    constructor(provider?: WasmExecutionProviderType | null, deviceId?: number | null);
    deviceId: number;
    get provider(): string;
    set provider(value: WasmExecutionProviderType);
}

/**
 * Types of inline text annotations.
 */
export class WasmAnnotationKind {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmAnnotationKind;
    constructor();
    annotationType: string;
    get name(): string | undefined;
    set name(value: string | null | undefined);
    get title(): string | undefined;
    set title(value: string | null | undefined);
    get url(): string | undefined;
    set url(value: string | null | undefined);
    get value(): string | undefined;
    set value(value: string | null | undefined);
}

/**
 * A single file extracted from an archive.
 *
 * When archives (ZIP, TAR, 7Z, GZIP) are extracted with recursive extraction
 * enabled, each processable file produces its own full `ExtractedDocument`.
 */
export class WasmArchiveEntry {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmArchiveEntry;
    constructor(path: string, mimeType: string, result: WasmExtractedDocument);
    mimeType: string;
    path: string;
    result: WasmExtractedDocument;
}

/**
 * Archive (ZIP/TAR/7Z) metadata.
 *
 * Extracted from compressed archive files containing file lists and size information.
 */
export class WasmArchiveMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmArchiveMetadata;
    constructor(format?: string | null, fileCount?: number | null, fileList?: string[] | null, totalSize?: bigint | null, compressedSize?: bigint | null);
    get compressedSize(): bigint | undefined;
    set compressedSize(value: bigint | null | undefined);
    fileCount: number;
    fileList: string[];
    format: string;
    totalSize: bigint;
}

/**
 * The category of a downloaded asset.
 */
export enum WasmAssetCategory {
    Document = 0,
    Image = 1,
    Audio = 2,
    Video = 3,
    Font = 4,
    Stylesheet = 5,
    Script = 6,
    Archive = 7,
    Data = 8,
    Other = 9,
}

/**
 * Authentication configuration.
 */
export class WasmAuthConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmAuthConfig;
    constructor();
    get name(): string | undefined;
    set name(value: string | null | undefined);
    get password(): string | undefined;
    set password(value: string | null | undefined);
    get token(): string | undefined;
    set token(value: string | null | undefined);
    type: string;
    get username(): string | undefined;
    set username(value: string | null | undefined);
    get value(): string | undefined;
    set value(value: string | null | undefined);
}

/**
 * BibTeX bibliography metadata.
 */
export class WasmBibtexMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmBibtexMetadata;
    constructor(entryCount?: number | null, citationKeys?: string[] | null, authors?: string[] | null, yearRange?: WasmYearRange | null, entryTypes?: any | null);
    authors: string[];
    citationKeys: string[];
    entryCount: number;
    get entryTypes(): any | undefined;
    set entryTypes(value: any | null | undefined);
    get yearRange(): WasmYearRange | undefined;
    set yearRange(value: WasmYearRange | null | undefined);
}

/**
 * Types of block-level elements in Djot.
 */
export enum WasmBlockType {
    Paragraph = 0,
    Heading = 1,
    Blockquote = 2,
    CodeBlock = 3,
    ListItem = 4,
    OrderedList = 5,
    BulletList = 6,
    TaskList = 7,
    DefinitionList = 8,
    DefinitionTerm = 9,
    DefinitionDescription = 10,
    Div = 11,
    Section = 12,
    ThematicBreak = 13,
    RawBlock = 14,
    MathDisplay = 15,
}

/**
 * Bounding box coordinates for element positioning.
 */
export class WasmBoundingBox {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmBoundingBox;
    constructor(x0?: number | null, y0?: number | null, x1?: number | null, y1?: number | null);
    x0: number;
    x1: number;
    y0: number;
    y1: number;
}

/**
 * Browser backend used for JavaScript rendering.
 */
export enum WasmBrowserBackend {
    Chromiumoxide = 0,
    Native = 1,
}

/**
 * Browser fallback configuration.
 */
export class WasmBrowserConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmBrowserConfig;
    constructor(mode?: WasmBrowserMode | null, backend?: WasmBrowserBackend | null, timeout?: bigint | null, wait?: WasmBrowserWait | null, blockUrlPatterns?: string[] | null, captureNetworkEvents?: boolean | null, sessionAffinity?: boolean | null, endpoint?: string | null, waitSelector?: string | null, extraWait?: bigint | null, proxy?: WasmProxyConfig | null, evalScript?: string | null, robotsUserAgent?: string | null);
    get backend(): string;
    set backend(value: WasmBrowserBackend);
    blockUrlPatterns: string[];
    captureNetworkEvents: boolean;
    get endpoint(): string | undefined;
    set endpoint(value: string | null | undefined);
    get evalScript(): string | undefined;
    set evalScript(value: string | null | undefined);
    get extraWait(): bigint | undefined;
    set extraWait(value: bigint | null | undefined);
    get mode(): string;
    set mode(value: WasmBrowserMode);
    get proxy(): WasmProxyConfig | undefined;
    set proxy(value: WasmProxyConfig | null | undefined);
    get robotsUserAgent(): string | undefined;
    set robotsUserAgent(value: string | null | undefined);
    sessionAffinity: boolean;
    get timeout(): bigint | undefined;
    set timeout(value: bigint | null | undefined);
    get wait(): string;
    set wait(value: WasmBrowserWait);
    get waitSelector(): string | undefined;
    set waitSelector(value: string | null | undefined);
}

/**
 * When to use the headless browser fallback.
 */
export enum WasmBrowserMode {
    Auto = 0,
    Always = 1,
    Never = 2,
    Stealth = 3,
}

/**
 * Wait strategy for browser page rendering.
 */
export enum WasmBrowserWait {
    NetworkIdle = 0,
    Selector = 1,
    Fixed = 2,
}

/**
 * Aggregate statistics for a xberg cache directory.
 */
export class WasmCacheStats {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCacheStats;
    constructor(totalFiles: number, totalSizeMb: number, availableSpaceMb: number, oldestFileAgeDays: number, newestFileAgeDays: number);
    availableSpaceMb: number;
    newestFileAgeDays: number;
    oldestFileAgeDays: number;
    totalFiles: number;
    totalSizeMb: number;
}

/**
 * How a structured-extraction preset is dispatched to the model.
 *
 * This is the preset-facing call mode (the `preferred_call_mode` field of a
 * `Preset`). The structured pipeline has a richer
 * runtime-only decision enum with skip and fallback states; this 3-variant
 * type is the stable, serializable surface presets and bindings depend on.
 */
export enum WasmCallMode {
    TextOnly = 0,
    VisionOnly = 1,
    TextPlusVision = 2,
}

/**
 * Configuration for the VLM captioning post-processor.
 */
export class WasmCaptioningConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCaptioningConfig;
    constructor(llm: WasmLlmConfig, minImageArea: number, prompt?: string | null);
    llm: WasmLlmConfig;
    minImageArea: number;
    get prompt(): string | undefined;
    set prompt(value: string | null | undefined);
}

/**
 * A single changed cell within a table.
 *
 * Defined here (rather than only in `crate.diff`) so `RevisionDelta` can
 * reference it unconditionally, without requiring the `diff` Cargo feature.
 * `crate.diff` re-exports this type verbatim.
 */
export class WasmCellChange {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCellChange;
    constructor(row: number, col: number, from: string, to: string);
    col: number;
    from: string;
    row: number;
    to: string;
}

/**
 * A text chunk with optional embedding and metadata.
 *
 * Chunks are created when chunking is enabled in `ExtractionConfig`. Each chunk
 * contains the text content, optional embedding vector (if embedding generation
 * is configured), and metadata about its position in the document.
 */
export class WasmChunk {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunk;
    constructor(content: string, chunkType: WasmChunkType, metadata: WasmChunkMetadata, embedding?: Float32Array | null);
    get chunkType(): string;
    set chunkType(value: WasmChunkType);
    content: string;
    get embedding(): Float32Array | undefined;
    set embedding(value: Float32Array | null | undefined);
    metadata: WasmChunkMetadata;
}

/**
 * Configuration for the chunk-classification post-processor.
 *
 * Chunk classification is always multi-label: a chunk may match zero, one, or
 * many of the configured definitions. This is the chunk-level equivalent of
 * `PageClassificationConfig`, but scoped to individual chunks
 * (`ExtractedDocument.chunks`) rather than whole pages, and built for large
 * taxonomies where each label needs its own description rather than a bare name.
 */
export class WasmChunkClassificationConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunkClassificationConfig;
    constructor(definitions: WasmChunkClassificationDefinition[], llm: WasmLlmConfig, batchSize: number, maxConcurrency: number, promptTemplate?: string | null);
    batchSize: number;
    definitions: WasmChunkClassificationDefinition[];
    llm: WasmLlmConfig;
    maxConcurrency: number;
    get promptTemplate(): string | undefined;
    set promptTemplate(value: string | null | undefined);
}

/**
 * A single labeled definition the chunk classifier may emit.
 *
 * Unlike `PageClassificationConfig.labels` (bare label names), chunk
 * classification targets potentially large domain taxonomies where every
 * label carries its own semantic description, letting the LLM disambiguate
 * similarly named labels without relying on the label string alone.
 */
export class WasmChunkClassificationDefinition {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunkClassificationDefinition;
    constructor(label: string, description: string);
    description: string;
    label: string;
}

/**
 * Metadata about a chunk's position in the original document.
 */
export class WasmChunkMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunkMetadata;
    constructor(byteStart: number, byteEnd: number, chunkIndex: number, totalChunks: number, headingPath: string[], imageIndices: Uint32Array, nodeIds: string[], pageSpans: WasmPageSpan[], classifications: WasmClassificationLabel[], tokenCount?: number | null, firstPage?: number | null, lastPage?: number | null, headingContext?: WasmHeadingContext | null);
    byteEnd: number;
    byteStart: number;
    chunkIndex: number;
    classifications: WasmClassificationLabel[];
    get firstPage(): number | undefined;
    set firstPage(value: number | null | undefined);
    get headingContext(): WasmHeadingContext | undefined;
    set headingContext(value: WasmHeadingContext | null | undefined);
    headingPath: string[];
    imageIndices: Uint32Array;
    get lastPage(): number | undefined;
    set lastPage(value: number | null | undefined);
    nodeIds: string[];
    pageSpans: WasmPageSpan[];
    get tokenCount(): number | undefined;
    set tokenCount(value: number | null | undefined);
    totalChunks: number;
}

/**
 * How chunk size is measured.
 *
 * Defaults to `Characters` (Unicode character count). When using token-based sizing,
 * chunks are sized by token count according to the specified tokenizer.
 *
 * Token-based sizing uses HuggingFace tokenizers loaded at runtime, or a tokenizer
 * backend you register yourself. Any tokenizer available on HuggingFace Hub can be
 * used, including OpenAI-compatible tokenizers (e.g., `Xenova/gpt-4o`,
 * `Xenova/cl100k_base`). To size chunks with your own tokenizer instead (llama.cpp/GGUF
 * vocabularies, SentencePiece models, custom vocabs), register a `TokenizerBackend`
 * with `register_tokenizer_backend` and set `model` to the registered name.
 */
export class WasmChunkSizing {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunkSizing;
    constructor();
    get cacheDir(): string | undefined;
    set cacheDir(value: string | null | undefined);
    get model(): string | undefined;
    set model(value: string | null | undefined);
    type: string;
}

/**
 * Semantic structural classification of a text chunk.
 *
 * Assigned by the heuristic classifier in `chunking.classifier`.
 * Defaults to `Unknown` when no rule matches.
 * Designed to be extended in future versions without breaking changes.
 */
export enum WasmChunkType {
    Heading = 0,
    PartyList = 1,
    Definitions = 2,
    OperativeClause = 3,
    SignatureBlock = 4,
    Schedule = 5,
    TableLike = 6,
    Formula = 7,
    CodeBlock = 8,
    Function = 9,
    Class = 10,
    Module = 11,
    Image = 12,
    OrgChart = 13,
    Diagram = 14,
    Unknown = 15,
}

/**
 * Type of text chunker to use.
 *
 * # Variants
 *
 * * `Text` - Generic text splitter, splits on whitespace and punctuation
 * * `Markdown` - Markdown-aware splitter, preserves formatting and structure
 * * `Yaml` - YAML-aware splitter, creates one chunk per top-level key
 * * `Semantic` - Topic-aware chunker. With an `EmbeddingConfig`, splits at
 *   embedding-based topic shifts tuned by `topic_threshold` (default 0.75,
 *   lower = more splits). Without an embedding, falls back to a
 *   structural-boundary heuristic (ALL-CAPS headers, numbered sections,
 *   blank-line paragraphs) and merges groups into chunks capped at
 *   `max_characters` (default 1000). `topic_threshold` has no effect in the
 *   fallback path. For best results, pair with an embedding model.
 */
export enum WasmChunkerType {
    Text = 0,
    Markdown = 1,
    Yaml = 2,
    Semantic = 3,
}

/**
 * Chunking configuration.
 *
 * Configures text chunking for document content, including chunk size,
 * overlap, trimming behavior, and optional embeddings.
 *
 * Use `..Default.default()` when constructing to allow for future field additions:
 */
export class WasmChunkingConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmChunkingConfig;
    constructor(maxCharacters?: number | null, overlap?: number | null, trim?: boolean | null, chunkerType?: WasmChunkerType | null, sizing?: any | null, prependHeadingContext?: boolean | null, tableChunking?: WasmTableChunkingMode | null, embedding?: WasmEmbeddingConfig | null, preset?: string | null, topicThreshold?: number | null);
    get chunkerType(): string;
    set chunkerType(value: WasmChunkerType);
    get embedding(): WasmEmbeddingConfig | undefined;
    set embedding(value: WasmEmbeddingConfig | null | undefined);
    maxCharacters: number;
    overlap: number;
    prependHeadingContext: boolean;
    get preset(): string | undefined;
    set preset(value: string | null | undefined);
    sizing: any;
    get tableChunking(): string;
    set tableChunking(value: WasmTableChunkingMode);
    get topicThreshold(): number | undefined;
    set topicThreshold(value: number | null | undefined);
    trim: boolean;
}

/**
 * Citation file metadata (RIS, PubMed, EndNote).
 */
export class WasmCitationMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCitationMetadata;
    constructor(citationCount?: number | null, authors?: string[] | null, dois?: string[] | null, keywords?: string[] | null, format?: string | null, yearRange?: WasmYearRange | null);
    authors: string[];
    citationCount: number;
    dois: string[];
    get format(): string | undefined;
    set format(value: string | null | undefined);
    keywords: string[];
    get yearRange(): WasmYearRange | undefined;
    set yearRange(value: WasmYearRange | null | undefined);
}

/**
 * A single label + confidence pair.
 */
export class WasmClassificationLabel {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmClassificationLabel;
    constructor(label: string, confidence?: number | null);
    get confidence(): number | undefined;
    set confidence(value: number | null | undefined);
    label: string;
}

/**
 * Code block fence style in Markdown output.
 *
 * Determines how code blocks (`<pre><code>`) are rendered in Markdown.
 */
export enum WasmCodeBlockStyle {
    Indented = 0,
    Backticks = 1,
    Tildes = 2,
}

/**
 * Content extraction and conversion configuration.
 *
 * Controls how HTML is converted to the output format. Uses
 * html-to-markdown-rs as the conversion engine for all formats
 * (markdown, plain text, djot).
 */
export class WasmContentConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmContentConfig;
    constructor(outputFormat?: string | null, preprocessingPreset?: string | null, removeNavigation?: boolean | null, removeForms?: boolean | null, stripTags?: string[] | null, preserveTags?: string[] | null, excludeSelectors?: string[] | null, skipImages?: boolean | null, wrap?: boolean | null, wrapWidth?: number | null, includeDocumentStructure?: boolean | null, maxDepth?: number | null);
    excludeSelectors: string[];
    includeDocumentStructure: boolean;
    get maxDepth(): number | undefined;
    set maxDepth(value: number | null | undefined);
    outputFormat: string;
    preprocessingPreset: string;
    preserveTags: string[];
    removeForms: boolean;
    removeNavigation: boolean;
    skipImages: boolean;
    stripTags: string[];
    wrap: boolean;
    wrapWidth: number;
}

/**
 * Cross-extractor content filtering configuration.
 *
 * Controls whether "furniture" content (headers, footers, page numbers,
 * watermarks, repeating text) is included in or stripped from extraction
 * results. Applies across all extractors (PDF, DOCX, RTF, ODT, HTML, etc.)
 * with format-specific implementation.
 *
 * When `None` on `ExtractionConfig`, each extractor uses its current
 * default behavior unchanged.
 */
export class WasmContentFilterConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmContentFilterConfig;
    constructor(includeHeaders?: boolean | null, includeFooters?: boolean | null, stripRepeatingText?: boolean | null, includeWatermarks?: boolean | null);
    includeFooters: boolean;
    includeHeaders: boolean;
    includeWatermarks: boolean;
    stripRepeatingText: boolean;
}

/**
 * Content layer classification for document nodes.
 *
 * Replaces separate body/furniture arrays with per-node granularity.
 */
export enum WasmContentLayer {
    Body = 0,
    Header = 1,
    Footer = 2,
    Footnote = 3,
}

/**
 * JATS contributor with role.
 */
export class WasmContributorRole {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmContributorRole;
    constructor(name: string, role?: string | null);
    name: string;
    get role(): string | undefined;
    set role(value: string | null | undefined);
}

/**
 * Main conversion options for HTML to Markdown conversion.
 *
 * Use `ConversionOptions.builder()` to construct, or `Default.default()` for defaults.
 *
 * # Example
 */
export class WasmConversionOptions {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmConversionOptions;
    constructor(headingStyle?: WasmHeadingStyle | null, listIndentType?: WasmListIndentType | null, listIndentWidth?: number | null, bullets?: string | null, strongEmSymbol?: string | null, escapeAsterisks?: boolean | null, escapeUnderscores?: boolean | null, escapeMisc?: boolean | null, escapeAscii?: boolean | null, codeLanguage?: string | null, autolinks?: boolean | null, defaultTitle?: boolean | null, brInTables?: boolean | null, compactTables?: boolean | null, highlightStyle?: WasmHighlightStyle | null, extractMetadata?: boolean | null, whitespaceMode?: WasmWhitespaceMode | null, stripNewlines?: boolean | null, wrap?: boolean | null, wrapWidth?: number | null, convertAsInline?: boolean | null, subSymbol?: string | null, supSymbol?: string | null, newlineStyle?: WasmNewlineStyle | null, codeBlockStyle?: WasmCodeBlockStyle | null, keepInlineImagesIn?: string[] | null, preprocessing?: WasmPreprocessingOptions | null, encoding?: string | null, debug?: boolean | null, stripTags?: string[] | null, preserveTags?: string[] | null, skipImages?: boolean | null, urlEscapeStyle?: WasmUrlEscapeStyle | null, linkStyle?: WasmLinkStyle | null, maxImageSize?: bigint | null, captureSvg?: boolean | null, inferDimensions?: boolean | null, excludeSelectors?: string[] | null, maxDepth?: number | null);
    autolinks: boolean;
    brInTables: boolean;
    bullets: string;
    captureSvg: boolean;
    get codeBlockStyle(): string;
    set codeBlockStyle(value: WasmCodeBlockStyle);
    codeLanguage: string;
    compactTables: boolean;
    convertAsInline: boolean;
    debug: boolean;
    defaultTitle: boolean;
    encoding: string;
    escapeAscii: boolean;
    escapeAsterisks: boolean;
    escapeMisc: boolean;
    escapeUnderscores: boolean;
    excludeSelectors: string[];
    extractMetadata: boolean;
    get headingStyle(): string;
    set headingStyle(value: WasmHeadingStyle);
    get highlightStyle(): string;
    set highlightStyle(value: WasmHighlightStyle);
    inferDimensions: boolean;
    keepInlineImagesIn: string[];
    get linkStyle(): string;
    set linkStyle(value: WasmLinkStyle);
    get listIndentType(): string;
    set listIndentType(value: WasmListIndentType);
    listIndentWidth: number;
    get maxDepth(): number | undefined;
    set maxDepth(value: number | null | undefined);
    maxImageSize: bigint;
    get newlineStyle(): string;
    set newlineStyle(value: WasmNewlineStyle);
    preprocessing: WasmPreprocessingOptions;
    preserveTags: string[];
    skipImages: boolean;
    stripNewlines: boolean;
    stripTags: string[];
    strongEmSymbol: string;
    subSymbol: string;
    supSymbol: string;
    get urlEscapeStyle(): string;
    set urlEscapeStyle(value: WasmUrlEscapeStyle);
    get whitespaceMode(): string;
    set whitespaceMode(value: WasmWhitespaceMode);
    wrap: boolean;
    wrapWidth: number;
}

/**
 * Configuration for crawl, scrape, and map operations.
 */
export class WasmCrawlConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCrawlConfig;
    constructor(respectRobotsTxt?: boolean | null, softHttpErrors?: boolean | null, stayOnDomain?: boolean | null, allowSubdomains?: boolean | null, includePaths?: string[] | null, excludePaths?: string[] | null, customHeaders?: any | null, requestTimeout?: bigint | null, maxRedirects?: number | null, retryCount?: number | null, retryCodes?: Uint16Array | null, cookiesEnabled?: boolean | null, removeTags?: string[] | null, content?: WasmContentConfig | null, downloadAssets?: boolean | null, assetTypes?: any[] | null, browser?: WasmBrowserConfig | null, userAgents?: string[] | null, captureScreenshot?: boolean | null, followDocumentUrls?: boolean | null, downloadDocuments?: boolean | null, documentMimeTypes?: string[] | null, saveBrowserProfile?: boolean | null, ssrf?: WasmSsrfPolicy | null, maxDepth?: number | null, maxPages?: number | null, maxConcurrent?: number | null, userAgent?: string | null, rateLimitMs?: bigint | null, auth?: any | null, maxBodySize?: number | null, mapLimit?: number | null, mapSearch?: string | null, maxAssetSize?: number | null, proxy?: WasmProxyConfig | null, documentUrlDepth?: number | null, documentMaxSize?: number | null, warcOutput?: string | null, browserProfile?: string | null);
    allowSubdomains: boolean;
    assetTypes: string[];
    get auth(): any | undefined;
    set auth(value: any | null | undefined);
    browser: WasmBrowserConfig;
    get browserProfile(): string | undefined;
    set browserProfile(value: string | null | undefined);
    captureScreenshot: boolean;
    content: WasmContentConfig;
    cookiesEnabled: boolean;
    customHeaders: any;
    get documentMaxSize(): number | undefined;
    set documentMaxSize(value: number | null | undefined);
    documentMimeTypes: string[];
    get documentUrlDepth(): number | undefined;
    set documentUrlDepth(value: number | null | undefined);
    downloadAssets: boolean;
    downloadDocuments: boolean;
    excludePaths: string[];
    followDocumentUrls: boolean;
    includePaths: string[];
    get mapLimit(): number | undefined;
    set mapLimit(value: number | null | undefined);
    get mapSearch(): string | undefined;
    set mapSearch(value: string | null | undefined);
    get maxAssetSize(): number | undefined;
    set maxAssetSize(value: number | null | undefined);
    get maxBodySize(): number | undefined;
    set maxBodySize(value: number | null | undefined);
    get maxConcurrent(): number | undefined;
    set maxConcurrent(value: number | null | undefined);
    get maxDepth(): number | undefined;
    set maxDepth(value: number | null | undefined);
    get maxPages(): number | undefined;
    set maxPages(value: number | null | undefined);
    maxRedirects: number;
    get proxy(): WasmProxyConfig | undefined;
    set proxy(value: WasmProxyConfig | null | undefined);
    get rateLimitMs(): bigint | undefined;
    set rateLimitMs(value: bigint | null | undefined);
    removeTags: string[];
    get requestTimeout(): bigint | undefined;
    set requestTimeout(value: bigint | null | undefined);
    respectRobotsTxt: boolean;
    retryCodes: Uint16Array;
    retryCount: number;
    saveBrowserProfile: boolean;
    softHttpErrors: boolean;
    ssrf: WasmSsrfPolicy;
    stayOnDomain: boolean;
    get userAgent(): string | undefined;
    set userAgent(value: string | null | undefined);
    userAgents: string[];
    get warcOutput(): string | undefined;
    set warcOutput(value: string | null | undefined);
}

/**
 * CSV/TSV file metadata.
 */
export class WasmCsvMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmCsvMetadata;
    constructor(rowCount?: number | null, columnCount?: number | null, hasHeader?: boolean | null, delimiter?: string | null, columnTypes?: string[] | null);
    columnCount: number;
    get columnTypes(): string[] | undefined;
    set columnTypes(value: string[] | null | undefined);
    get delimiter(): string | undefined;
    set delimiter(value: string | null | undefined);
    hasHeader: boolean;
    rowCount: number;
}

/**
 * dBASE field information.
 */
export class WasmDbfFieldInfo {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDbfFieldInfo;
    constructor(name: string, fieldType: string);
    fieldType: string;
    name: string;
}

/**
 * dBASE (DBF) file metadata.
 */
export class WasmDbfMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDbfMetadata;
    constructor(recordCount?: number | null, fieldCount?: number | null, fields?: WasmDbfFieldInfo[] | null);
    fieldCount: number;
    fields: WasmDbfFieldInfo[];
    recordCount: number;
}

/**
 * A single line in a unified-diff hunk.
 *
 * Defined here (rather than only in `crate.diff`) so `RevisionDelta` can
 * reference it unconditionally, without requiring the `diff` Cargo feature.
 * `crate.diff` re-exports this type verbatim.
 */
export class WasmDiffLine {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDiffLine;
    constructor();
    get 0(): string | undefined;
    set 0(value: string | null | undefined);
    kind: string;
}

/**
 * Comprehensive Djot document structure with semantic preservation.
 *
 * This type captures the full richness of Djot markup, including:
 * - Block-level structures (headings, lists, blockquotes, code blocks, etc.)
 * - Inline formatting (emphasis, strong, highlight, subscript, superscript, etc.)
 * - Attributes (classes, IDs, key-value pairs)
 * - Links, images, footnotes
 * - Math expressions (inline and display)
 * - Tables with full structure
 *
 * Available when the `djot` feature is enabled.
 */
export class WasmDjotContent {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDjotContent;
    constructor(plainText: string, blocks: WasmFormattedBlock[], metadata: WasmMetadata, tables: WasmTable[], images: WasmDjotImage[], links: WasmDjotLink[], footnotes: WasmFootnote[]);
    blocks: WasmFormattedBlock[];
    footnotes: WasmFootnote[];
    images: WasmDjotImage[];
    links: WasmDjotLink[];
    metadata: WasmMetadata;
    plainText: string;
    tables: WasmTable[];
}

/**
 * Image element in Djot.
 */
export class WasmDjotImage {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDjotImage;
    constructor(src: string, alt: string, title?: string | null);
    alt: string;
    src: string;
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * Link element in Djot.
 */
export class WasmDjotLink {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDjotLink;
    constructor(url: string, text: string, title?: string | null);
    text: string;
    get title(): string | undefined;
    set title(value: string | null | undefined);
    url: string;
}

/**
 * Cheap structural counts for an extracted document.
 *
 * Populated on every `ExtractedDocument` returned by `extract` /
 * `extract_batch`, regardless of whether the heavy `pages` / `images`
 * collections are materialized. A caller that only needs "how many pages /
 * tables / images did this document have?" (reporting, cost estimation,
 * progress, quotas) can read these without enabling per-page or per-image
 * extraction.
 *
 * The page count comes from the parse (the extractor already walks the page
 * tree); it does not require opting into per-page content. `pages` is `0` for
 * inputs that are not page-addressable (e.g. plain text).
 */
export class WasmDocumentCounts {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentCounts;
    constructor(pages?: number | null, tables?: number | null, images?: number | null);
    images: number;
    pages: number;
    tables: number;
}

/**
 * A single node in the document tree.
 *
 * Each node has deterministic `id`, typed `content`, optional `parent`/`children`
 * for tree structure, and metadata like page number, bounding box, and content layer.
 */
export class WasmDocumentNode {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentNode;
    constructor(id: string, content: any, children: Uint32Array, contentLayer: WasmContentLayer, annotations: WasmTextAnnotation[], parent?: number | null, page?: number | null, pageEnd?: number | null, bbox?: WasmBoundingBox | null, attributes?: any | null);
    annotations: WasmTextAnnotation[];
    get attributes(): any | undefined;
    set attributes(value: any | null | undefined);
    get bbox(): WasmBoundingBox | undefined;
    set bbox(value: WasmBoundingBox | null | undefined);
    children: Uint32Array;
    content: any;
    get contentLayer(): string;
    set contentLayer(value: WasmContentLayer);
    id: string;
    get page(): number | undefined;
    set page(value: number | null | undefined);
    get pageEnd(): number | undefined;
    set pageEnd(value: number | null | undefined);
    get parent(): number | undefined;
    set parent(value: number | null | undefined);
}

/**
 * A resolved relationship between two nodes in the document tree.
 */
export class WasmDocumentRelationship {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentRelationship;
    constructor(source: number, target: number, kind: WasmRelationshipKind);
    get kind(): string;
    set kind(value: WasmRelationshipKind);
    source: number;
    target: number;
}

/**
 * A single tracked change embedded in a document.
 *
 * Populated by per-format extractors that understand change-tracking metadata
 * (DOCX `w:ins`/`w:del`/`w:rPrChange`, ODT `text:change-*`, …). Every
 * extractor defaults to `ExtractedDocument.revisions = None` until a
 * format-specific implementation is added.
 */
export class WasmDocumentRevision {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentRevision;
    constructor(revisionId: string, kind: WasmRevisionKind, delta: WasmRevisionDelta, author?: string | null, timestamp?: string | null, anchor?: any | null);
    get anchor(): any | undefined;
    set anchor(value: any | null | undefined);
    get author(): string | undefined;
    set author(value: string | null | undefined);
    delta: WasmRevisionDelta;
    get kind(): string;
    set kind(value: WasmRevisionKind);
    revisionId: string;
    get timestamp(): string | undefined;
    set timestamp(value: string | null | undefined);
}

/**
 * Top-level structured document representation.
 *
 * A flat array of nodes with index-based parent/child references forming a tree.
 * Root-level nodes have `parent: None`. Use `body_roots()` and `furniture_roots()`
 * to iterate over top-level content by layer.
 *
 * # Validation
 *
 * Call `validate()` after construction to verify all node indices are in bounds
 * and parent-child relationships are bidirectionally consistent.
 */
export class WasmDocumentStructure {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentStructure;
    /**
     * Compute and populate the `node_types` field from the current `nodes`.
     *
     * Call this after all nodes have been added to the structure. Internal
     * construction paths (builder, derivation) call this automatically.
     *
     * # Examples
     */
    finalizeNodeTypes(): void;
    /**
     * Check if the document structure is empty.
     */
    isEmpty(): boolean;
    constructor(nodes?: WasmDocumentNode[] | null, relationships?: WasmDocumentRelationship[] | null, nodeTypes?: string[] | null, sourceFormat?: string | null);
    nodeTypes: string[];
    nodes: WasmDocumentNode[];
    relationships: WasmDocumentRelationship[];
    get sourceFormat(): string | undefined;
    set sourceFormat(value: string | null | undefined);
}

/**
 * Summary of an extracted document.
 */
export class WasmDocumentSummary {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmDocumentSummary;
    constructor(text: string, strategy: WasmSummaryStrategy, tokenCount?: number | null);
    get strategy(): string;
    set strategy(value: WasmSummaryStrategy);
    text: string;
    get tokenCount(): number | undefined;
    set tokenCount(value: number | null | undefined);
}

/**
 * Semantic element extracted from document.
 *
 * Represents a logical unit of content with semantic classification,
 * unique identifier, and metadata for tracking origin and position.
 */
export class WasmElement {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmElement;
    constructor(elementType: WasmElementType, text: string, metadata: WasmElementMetadata);
    get elementType(): string;
    set elementType(value: WasmElementType);
    metadata: WasmElementMetadata;
    text: string;
}

/**
 * Metadata for a semantic element.
 */
export class WasmElementMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmElementMetadata;
    constructor(additional: any, pageNumber?: number | null, filename?: string | null, coordinates?: WasmBoundingBox | null, elementIndex?: number | null);
    additional: any;
    get coordinates(): WasmBoundingBox | undefined;
    set coordinates(value: WasmBoundingBox | null | undefined);
    get elementIndex(): number | undefined;
    set elementIndex(value: number | null | undefined);
    get filename(): string | undefined;
    set filename(value: string | null | undefined);
    get pageNumber(): number | undefined;
    set pageNumber(value: number | null | undefined);
}

/**
 * Semantic element type classification.
 *
 * Categorizes text content into semantic units for downstream processing.
 * Supports the element types commonly found in Unstructured documents.
 */
export enum WasmElementType {
    Title = 0,
    NarrativeText = 1,
    Heading = 2,
    ListItem = 3,
    Table = 4,
    Image = 5,
    PageBreak = 6,
    CodeBlock = 7,
    BlockQuote = 8,
    Footer = 9,
    Header = 10,
}

/**
 * Email attachment representation.
 *
 * Contains metadata and optionally the content of an email attachment.
 */
export class WasmEmailAttachment {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmailAttachment;
    constructor(isImage: boolean, name?: string | null, filename?: string | null, mimeType?: string | null, size?: number | null, data?: Uint8Array | null);
    get data(): Uint8Array | undefined;
    set data(value: Uint8Array | null | undefined);
    get filename(): string | undefined;
    set filename(value: string | null | undefined);
    isImage: boolean;
    get mimeType(): string | undefined;
    set mimeType(value: string | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    get size(): number | undefined;
    set size(value: number | null | undefined);
}

/**
 * Configuration for email extraction.
 */
export class WasmEmailConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmailConfig;
    constructor(msgFallbackCodepage?: number | null);
    get msgFallbackCodepage(): number | undefined;
    set msgFallbackCodepage(value: number | null | undefined);
}

/**
 * Email extraction result.
 *
 * Complete representation of an extracted email message (.eml or .msg)
 * including headers, body content, and attachments.
 */
export class WasmEmailExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmailExtractionResult;
    constructor(toEmails: string[], ccEmails: string[], bccEmails: string[], content: string, attachments: WasmEmailAttachment[], metadata: any, subject?: string | null, fromEmail?: string | null, date?: string | null, messageId?: string | null, plainText?: string | null, htmlContent?: string | null);
    attachments: WasmEmailAttachment[];
    bccEmails: string[];
    ccEmails: string[];
    content: string;
    get date(): string | undefined;
    set date(value: string | null | undefined);
    get fromEmail(): string | undefined;
    set fromEmail(value: string | null | undefined);
    get htmlContent(): string | undefined;
    set htmlContent(value: string | null | undefined);
    get messageId(): string | undefined;
    set messageId(value: string | null | undefined);
    metadata: any;
    get plainText(): string | undefined;
    set plainText(value: string | null | undefined);
    get subject(): string | undefined;
    set subject(value: string | null | undefined);
    toEmails: string[];
}

/**
 * Email metadata extracted from .eml and .msg files.
 *
 * Includes sender/recipient information, message ID, and attachment list.
 */
export class WasmEmailMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmailMetadata;
    constructor(toEmails?: string[] | null, ccEmails?: string[] | null, bccEmails?: string[] | null, attachments?: string[] | null, fromEmail?: string | null, fromName?: string | null, messageId?: string | null);
    attachments: string[];
    bccEmails: string[];
    ccEmails: string[];
    get fromEmail(): string | undefined;
    set fromEmail(value: string | null | undefined);
    get fromName(): string | undefined;
    set fromName(value: string | null | undefined);
    get messageId(): string | undefined;
    set messageId(value: string | null | undefined);
    toEmails: string[];
}

/**
 * Embedding configuration for text chunks.
 *
 * Configures embedding generation using ONNX models via the vendored embedding engine.
 * Requires the `embeddings` feature to be enabled.
 */
export class WasmEmbeddingConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmbeddingConfig;
    constructor(model?: any | null, normalize?: boolean | null, batchSize?: number | null, showDownloadProgress?: boolean | null, cacheDir?: string | null, acceleration?: WasmAccelerationConfig | null, maxEmbedDurationSecs?: bigint | null, maxSequenceLength?: number | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    batchSize: number;
    get cacheDir(): string | undefined;
    set cacheDir(value: string | null | undefined);
    get maxEmbedDurationSecs(): bigint | undefined;
    set maxEmbedDurationSecs(value: bigint | null | undefined);
    get maxSequenceLength(): number | undefined;
    set maxSequenceLength(value: number | null | undefined);
    model: any;
    normalize: boolean;
    showDownloadProgress: boolean;
}

/**
 * Embedding model types supported by Xberg.
 */
export class WasmEmbeddingModelType {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEmbeddingModelType;
    constructor();
    get dimensions(): number | undefined;
    set dimensions(value: number | null | undefined);
    get llm(): WasmLlmConfig | undefined;
    set llm(value: WasmLlmConfig | null | undefined);
    get modelId(): string | undefined;
    set modelId(value: string | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    type: string;
}

/**
 * A single named entity detected in the extracted text.
 */
export class WasmEntity {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEntity;
    constructor(category: WasmEntityCategory, text: string, start: number, end: number, confidence?: number | null);
    get category(): string;
    set category(value: WasmEntityCategory);
    get confidence(): number | undefined;
    set confidence(value: number | null | undefined);
    end: number;
    start: number;
    text: string;
}

/**
 * Standard entity categories produced by built-in NER backends.
 *
 * The `Custom(String)` variant lets caller-supplied categories (e.g. LLM
 * schemas) flow through without losing fidelity to the consumer.
 */
export enum WasmEntityCategory {
    Person = 0,
    Organization = 1,
    Location = 2,
    Date = 3,
    Time = 4,
    Money = 5,
    Percent = 6,
    Email = 7,
    Phone = 8,
    Url = 9,
    Custom = 10,
}

/**
 * EPUB metadata (Dublin Core extensions).
 */
export class WasmEpubMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmEpubMetadata;
    constructor(coverage?: string | null, dcFormat?: string | null, relation?: string | null, source?: string | null, dcType?: string | null, coverImage?: string | null);
    get coverImage(): string | undefined;
    set coverImage(value: string | null | undefined);
    get coverage(): string | undefined;
    set coverage(value: string | null | undefined);
    get dcFormat(): string | undefined;
    set dcFormat(value: string | null | undefined);
    get dcType(): string | undefined;
    set dcType(value: string | null | undefined);
    get relation(): string | undefined;
    set relation(value: string | null | undefined);
    get source(): string | undefined;
    set source(value: string | null | undefined);
}

/**
 * Error metadata (for batch operations).
 */
export class WasmErrorMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmErrorMetadata;
    constructor(errorType: string, message: string);
    errorType: string;
    message: string;
}

/**
 * Excel/spreadsheet format metadata.
 *
 * Identifies the document as a spreadsheet source via the `FormatMetadata.Excel`
 * discriminant. Sheet count and sheet names are stored inside this struct.
 */
export class WasmExcelMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExcelMetadata;
    constructor(sheetCount?: number | null, sheetNames?: string[] | null);
    get sheetCount(): number | undefined;
    set sheetCount(value: number | null | undefined);
    get sheetNames(): string[] | undefined;
    set sheetNames(value: string[] | null | undefined);
}

/**
 * Single Excel worksheet.
 *
 * Represents one sheet from an Excel workbook with its content
 * converted to Markdown format and dimensional statistics.
 */
export class WasmExcelSheet {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExcelSheet;
    constructor(name: string, markdown: string, rowCount: number, colCount: number, cellCount: number, tableCells?: any | null);
    cellCount: number;
    colCount: number;
    markdown: string;
    name: string;
    rowCount: number;
    get tableCells(): any | undefined;
    set tableCells(value: any | null | undefined);
}

/**
 * Excel workbook representation.
 *
 * Contains all sheets from an Excel file (.xlsx, .xls, etc.) with
 * extracted content and metadata.
 */
export class WasmExcelWorkbook {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExcelWorkbook;
    constructor(sheets: WasmExcelSheet[], metadata: any, revisions?: WasmDocumentRevision[] | null);
    metadata: any;
    get revisions(): Array<any> | undefined;
    set revisions(value: WasmDocumentRevision[] | null | undefined);
    sheets: WasmExcelSheet[];
}

/**
 * ONNX Runtime execution provider type.
 *
 * Determines which hardware backend is used for model inference.
 * `Auto` (default) selects the best available provider per platform.
 */
export enum WasmExecutionProviderType {
    Auto = 0,
    Cpu = 1,
    CoreMl = 2,
    Cuda = 3,
    TensorRt = 4,
}

/**
 * Unified extraction input for all public extraction entry points.
 */
export class WasmExtractInput {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractInput;
    /**
     * Build a bytes input with a MIME type and optional filename hint.
     */
    static fromBytes(bytes: Uint8Array, mime_type: string, filename?: string | null): WasmExtractInput;
    /**
     * Build a URI input from a local path, `file://` URI, or HTTP(S) URL.
     */
    static fromUri(uri: string): WasmExtractInput;
    constructor(kind?: WasmExtractInputKind | null, bytes?: Uint8Array | null, uri?: string | null, mimeType?: string | null, filename?: string | null, config?: WasmFileExtractionConfig | null);
    get bytes(): Uint8Array | undefined;
    set bytes(value: Uint8Array | null | undefined);
    get config(): WasmFileExtractionConfig | undefined;
    set config(value: WasmFileExtractionConfig | null | undefined);
    get filename(): string | undefined;
    set filename(value: string | null | undefined);
    get kind(): string;
    set kind(value: WasmExtractInputKind);
    get mimeType(): string | undefined;
    set mimeType(value: string | null | undefined);
    get uri(): string | undefined;
    set uri(value: string | null | undefined);
}

/**
 * Source kind for `ExtractInput`.
 */
export enum WasmExtractInputKind {
    Bytes = 0,
    Uri = 1,
}

/**
 * Document extracted by the core extraction pipeline.
 *
 * `extract` and `extract_batch` return an `ExtractionResult` envelope whose
 * `results` field contains these per-document payloads.
 */
export class WasmExtractedDocument {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractedDocument;
    constructor(content?: string | null, mimeType?: string | null, metadata?: WasmMetadata | null, tables?: WasmTable[] | null, counts?: WasmDocumentCounts | null, processingWarnings?: WasmProcessingWarning[] | null, formulas?: WasmFormula[] | null, formFields?: WasmPdfFormField[] | null, extractionMethod?: WasmExtractionMethod | null, detectedLanguages?: string[] | null, chunks?: WasmChunk[] | null, images?: WasmExtractedImage[] | null, pages?: WasmPageContent[] | null, elements?: WasmElement[] | null, djotContent?: WasmDjotContent | null, ocrElements?: WasmOcrElement[] | null, document?: WasmDocumentStructure | null, qualityScore?: number | null, annotations?: WasmPdfAnnotation[] | null, children?: WasmArchiveEntry[] | null, uris?: WasmExtractedUri[] | null, revisions?: WasmDocumentRevision[] | null, structuredOutput?: any | null, llmUsage?: WasmLlmUsage[] | null, entities?: WasmEntity[] | null, summary?: WasmDocumentSummary | null, translation?: WasmTranslation | null, pageClassifications?: WasmPageClassification[] | null, redactionReport?: WasmRedactionReport | null, formattedContent?: string | null);
    get annotations(): Array<any> | undefined;
    set annotations(value: WasmPdfAnnotation[] | null | undefined);
    get children(): Array<any> | undefined;
    set children(value: WasmArchiveEntry[] | null | undefined);
    get chunks(): Array<any> | undefined;
    set chunks(value: WasmChunk[] | null | undefined);
    content: string;
    counts: WasmDocumentCounts;
    get detectedLanguages(): string[] | undefined;
    set detectedLanguages(value: string[] | null | undefined);
    get djotContent(): WasmDjotContent | undefined;
    set djotContent(value: WasmDjotContent | null | undefined);
    get document(): WasmDocumentStructure | undefined;
    set document(value: WasmDocumentStructure | null | undefined);
    get elements(): Array<any> | undefined;
    set elements(value: WasmElement[] | null | undefined);
    get entities(): Array<any> | undefined;
    set entities(value: WasmEntity[] | null | undefined);
    get extractionMethod(): string | undefined;
    set extractionMethod(value: WasmExtractionMethod | null | undefined);
    formFields: WasmPdfFormField[];
    get formattedContent(): string | undefined;
    set formattedContent(value: string | null | undefined);
    formulas: WasmFormula[];
    get images(): Array<any> | undefined;
    set images(value: WasmExtractedImage[] | null | undefined);
    get llmUsage(): Array<any> | undefined;
    set llmUsage(value: WasmLlmUsage[] | null | undefined);
    metadata: WasmMetadata;
    mimeType: string;
    get ocrElements(): Array<any> | undefined;
    set ocrElements(value: WasmOcrElement[] | null | undefined);
    get pageClassifications(): Array<any> | undefined;
    set pageClassifications(value: WasmPageClassification[] | null | undefined);
    get pages(): Array<any> | undefined;
    set pages(value: WasmPageContent[] | null | undefined);
    processingWarnings: WasmProcessingWarning[];
    get qualityScore(): number | undefined;
    set qualityScore(value: number | null | undefined);
    get redactionReport(): WasmRedactionReport | undefined;
    set redactionReport(value: WasmRedactionReport | null | undefined);
    get revisions(): Array<any> | undefined;
    set revisions(value: WasmDocumentRevision[] | null | undefined);
    get structuredOutput(): any | undefined;
    set structuredOutput(value: any | null | undefined);
    get summary(): WasmDocumentSummary | undefined;
    set summary(value: WasmDocumentSummary | null | undefined);
    tables: WasmTable[];
    get translation(): WasmTranslation | undefined;
    set translation(value: WasmTranslation | null | undefined);
    get uris(): Array<any> | undefined;
    set uris(value: WasmExtractedUri[] | null | undefined);
}

/**
 * Extracted image from a document.
 *
 * Contains raw image data, metadata, and optional nested OCR results.
 * Raw bytes allow cross-language compatibility - users can convert to
 * PIL.Image (Python), Sharp (Node.js), or other formats as needed.
 */
export class WasmExtractedImage {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractedImage;
    constructor(data?: Uint8Array | null, format?: string | null, imageIndex?: number | null, isMask?: boolean | null, pageNumber?: number | null, width?: number | null, height?: number | null, colorspace?: string | null, bitsPerComponent?: number | null, description?: string | null, ocrResult?: WasmExtractedDocument | null, boundingBox?: WasmBoundingBox | null, sourcePath?: string | null, imageKind?: WasmImageKind | null, kindConfidence?: number | null, clusterId?: number | null, caption?: string | null, qrCodes?: WasmQrCode[] | null, dataBase64?: string | null);
    get bitsPerComponent(): number | undefined;
    set bitsPerComponent(value: number | null | undefined);
    get boundingBox(): WasmBoundingBox | undefined;
    set boundingBox(value: WasmBoundingBox | null | undefined);
    get caption(): string | undefined;
    set caption(value: string | null | undefined);
    get clusterId(): number | undefined;
    set clusterId(value: number | null | undefined);
    get colorspace(): string | undefined;
    set colorspace(value: string | null | undefined);
    data: Uint8Array;
    get dataBase64(): string | undefined;
    set dataBase64(value: string | null | undefined);
    get description(): string | undefined;
    set description(value: string | null | undefined);
    format: string;
    get height(): number | undefined;
    set height(value: number | null | undefined);
    imageIndex: number;
    get imageKind(): string | undefined;
    set imageKind(value: WasmImageKind | null | undefined);
    isMask: boolean;
    get kindConfidence(): number | undefined;
    set kindConfidence(value: number | null | undefined);
    get ocrResult(): WasmExtractedDocument | undefined;
    set ocrResult(value: WasmExtractedDocument | null | undefined);
    get pageNumber(): number | undefined;
    set pageNumber(value: number | null | undefined);
    get qrCodes(): Array<any> | undefined;
    set qrCodes(value: WasmQrCode[] | null | undefined);
    get sourcePath(): string | undefined;
    set sourcePath(value: string | null | undefined);
    get width(): number | undefined;
    set width(value: number | null | undefined);
}

/**
 * A URI extracted from a document.
 *
 * Represents any link, reference, or resource pointer found during extraction.
 * The `kind` field classifies the URI semantically, while `label` carries
 * optional human-readable display text.
 */
export class WasmExtractedUri {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractedUri;
    constructor(url: string, kind: WasmUriKind, label?: string | null, page?: number | null);
    get kind(): string;
    set kind(value: WasmUriKind);
    get label(): string | undefined;
    set label(value: string | null | undefined);
    get page(): number | undefined;
    set page(value: number | null | undefined);
    url: string;
}

/**
 * Main extraction configuration.
 *
 * This struct contains all configuration options for the extraction process.
 * It can be loaded from TOML, YAML, or JSON files, or created programmatically.
 *
 * # Example
 */
export class WasmExtractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractionConfig;
    /**
     * Check if image processing is needed by examining OCR and image extraction settings.
     *
     * Returns `true` if either OCR is enabled or image extraction is configured,
     * indicating that image decompression and processing should occur.
     * Returns `false` if both are disabled, allowing optimization to skip unnecessary
     * image decompression for text-only extraction workflows.
     *
     * # Optimization Impact
     * For text-only extractions (no OCR, no image extraction), skipping image
     * decompression can improve CPU utilization by 5-10% by avoiding wasteful
     * image I/O and processing when results won't be used.
     * Returns `true` when image binary data should be extracted.
     *
     * True when `config.images.extract_images` is set, captioning is configured, or QR-code
     * detection is enabled. Captioning and QR-code detection both require image bytes
     * regardless of whether the caller also requested image extraction.
     */
    needsImageData(): boolean;
    /**
     * Returns `true` when any image processing is needed during extraction.
     *
     * # Optimization Impact
     *
     * For text-only extractions (no OCR, no image extraction, no captioning), skipping
     * image decompression can improve CPU utilization by 5-10% by avoiding wasteful
     * image I/O and processing when results won't be used.
     */
    needsImageProcessing(): boolean;
    constructor(useCache?: boolean | null, enableQualityProcessing?: boolean | null, forceOcr?: boolean | null, ocrStrategy?: any | null, disableOcr?: boolean | null, resultFormat?: WasmResultFormat | null, outputFormat?: WasmOutputFormat | null, escapeMarkdown?: boolean | null, tableAnchors?: boolean | null, jupyterCellRendering?: WasmJupyterCellRendering | null, useLayoutForMarkdown?: boolean | null, includeDocumentStructure?: boolean | null, url?: WasmUrlExtractionConfig | null, maxArchiveDepth?: number | null, ocr?: WasmOcrConfig | null, forceOcrPages?: Uint32Array | null, chunking?: WasmChunkingConfig | null, contentFilter?: WasmContentFilterConfig | null, images?: WasmImageExtractionConfig | null, tokenReduction?: WasmTokenReductionOptions | null, languageDetection?: WasmLanguageDetectionConfig | null, pages?: WasmPageConfig | null, postprocessor?: WasmPostProcessorConfig | null, extractionTimeoutSecs?: bigint | null, maxConcurrentExtractions?: number | null, securityLimits?: WasmSecurityLimits | null, maxEmbeddedFileBytes?: bigint | null, acceleration?: WasmAccelerationConfig | null, cacheNamespace?: string | null, cacheTtlSecs?: bigint | null, email?: WasmEmailConfig | null, structuredExtraction?: WasmStructuredExtractionConfig | null, ner?: WasmNerConfig | null, redaction?: WasmRedactionConfig | null, summarization?: WasmSummarizationConfig | null, translation?: WasmTranslationConfig | null, pageClassification?: WasmPageClassificationConfig | null, chunkClassification?: WasmChunkClassificationConfig | null, captioning?: WasmCaptioningConfig | null, qrCodes?: boolean | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    get cacheNamespace(): string | undefined;
    set cacheNamespace(value: string | null | undefined);
    get cacheTtlSecs(): bigint | undefined;
    set cacheTtlSecs(value: bigint | null | undefined);
    get captioning(): WasmCaptioningConfig | undefined;
    set captioning(value: WasmCaptioningConfig | null | undefined);
    get chunkClassification(): WasmChunkClassificationConfig | undefined;
    set chunkClassification(value: WasmChunkClassificationConfig | null | undefined);
    get chunking(): WasmChunkingConfig | undefined;
    set chunking(value: WasmChunkingConfig | null | undefined);
    get contentFilter(): WasmContentFilterConfig | undefined;
    set contentFilter(value: WasmContentFilterConfig | null | undefined);
    disableOcr: boolean;
    get email(): WasmEmailConfig | undefined;
    set email(value: WasmEmailConfig | null | undefined);
    enableQualityProcessing: boolean;
    escapeMarkdown: boolean;
    get extractionTimeoutSecs(): bigint | undefined;
    set extractionTimeoutSecs(value: bigint | null | undefined);
    forceOcr: boolean;
    get forceOcrPages(): Uint32Array | undefined;
    set forceOcrPages(value: Uint32Array | null | undefined);
    get images(): WasmImageExtractionConfig | undefined;
    set images(value: WasmImageExtractionConfig | null | undefined);
    includeDocumentStructure: boolean;
    get jupyterCellRendering(): string;
    set jupyterCellRendering(value: WasmJupyterCellRendering);
    get languageDetection(): WasmLanguageDetectionConfig | undefined;
    set languageDetection(value: WasmLanguageDetectionConfig | null | undefined);
    maxArchiveDepth: number;
    get maxConcurrentExtractions(): number | undefined;
    set maxConcurrentExtractions(value: number | null | undefined);
    get maxEmbeddedFileBytes(): bigint | undefined;
    set maxEmbeddedFileBytes(value: bigint | null | undefined);
    get ner(): WasmNerConfig | undefined;
    set ner(value: WasmNerConfig | null | undefined);
    get ocr(): WasmOcrConfig | undefined;
    set ocr(value: WasmOcrConfig | null | undefined);
    ocrStrategy: any;
    get outputFormat(): string;
    set outputFormat(value: WasmOutputFormat);
    get pageClassification(): WasmPageClassificationConfig | undefined;
    set pageClassification(value: WasmPageClassificationConfig | null | undefined);
    get pages(): WasmPageConfig | undefined;
    set pages(value: WasmPageConfig | null | undefined);
    get postprocessor(): WasmPostProcessorConfig | undefined;
    set postprocessor(value: WasmPostProcessorConfig | null | undefined);
    get qrCodes(): boolean | undefined;
    set qrCodes(value: boolean | null | undefined);
    get redaction(): WasmRedactionConfig | undefined;
    set redaction(value: WasmRedactionConfig | null | undefined);
    get resultFormat(): string;
    set resultFormat(value: WasmResultFormat);
    get securityLimits(): WasmSecurityLimits | undefined;
    set securityLimits(value: WasmSecurityLimits | null | undefined);
    get structuredExtraction(): WasmStructuredExtractionConfig | undefined;
    set structuredExtraction(value: WasmStructuredExtractionConfig | null | undefined);
    get summarization(): WasmSummarizationConfig | undefined;
    set summarization(value: WasmSummarizationConfig | null | undefined);
    tableAnchors: boolean;
    get tokenReduction(): WasmTokenReductionOptions | undefined;
    set tokenReduction(value: WasmTokenReductionOptions | null | undefined);
    get translation(): WasmTranslationConfig | undefined;
    set translation(value: WasmTranslationConfig | null | undefined);
    url: WasmUrlExtractionConfig;
    useCache: boolean;
    useLayoutForMarkdown: boolean;
}

/**
 * Non-fatal per-input extraction error captured by `ExtractionResult`.
 */
export class WasmExtractionErrorItem {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractionErrorItem;
    constructor(index: number, code: number, errorType: string, source: string, message: string);
    code: number;
    errorType: string;
    index: number;
    message: string;
    source: string;
}

/**
 * How the extracted text was produced.
 */
export enum WasmExtractionMethod {
    Native = 0,
    Ocr = 1,
    Mixed = 2,
}

/**
 * Unified extraction result envelope.
 */
export class WasmExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractionResult;
    constructor(results?: WasmExtractedDocument[] | null, errors?: WasmExtractionErrorItem[] | null, summary?: WasmExtractionSummary | null, crawlFinalUrls?: string[] | null, crawlRedirectCount?: number | null, crawlUniqueNormalizedUrls?: string[] | null);
    /**
     * Build an output containing one successful result.
     */
    static single(result: WasmExtractedDocument): WasmExtractionResult;
    crawlFinalUrls: string[];
    crawlRedirectCount: number;
    crawlUniqueNormalizedUrls: string[];
    errors: WasmExtractionErrorItem[];
    results: WasmExtractedDocument[];
    summary: WasmExtractionSummary;
}

/**
 * Summary for a unified extraction call.
 */
export class WasmExtractionSummary {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmExtractionSummary;
    constructor(inputs?: number | null, results?: number | null, errors?: number | null, remoteUrls?: number | null, pagesCrawled?: number | null, documentsDownloaded?: number | null);
    documentsDownloaded: number;
    errors: number;
    inputs: number;
    pagesCrawled: number;
    remoteUrls: number;
    results: number;
}

/**
 * FictionBook (FB2) metadata.
 */
export class WasmFictionBookMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFictionBookMetadata;
    constructor(genres?: string[] | null, sequences?: string[] | null, annotation?: string | null);
    get annotation(): string | undefined;
    set annotation(value: string | null | undefined);
    genres: string[];
    sequences: string[];
}

/**
 * Per-file extraction configuration overrides for batch processing.
 *
 * All fields are `Option<T>` — `None` means "use the batch-level default."
 * This type is used by `config` and `extract_batch`
 * to allow heterogeneous extraction settings within a single batch.
 *
 * # Excluded Fields
 *
 * The following `ExtractionConfig` fields are batch-level only and
 * cannot be overridden per file:
 * - `max_concurrent_extractions` — controls batch parallelism
 * - `use_cache` — global caching policy
 * - `acceleration` — shared ONNX execution provider
 * - `security_limits` — global archive security policy
 *
 * # Example
 */
export class WasmFileExtractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFileExtractionConfig;
    constructor(enableQualityProcessing?: boolean | null, ocr?: WasmOcrConfig | null, forceOcr?: boolean | null, ocrStrategy?: any | null, forceOcrPages?: Uint32Array | null, disableOcr?: boolean | null, chunking?: WasmChunkingConfig | null, contentFilter?: WasmContentFilterConfig | null, images?: WasmImageExtractionConfig | null, tokenReduction?: WasmTokenReductionOptions | null, languageDetection?: WasmLanguageDetectionConfig | null, pages?: WasmPageConfig | null, postprocessor?: WasmPostProcessorConfig | null, resultFormat?: WasmResultFormat | null, outputFormat?: WasmOutputFormat | null, includeDocumentStructure?: boolean | null, timeoutSecs?: bigint | null, structuredExtraction?: WasmStructuredExtractionConfig | null, url?: WasmUrlExtractionConfig | null, ner?: WasmNerConfig | null, redaction?: WasmRedactionConfig | null, summarization?: WasmSummarizationConfig | null, translation?: WasmTranslationConfig | null, pageClassification?: WasmPageClassificationConfig | null, chunkClassification?: WasmChunkClassificationConfig | null, captioning?: WasmCaptioningConfig | null, qrCodes?: boolean | null);
    get captioning(): WasmCaptioningConfig | undefined;
    set captioning(value: WasmCaptioningConfig | null | undefined);
    get chunkClassification(): WasmChunkClassificationConfig | undefined;
    set chunkClassification(value: WasmChunkClassificationConfig | null | undefined);
    get chunking(): WasmChunkingConfig | undefined;
    set chunking(value: WasmChunkingConfig | null | undefined);
    get contentFilter(): WasmContentFilterConfig | undefined;
    set contentFilter(value: WasmContentFilterConfig | null | undefined);
    get disableOcr(): boolean | undefined;
    set disableOcr(value: boolean | null | undefined);
    get enableQualityProcessing(): boolean | undefined;
    set enableQualityProcessing(value: boolean | null | undefined);
    get forceOcr(): boolean | undefined;
    set forceOcr(value: boolean | null | undefined);
    get forceOcrPages(): Uint32Array | undefined;
    set forceOcrPages(value: Uint32Array | null | undefined);
    get images(): WasmImageExtractionConfig | undefined;
    set images(value: WasmImageExtractionConfig | null | undefined);
    get includeDocumentStructure(): boolean | undefined;
    set includeDocumentStructure(value: boolean | null | undefined);
    get languageDetection(): WasmLanguageDetectionConfig | undefined;
    set languageDetection(value: WasmLanguageDetectionConfig | null | undefined);
    get ner(): WasmNerConfig | undefined;
    set ner(value: WasmNerConfig | null | undefined);
    get ocr(): WasmOcrConfig | undefined;
    set ocr(value: WasmOcrConfig | null | undefined);
    get ocrStrategy(): any | undefined;
    set ocrStrategy(value: any | null | undefined);
    get outputFormat(): string | undefined;
    set outputFormat(value: WasmOutputFormat | null | undefined);
    get pageClassification(): WasmPageClassificationConfig | undefined;
    set pageClassification(value: WasmPageClassificationConfig | null | undefined);
    get pages(): WasmPageConfig | undefined;
    set pages(value: WasmPageConfig | null | undefined);
    get postprocessor(): WasmPostProcessorConfig | undefined;
    set postprocessor(value: WasmPostProcessorConfig | null | undefined);
    get qrCodes(): boolean | undefined;
    set qrCodes(value: boolean | null | undefined);
    get redaction(): WasmRedactionConfig | undefined;
    set redaction(value: WasmRedactionConfig | null | undefined);
    get resultFormat(): string | undefined;
    set resultFormat(value: WasmResultFormat | null | undefined);
    get structuredExtraction(): WasmStructuredExtractionConfig | undefined;
    set structuredExtraction(value: WasmStructuredExtractionConfig | null | undefined);
    get summarization(): WasmSummarizationConfig | undefined;
    set summarization(value: WasmSummarizationConfig | null | undefined);
    get timeoutSecs(): bigint | undefined;
    set timeoutSecs(value: bigint | null | undefined);
    get tokenReduction(): WasmTokenReductionOptions | undefined;
    set tokenReduction(value: WasmTokenReductionOptions | null | undefined);
    get translation(): WasmTranslationConfig | undefined;
    set translation(value: WasmTranslationConfig | null | undefined);
    get url(): WasmUrlExtractionConfig | undefined;
    set url(value: WasmUrlExtractionConfig | null | undefined);
}

/**
 * Footnote in Djot.
 */
export class WasmFootnote {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFootnote;
    constructor(label: string, content: WasmFormattedBlock[]);
    content: WasmFormattedBlock[];
    label: string;
}

/**
 * Kind of a PDF form field.
 *
 * Mirrors `pdf_oxide`'s widget field taxonomy without leaking the upstream
 * type across the binding surface.
 */
export enum WasmFormFieldType {
    Text = 0,
    Checkbox = 1,
    Radio = 2,
    Choice = 3,
    Signature = 4,
    Button = 5,
    Unknown = 6,
}

/**
 * Format-specific metadata (discriminated union).
 *
 * Only one format type can exist per extraction result. This provides
 * type-safe, clean metadata without nested optionals.
 */
export class WasmFormatMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFormatMetadata;
    constructor();
    get 0(): any | undefined;
    set 0(value: any | null | undefined);
    formatType: string;
}

/**
 * Block-level element in a Djot document.
 *
 * Represents structural elements like headings, paragraphs, lists, code blocks, etc.
 */
export class WasmFormattedBlock {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFormattedBlock;
    constructor(blockType: WasmBlockType, inlineContent: WasmInlineElement[], children: WasmFormattedBlock[], level?: number | null, language?: string | null, code?: string | null);
    get blockType(): string;
    set blockType(value: WasmBlockType);
    children: WasmFormattedBlock[];
    get code(): string | undefined;
    set code(value: string | null | undefined);
    inlineContent: WasmInlineElement[];
    get language(): string | undefined;
    set language(value: string | null | undefined);
    get level(): number | undefined;
    set level(value: number | null | undefined);
}

/**
 * A mathematical formula detected and recognized in a document.
 *
 * Populated by the layout-guided formula pipeline: regions classified as
 * `LayoutClass.Formula` are routed to the formula OCR task, which returns the
 * LaTeX source for the region. The field is always present on
 * `ExtractedDocument` but only populated
 * when the `layout-detection` feature is active and the document contains
 * formula regions.
 */
export class WasmFormula {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmFormula;
    constructor(latex: string, bbox: WasmBoundingBox, page: number);
    bbox: WasmBoundingBox;
    latex: string;
    page: number;
}

/**
 * Individual grid cell with position and span metadata.
 */
export class WasmGridCell {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmGridCell;
    constructor(content: string, row: number, col: number, rowSpan: number, colSpan: number, isHeader: boolean, bbox?: WasmBoundingBox | null);
    get bbox(): WasmBoundingBox | undefined;
    set bbox(value: WasmBoundingBox | null | undefined);
    col: number;
    colSpan: number;
    content: string;
    isHeader: boolean;
    row: number;
    rowSpan: number;
}

/**
 * Header/heading element metadata.
 */
export class WasmHeaderMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmHeaderMetadata;
    constructor(level: number, text: string, depth: number, htmlOffset: number, id?: string | null);
    depth: number;
    htmlOffset: number;
    get id(): string | undefined;
    set id(value: string | null | undefined);
    level: number;
    text: string;
}

/**
 * Heading context for a chunk within a Markdown document.
 *
 * Contains the heading hierarchy from document root to this chunk's section.
 */
export class WasmHeadingContext {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmHeadingContext;
    constructor(headings: WasmHeadingLevel[]);
    headings: WasmHeadingLevel[];
}

/**
 * A single heading in the hierarchy.
 */
export class WasmHeadingLevel {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmHeadingLevel;
    constructor(level: number, text: string);
    level: number;
    text: string;
}

/**
 * Heading style options for Markdown output.
 *
 * Controls how headings (h1-h6) are rendered in the output Markdown.
 */
export enum WasmHeadingStyle {
    Underlined = 0,
    Atx = 1,
    AtxClosed = 2,
}

/**
 * A text block with hierarchy level assignment.
 *
 * Represents a block of text with semantic heading information extracted from
 * font size clustering and hierarchical analysis.
 */
export class WasmHierarchicalBlock {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmHierarchicalBlock;
    constructor(text: string, fontSize: number, level: string);
    fontSize: number;
    level: string;
    text: string;
}

/**
 * Highlight rendering style for `<mark>` elements.
 *
 * Controls how highlighted text is rendered in Markdown output.
 */
export enum WasmHighlightStyle {
    DoubleEqual = 0,
    Html = 1,
    Bold = 2,
    None = 3,
}

/**
 * HTML metadata extracted from HTML documents.
 *
 * Includes document-level metadata, Open Graph data, Twitter Card metadata,
 * and extracted structural elements (headers, links, images, structured data).
 */
export class WasmHtmlMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmHtmlMetadata;
    constructor(keywords?: string[] | null, openGraph?: any | null, twitterCard?: any | null, metaTags?: any | null, headers?: WasmHeaderMetadata[] | null, links?: WasmLinkMetadata[] | null, images?: WasmImageMetadataType[] | null, structuredData?: WasmStructuredData[] | null, title?: string | null, description?: string | null, author?: string | null, canonicalUrl?: string | null, baseHref?: string | null, language?: string | null, textDirection?: WasmTextDirection | null);
    get author(): string | undefined;
    set author(value: string | null | undefined);
    get baseHref(): string | undefined;
    set baseHref(value: string | null | undefined);
    get canonicalUrl(): string | undefined;
    set canonicalUrl(value: string | null | undefined);
    get description(): string | undefined;
    set description(value: string | null | undefined);
    headers: WasmHeaderMetadata[];
    images: WasmImageMetadataType[];
    keywords: string[];
    get language(): string | undefined;
    set language(value: string | null | undefined);
    links: WasmLinkMetadata[];
    metaTags: any;
    openGraph: any;
    structuredData: WasmStructuredData[];
    get textDirection(): string | undefined;
    set textDirection(value: WasmTextDirection | null | undefined);
    get title(): string | undefined;
    set title(value: string | null | undefined);
    twitterCard: any;
}

/**
 * Image extraction configuration.
 */
export class WasmImageExtractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImageExtractionConfig;
    constructor(extractImages?: boolean | null, targetDpi?: number | null, maxImageDimension?: number | null, injectPlaceholders?: boolean | null, autoAdjustDpi?: boolean | null, minDpi?: number | null, maxDpi?: number | null, classify?: boolean | null, includePageRasters?: boolean | null, runOcrOnImages?: boolean | null, ocrTextOnly?: boolean | null, appendOcrText?: boolean | null, outputFormat?: any | null, includeDataBase64?: boolean | null, maxImagesPerPage?: number | null);
    appendOcrText: boolean;
    autoAdjustDpi: boolean;
    classify: boolean;
    extractImages: boolean;
    includeDataBase64: boolean;
    includePageRasters: boolean;
    injectPlaceholders: boolean;
    maxDpi: number;
    maxImageDimension: number;
    get maxImagesPerPage(): number | undefined;
    set maxImagesPerPage(value: number | null | undefined);
    minDpi: number;
    ocrTextOnly: boolean;
    outputFormat: any;
    runOcrOnImages: boolean;
    targetDpi: number;
}

/**
 * Heuristic classification of what an image likely depicts.
 */
export enum WasmImageKind {
    Photograph = 0,
    Diagram = 1,
    Chart = 2,
    Drawing = 3,
    TextBlock = 4,
    Decoration = 5,
    Logo = 6,
    Icon = 7,
    TileFragment = 8,
    Mask = 9,
    PageRaster = 10,
    Unknown = 11,
}

/**
 * Image metadata extracted from image files.
 *
 * Includes dimensions, format, and EXIF data.
 */
export class WasmImageMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImageMetadata;
    constructor(width?: number | null, height?: number | null, format?: string | null, exif?: any | null);
    exif: any;
    format: string;
    height: number;
    width: number;
}

/**
 * Image element metadata.
 */
export class WasmImageMetadataType {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImageMetadataType;
    constructor(src: string, imageType: WasmImageType, alt?: string | null, title?: string | null);
    get alt(): string | undefined;
    set alt(value: string | null | undefined);
    get imageType(): string;
    set imageType(value: WasmImageType);
    src: string;
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * Target format for re-encoding extracted images.
 *
 * Controls whether and how extracted images are normalised to a uniform
 * container format before being returned in `ExtractedDocument.images`.
 * The default (`Native`) preserves the format produced by each extractor
 * without any additional encode pass.
 *
 * Callers that need uniform output — e.g. cloud pipelines that always store
 * WebP thumbnails — set this once on `ImageExtractionConfig.output_format`
 * rather than re-encoding downstream.
 *
 * # Serde shape
 *
 * Uses a tagged enum: `{"type": "native"}`, `{"type": "png"}`,
 * `{"type": "jpeg", "quality": 90}`, etc.
 */
export class WasmImageOutputFormat {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImageOutputFormat;
    constructor();
    get quality(): number | undefined;
    set quality(value: number | null | undefined);
    type: string;
}

/**
 * Image preprocessing configuration for OCR.
 *
 * These settings control how images are preprocessed before OCR to improve
 * text recognition quality. Different preprocessing strategies work better
 * for different document types.
 */
export class WasmImagePreprocessingConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImagePreprocessingConfig;
    constructor(targetDpi?: number | null, autoRotate?: boolean | null, deskew?: boolean | null, denoise?: boolean | null, contrastEnhance?: boolean | null, binarizationMethod?: string | null, invertColors?: boolean | null);
    autoRotate: boolean;
    binarizationMethod: string;
    contrastEnhance: boolean;
    denoise: boolean;
    deskew: boolean;
    invertColors: boolean;
    targetDpi: number;
}

/**
 * Image preprocessing metadata.
 *
 * Tracks the transformations applied to an image during OCR preprocessing,
 * including DPI normalization, resizing, and resampling.
 */
export class WasmImagePreprocessingMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmImagePreprocessingMetadata;
    constructor(targetDpi: number, scaleFactor: number, autoAdjusted: boolean, finalDpi: number, resampleMethod: string, dimensionClamped: boolean, skippedResize: boolean, calculatedDpi?: number | null, resizeError?: string | null);
    autoAdjusted: boolean;
    get calculatedDpi(): number | undefined;
    set calculatedDpi(value: number | null | undefined);
    dimensionClamped: boolean;
    finalDpi: number;
    resampleMethod: string;
    get resizeError(): string | undefined;
    set resizeError(value: string | null | undefined);
    scaleFactor: number;
    skippedResize: boolean;
    targetDpi: number;
}

/**
 * Image type classification.
 */
export enum WasmImageType {
    DataUri = 0,
    InlineSvg = 1,
    External = 2,
    Relative = 3,
}

/**
 * Inline element within a block.
 *
 * Represents text with formatting, links, images, etc.
 */
export class WasmInlineElement {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmInlineElement;
    constructor(elementType: WasmInlineType, content: string, metadata?: any | null);
    content: string;
    get elementType(): string;
    set elementType(value: WasmInlineType);
    get metadata(): any | undefined;
    set metadata(value: any | null | undefined);
}

/**
 * Types of inline elements in Djot.
 */
export enum WasmInlineType {
    Text = 0,
    Strong = 1,
    Emphasis = 2,
    Highlight = 3,
    Subscript = 4,
    Superscript = 5,
    Insert = 6,
    Delete = 7,
    Code = 8,
    Link = 9,
    Image = 10,
    Span = 11,
    Math = 12,
    RawInline = 13,
    FootnoteRef = 14,
    Symbol = 15,
}

/**
 * JATS (Journal Article Tag Suite) metadata.
 */
export class WasmJatsMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmJatsMetadata;
    constructor(historyDates?: any | null, contributorRoles?: WasmContributorRole[] | null, copyright?: string | null, license?: string | null);
    contributorRoles: WasmContributorRole[];
    get copyright(): string | undefined;
    set copyright(value: string | null | undefined);
    historyDates: any;
    get license(): string | undefined;
    set license(value: string | null | undefined);
}

/**
 * Controls how Jupyter notebook code cells are rendered during extraction.
 *
 * A code cell carries both its **source** and any **outputs** that were saved in
 * the notebook. Callers ingesting notebooks for AI agents want different slices of
 * this depending on the task. Xberg never executes cells — `Outputs` and `Both`
 * only surface outputs already stored in the `.ipynb`.
 *
 * This toggle governs a code cell's **source body** and its **saved outputs**.
 * Markdown (prose) cells and structural markers (kernel language, cell id, tags,
 * execution count) are unaffected — prose always renders and markers orient the
 * reader regardless of mode.
 */
export enum WasmJupyterCellRendering {
    Source = 0,
    Outputs = 1,
    Both = 2,
}

/**
 * Language detection configuration.
 */
export class WasmLanguageDetectionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLanguageDetectionConfig;
    constructor(enabled?: boolean | null, minConfidence?: number | null, detectMultiple?: boolean | null);
    detectMultiple: boolean;
    enabled: boolean;
    minConfidence: number;
}

/**
 * Configuration for the late-interaction (ColBERT) pipeline.
 *
 * Controls which model to use, batching, and download/cache behavior for the
 * local ONNX ColBERT model.
 *
 * Since v5.0.0.
 */
export class WasmLateInteractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLateInteractionConfig;
    constructor(model?: any | null, batchSize?: number | null, maxLength?: number | null, queryMaxLength?: number | null, showDownloadProgress?: boolean | null, cacheDir?: string | null, acceleration?: WasmAccelerationConfig | null, maxEmbedDurationSecs?: bigint | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    batchSize: number;
    get cacheDir(): string | undefined;
    set cacheDir(value: string | null | undefined);
    get maxEmbedDurationSecs(): bigint | undefined;
    set maxEmbedDurationSecs(value: bigint | null | undefined);
    maxLength: number;
    model: any;
    queryMaxLength: number;
    showDownloadProgress: boolean;
}

/**
 * Late-interaction model types supported by Xberg.
 *
 * Since v5.0.0.
 */
export class WasmLateInteractionModelType {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLateInteractionModelType;
    constructor();
    get additionalFiles(): string[] | undefined;
    set additionalFiles(value: string[] | null | undefined);
    get maxLength(): bigint | undefined;
    set maxLength(value: bigint | null | undefined);
    get modelFile(): string | undefined;
    set modelFile(value: string | null | undefined);
    get modelId(): string | undefined;
    set modelId(value: string | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    type: string;
}

/**
 * A detected layout region on a page.
 *
 * When layout detection is enabled, each page may have layout regions
 * identifying different content types (text, pictures, tables, etc.)
 * with confidence scores and spatial positions.
 */
export class WasmLayoutRegion {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLayoutRegion;
    constructor(className?: string | null, confidence?: number | null, boundingBox?: WasmBoundingBox | null, areaFraction?: number | null);
    areaFraction: number;
    boundingBox: WasmBoundingBox;
    className: string;
    confidence: number;
}

/**
 * Link element metadata.
 */
export class WasmLinkMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLinkMetadata;
    constructor(href: string, text: string, linkType: WasmLinkType, rel: string[], title?: string | null);
    href: string;
    get linkType(): string;
    set linkType(value: WasmLinkType);
    rel: string[];
    text: string;
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * Link rendering style in Markdown output.
 *
 * Controls whether links and images use inline `[text](url)` syntax or
 * reference-style `[text][1]` syntax with definitions collected at the end.
 */
export enum WasmLinkStyle {
    Inline = 0,
    Reference = 1,
}

/**
 * Link type classification.
 */
export enum WasmLinkType {
    Anchor = 0,
    Internal = 1,
    External = 2,
    Email = 3,
    Phone = 4,
    Other = 5,
}

/**
 * List indentation character type.
 *
 * Controls whether list items are indented with spaces or tabs.
 */
export enum WasmListIndentType {
    Spaces = 0,
    Tabs = 1,
}

/**
 * Type of list detection.
 */
export enum WasmListType {
    Bullet = 0,
    Numbered = 1,
    Lettered = 2,
    Indented = 3,
}

/**
 * Configuration for an LLM provider/model via liter-llm.
 *
 * Each feature (VLM OCR, VLM embeddings, structured extraction) carries
 * its own `LlmConfig`, allowing different providers per feature.
 *
 * # Example
 *
 * ```toml
 * [structured_extraction.llm]
 * model = "openai/gpt-4o"
 * api_key = "sk-..."  # or use XBERG_LLM_API_KEY env var
 * ```
 */
export class WasmLlmConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLlmConfig;
    constructor(model?: string | null, apiKey?: string | null, baseUrl?: string | null, timeoutSecs?: bigint | null, maxRetries?: number | null, temperature?: number | null, maxTokens?: bigint | null, loadEnv?: boolean | null, headers?: any | null);
    get apiKey(): string | undefined;
    set apiKey(value: string | null | undefined);
    get baseUrl(): string | undefined;
    set baseUrl(value: string | null | undefined);
    get headers(): any | undefined;
    set headers(value: any | null | undefined);
    get loadEnv(): boolean | undefined;
    set loadEnv(value: boolean | null | undefined);
    get maxRetries(): number | undefined;
    set maxRetries(value: number | null | undefined);
    get maxTokens(): bigint | undefined;
    set maxTokens(value: bigint | null | undefined);
    model: string;
    get temperature(): number | undefined;
    set temperature(value: number | null | undefined);
    get timeoutSecs(): bigint | undefined;
    set timeoutSecs(value: bigint | null | undefined);
}

/**
 * Token usage and cost data for a single LLM call made during extraction.
 *
 * Populated when VLM OCR, structured extraction, or LLM-based embeddings
 * are used. Multiple entries may be present when multiple LLM calls occur
 * within one extraction (e.g. VLM OCR + structured extraction).
 */
export class WasmLlmUsage {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmLlmUsage;
    constructor(model?: string | null, source?: string | null, inputTokens?: bigint | null, outputTokens?: bigint | null, totalTokens?: bigint | null, estimatedCost?: number | null, finishReason?: string | null);
    get estimatedCost(): number | undefined;
    set estimatedCost(value: number | null | undefined);
    get finishReason(): string | undefined;
    set finishReason(value: string | null | undefined);
    get inputTokens(): bigint | undefined;
    set inputTokens(value: bigint | null | undefined);
    model: string;
    get outputTokens(): bigint | undefined;
    set outputTokens(value: bigint | null | undefined);
    source: string;
    get totalTokens(): bigint | undefined;
    set totalTokens(value: bigint | null | undefined);
}

/**
 * The result of a map operation, containing discovered URLs.
 */
export class WasmMapResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmMapResult;
    constructor(urls?: WasmSitemapUrl[] | null);
    urls: WasmSitemapUrl[];
}

/**
 * How partial results from multiple model calls (e.g. per page batch) are combined.
 *
 * Canonical home for the merge strategy referenced by presets and by the
 * structured pipeline's post-processing. There is intentionally only one merge
 * type across the crate — do not introduce a second.
 */
export enum WasmMergeMode {
    ObjectMerge = 0,
    ArrayConcat = 1,
    ObjectFirst = 2,
}

/**
 * Extraction result metadata.
 *
 * Contains common fields applicable to all formats, format-specific metadata
 * via a discriminated union, and additional custom fields from postprocessors.
 */
export class WasmMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmMetadata;
    /**
     * Returns `true` when no metadata fields, format-specific metadata, or
     * additional postprocessor fields are populated.
     */
    isEmpty(): boolean;
    constructor(ocrUsed?: boolean | null, additional?: any | null, title?: string | null, subject?: string | null, authors?: string[] | null, keywords?: string[] | null, language?: string | null, createdAt?: string | null, modifiedAt?: string | null, createdBy?: string | null, modifiedBy?: string | null, pages?: WasmPageStructure | null, format?: any | null, imagePreprocessing?: WasmImagePreprocessingMetadata | null, jsonSchema?: any | null, error?: WasmErrorMetadata | null, extractionDurationMs?: bigint | null, category?: string | null, tags?: string[] | null, documentVersion?: string | null, abstractText?: string | null, outputFormat?: string | null);
    get abstractText(): string | undefined;
    set abstractText(value: string | null | undefined);
    additional: any;
    get authors(): string[] | undefined;
    set authors(value: string[] | null | undefined);
    get category(): string | undefined;
    set category(value: string | null | undefined);
    get createdAt(): string | undefined;
    set createdAt(value: string | null | undefined);
    get createdBy(): string | undefined;
    set createdBy(value: string | null | undefined);
    get documentVersion(): string | undefined;
    set documentVersion(value: string | null | undefined);
    get error(): WasmErrorMetadata | undefined;
    set error(value: WasmErrorMetadata | null | undefined);
    get extractionDurationMs(): bigint | undefined;
    set extractionDurationMs(value: bigint | null | undefined);
    get format(): any | undefined;
    set format(value: any | null | undefined);
    get imagePreprocessing(): WasmImagePreprocessingMetadata | undefined;
    set imagePreprocessing(value: WasmImagePreprocessingMetadata | null | undefined);
    get jsonSchema(): any | undefined;
    set jsonSchema(value: any | null | undefined);
    get keywords(): string[] | undefined;
    set keywords(value: string[] | null | undefined);
    get language(): string | undefined;
    set language(value: string | null | undefined);
    get modifiedAt(): string | undefined;
    set modifiedAt(value: string | null | undefined);
    get modifiedBy(): string | undefined;
    set modifiedBy(value: string | null | undefined);
    ocrUsed: boolean;
    get outputFormat(): string | undefined;
    set outputFormat(value: string | null | undefined);
    get pages(): WasmPageStructure | undefined;
    set pages(value: WasmPageStructure | null | undefined);
    get subject(): string | undefined;
    set subject(value: string | null | undefined);
    get tags(): string[] | undefined;
    set tags(value: string[] | null | undefined);
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * NER backend selector.
 */
export enum WasmNerBackendKind {
    Onnx = 0,
    Llm = 1,
}

/**
 * Configuration for the NER post-processor.
 */
export class WasmNerConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmNerConfig;
    constructor(backend?: WasmNerBackendKind | null, categories?: any[] | null, customLabels?: string[] | null, model?: string | null, llm?: WasmLlmConfig | null);
    get backend(): string;
    set backend(value: WasmNerBackendKind);
    categories: string[];
    customLabels: string[];
    get llm(): WasmLlmConfig | undefined;
    set llm(value: WasmLlmConfig | null | undefined);
    get model(): string | undefined;
    set model(value: string | null | undefined);
}

/**
 * Line break syntax in Markdown output.
 *
 * Controls how soft line breaks (from `<br>` or line breaks in source) are rendered.
 */
export enum WasmNewlineStyle {
    Spaces = 0,
    Backslash = 1,
}

/**
 * Tagged enum for node content. Each variant carries only type-specific data.
 *
 * Uses `#[serde(tag = "node_type")]` to avoid "type" keyword collision in
 * Go/Java/TypeScript bindings.
 */
export class WasmNodeContent {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmNodeContent;
    constructor();
    get content(): string | undefined;
    set content(value: string | null | undefined);
    get definition(): string | undefined;
    set definition(value: string | null | undefined);
    get description(): string | undefined;
    set description(value: string | null | undefined);
    get entries(): any | undefined;
    set entries(value: any | null | undefined);
    get format(): string | undefined;
    set format(value: string | null | undefined);
    get grid(): WasmTableGrid | undefined;
    set grid(value: WasmTableGrid | null | undefined);
    get headingLevel(): number | undefined;
    set headingLevel(value: number | null | undefined);
    get headingText(): string | undefined;
    set headingText(value: string | null | undefined);
    get imageIndex(): number | undefined;
    set imageIndex(value: number | null | undefined);
    get key(): string | undefined;
    set key(value: string | null | undefined);
    get kind(): string | undefined;
    set kind(value: string | null | undefined);
    get label(): string | undefined;
    set label(value: string | null | undefined);
    get language(): string | undefined;
    set language(value: string | null | undefined);
    get level(): number | undefined;
    set level(value: number | null | undefined);
    nodeType: string;
    get number(): number | undefined;
    set number(value: number | null | undefined);
    get ordered(): boolean | undefined;
    set ordered(value: boolean | null | undefined);
    get src(): string | undefined;
    set src(value: string | null | undefined);
    get term(): string | undefined;
    set term(value: string | null | undefined);
    get text(): string | undefined;
    set text(value: string | null | undefined);
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * OCR backend types.
 */
export enum WasmOcrBackendType {
    Tesseract = 0,
    PaddleOCR = 1,
    Candle = 2,
    Custom = 3,
}

/**
 * Bounding geometry for an OCR element.
 *
 * Supports both axis-aligned rectangles (from Tesseract) and 4-point quadrilaterals
 * (from PaddleOCR and rotated text detection).
 */
export class WasmOcrBoundingGeometry {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrBoundingGeometry;
    constructor();
    get height(): number | undefined;
    set height(value: number | null | undefined);
    get left(): number | undefined;
    set left(value: number | null | undefined);
    get points(): any | undefined;
    set points(value: any | null | undefined);
    get top(): number | undefined;
    set top(value: number | null | undefined);
    type: string;
    get width(): number | undefined;
    set width(value: number | null | undefined);
}

/**
 * Confidence scores for an OCR element.
 *
 * Separates detection confidence (how confident that text exists at this location)
 * from recognition confidence (how confident about the actual text content).
 */
export class WasmOcrConfidence {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrConfidence;
    constructor(recognition?: number | null, detection?: number | null);
    get detection(): number | undefined;
    set detection(value: number | null | undefined);
    recognition: number;
}

/**
 * OCR configuration.
 */
export class WasmOcrConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrConfig;
    constructor(enabled?: boolean | null, backend?: string | null, language?: string[] | null, autoRotate?: boolean | null, vlmFallback?: any | null, tesseractConfig?: WasmTesseractConfig | null, outputFormat?: WasmOutputFormat | null, paddleOcrConfig?: any | null, backendOptions?: any | null, elementConfig?: WasmOcrElementConfig | null, qualityThresholds?: WasmOcrQualityThresholds | null, pipeline?: WasmOcrPipelineConfig | null, vlmConfig?: WasmLlmConfig | null, vlmPrompt?: string | null, acceleration?: WasmAccelerationConfig | null, tessdataBytes?: any | null, tessdataPath?: string | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    autoRotate: boolean;
    backend: string;
    get backendOptions(): any | undefined;
    set backendOptions(value: any | null | undefined);
    get elementConfig(): WasmOcrElementConfig | undefined;
    set elementConfig(value: WasmOcrElementConfig | null | undefined);
    enabled: boolean;
    language: string[];
    get outputFormat(): string | undefined;
    set outputFormat(value: WasmOutputFormat | null | undefined);
    get paddleOcrConfig(): any | undefined;
    set paddleOcrConfig(value: any | null | undefined);
    get pipeline(): WasmOcrPipelineConfig | undefined;
    set pipeline(value: WasmOcrPipelineConfig | null | undefined);
    get qualityThresholds(): WasmOcrQualityThresholds | undefined;
    set qualityThresholds(value: WasmOcrQualityThresholds | null | undefined);
    get tessdataBytes(): any | undefined;
    set tessdataBytes(value: any | null | undefined);
    get tessdataPath(): string | undefined;
    set tessdataPath(value: string | null | undefined);
    get tesseractConfig(): WasmTesseractConfig | undefined;
    set tesseractConfig(value: WasmTesseractConfig | null | undefined);
    get vlmConfig(): WasmLlmConfig | undefined;
    set vlmConfig(value: WasmLlmConfig | null | undefined);
    vlmFallback: any;
    get vlmPrompt(): string | undefined;
    set vlmPrompt(value: string | null | undefined);
}

/**
 * A unified OCR element representing detected text with full metadata.
 *
 * This is the primary type for structured OCR output, preserving all information
 * from both Tesseract and PaddleOCR backends.
 */
export class WasmOcrElement {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrElement;
    constructor(text?: string | null, geometry?: any | null, confidence?: WasmOcrConfidence | null, level?: WasmOcrElementLevel | null, pageNumber?: number | null, backendMetadata?: any | null, rotation?: WasmOcrRotation | null, parentId?: string | null);
    backendMetadata: any;
    confidence: WasmOcrConfidence;
    geometry: any;
    get level(): string;
    set level(value: WasmOcrElementLevel);
    pageNumber: number;
    get parentId(): string | undefined;
    set parentId(value: string | null | undefined);
    get rotation(): WasmOcrRotation | undefined;
    set rotation(value: WasmOcrRotation | null | undefined);
    text: string;
}

/**
 * Configuration for OCR element extraction.
 *
 * Controls how OCR elements are extracted and filtered.
 */
export class WasmOcrElementConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrElementConfig;
    constructor(includeElements?: boolean | null, minLevel?: WasmOcrElementLevel | null, minConfidence?: number | null, buildHierarchy?: boolean | null);
    buildHierarchy: boolean;
    includeElements: boolean;
    minConfidence: number;
    get minLevel(): string;
    set minLevel(value: WasmOcrElementLevel);
}

/**
 * Hierarchical level of an OCR element.
 *
 * Maps to Tesseract's page segmentation hierarchy and provides
 * equivalent semantics for PaddleOCR.
 */
export enum WasmOcrElementLevel {
    Word = 0,
    Line = 1,
    Block = 2,
    Page = 3,
}

/**
 * OCR extraction result.
 *
 * Result of performing OCR on an image or scanned document,
 * including recognized text and detected tables.
 */
export class WasmOcrExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrExtractionResult;
    constructor(content?: string | null, mimeType?: string | null, metadata?: any | null, tables?: WasmOcrTable[] | null, ocrElements?: WasmOcrElement[] | null);
    content: string;
    metadata: any;
    mimeType: string;
    get ocrElements(): Array<any> | undefined;
    set ocrElements(value: WasmOcrElement[] | null | undefined);
    tables: WasmOcrTable[];
}

/**
 * OCR processing metadata.
 *
 * Captures information about OCR processing configuration and results.
 */
export class WasmOcrMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrMetadata;
    constructor(language?: string | null, psm?: number | null, outputFormat?: string | null, tableCount?: number | null, tableRows?: number | null, tableCols?: number | null);
    language: string;
    outputFormat: string;
    psm: number;
    get tableCols(): number | undefined;
    set tableCols(value: number | null | undefined);
    tableCount: number;
    get tableRows(): number | undefined;
    set tableRows(value: number | null | undefined);
}

/**
 * Multi-backend OCR pipeline with quality-based fallback.
 *
 * Backends are tried in priority order (highest first). After each backend
 * produces output, quality is evaluated. If it meets `quality_thresholds.pipeline_min_quality`,
 * the result is accepted. Otherwise the next backend is tried.
 */
export class WasmOcrPipelineConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrPipelineConfig;
    constructor(stages: WasmOcrPipelineStage[], qualityThresholds: WasmOcrQualityThresholds);
    qualityThresholds: WasmOcrQualityThresholds;
    stages: WasmOcrPipelineStage[];
}

/**
 * A single backend stage in the OCR pipeline.
 */
export class WasmOcrPipelineStage {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrPipelineStage;
    constructor(backend: string, priority: number, language?: string[] | null, tesseractConfig?: WasmTesseractConfig | null, paddleOcrConfig?: any | null, vlmConfig?: WasmLlmConfig | null, backendOptions?: any | null);
    backend: string;
    get backendOptions(): any | undefined;
    set backendOptions(value: any | null | undefined);
    get language(): string[] | undefined;
    set language(value: string[] | null | undefined);
    get paddleOcrConfig(): any | undefined;
    set paddleOcrConfig(value: any | null | undefined);
    priority: number;
    get tesseractConfig(): WasmTesseractConfig | undefined;
    set tesseractConfig(value: WasmTesseractConfig | null | undefined);
    get vlmConfig(): WasmLlmConfig | undefined;
    set vlmConfig(value: WasmLlmConfig | null | undefined);
}

/**
 * Quality thresholds for OCR fallback decisions and pipeline quality gating.
 *
 * All fields default to the values that match the previous hardcoded behavior,
 * so `OcrQualityThresholds.default()` preserves existing semantics exactly.
 */
export class WasmOcrQualityThresholds {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrQualityThresholds;
    constructor(minTotalNonWhitespace?: number | null, minNonWhitespacePerPage?: number | null, minMeaningfulWordLen?: number | null, minMeaningfulWords?: number | null, minAlnumRatio?: number | null, minGarbageChars?: number | null, maxFragmentedWordRatio?: number | null, criticalFragmentedWordRatio?: number | null, minAvgWordLength?: number | null, minWordsForAvgLengthCheck?: number | null, minConsecutiveRepeatRatio?: number | null, minWordsForRepeatCheck?: number | null, substantiveMinChars?: number | null, nonTextMinChars?: number | null, alnumWsRatioThreshold?: number | null, pipelineMinQuality?: number | null, minUndecodableRatio?: number | null, enableProvenanceOcrRouting?: boolean | null, minProvenanceFallbackRatio?: number | null);
    alnumWsRatioThreshold: number;
    criticalFragmentedWordRatio: number;
    enableProvenanceOcrRouting: boolean;
    maxFragmentedWordRatio: number;
    minAlnumRatio: number;
    minAvgWordLength: number;
    minConsecutiveRepeatRatio: number;
    minGarbageChars: number;
    minMeaningfulWordLen: number;
    minMeaningfulWords: number;
    minNonWhitespacePerPage: number;
    minProvenanceFallbackRatio: number;
    minTotalNonWhitespace: number;
    minUndecodableRatio: number;
    minWordsForAvgLengthCheck: number;
    minWordsForRepeatCheck: number;
    nonTextMinChars: number;
    pipelineMinQuality: number;
    substantiveMinChars: number;
}

/**
 * Rotation information for an OCR element.
 */
export class WasmOcrRotation {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrRotation;
    constructor(angleDegrees: number, confidence?: number | null);
    angleDegrees: number;
    get confidence(): number | undefined;
    set confidence(value: number | null | undefined);
}

/**
 * Which pages of a PDF get OCR'd when neither `force_ocr` nor `force_ocr_pages` applies.
 *
 * # Examples
 */
export class WasmOcrStrategy {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrStrategy;
    constructor();
    get minConfidence(): number | undefined;
    set minConfidence(value: number | null | undefined);
    mode: string;
}

/**
 * Table detected via OCR.
 *
 * Represents a table structure recognized during OCR processing.
 */
export class WasmOcrTable {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrTable;
    constructor(cells: any, markdown: string, pageNumber: number, boundingBox?: WasmOcrTableBoundingBox | null);
    get boundingBox(): WasmOcrTableBoundingBox | undefined;
    set boundingBox(value: WasmOcrTableBoundingBox | null | undefined);
    cells: any;
    markdown: string;
    pageNumber: number;
}

/**
 * Bounding box for an OCR-detected table in pixel coordinates.
 */
export class WasmOcrTableBoundingBox {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmOcrTableBoundingBox;
    constructor(left: number, top: number, right: number, bottom: number);
    bottom: number;
    left: number;
    right: number;
    top: number;
}

/**
 * Output format for extraction results.
 *
 * Controls the format of the `content` field in `ExtractedDocument`.
 * When set to `Markdown`, `Djot`, or `Html`, the output uses that format.
 * `Plain` returns the raw extracted text.
 * `Structured` returns JSON with full OCR element data including bounding
 * boxes and confidence scores.
 */
export enum WasmOutputFormat {
    Plain = 0,
    Markdown = 1,
    Djot = 2,
    Html = 3,
    Json = 4,
    Structured = 5,
    Custom = 6,
}

/**
 * Byte offset boundary for a page.
 *
 * Tracks where a specific page's content starts and ends in the main content string,
 * enabling mapping from byte positions to page numbers. Offsets are guaranteed to be
 * at valid UTF-8 character boundaries when using standard String methods (push_str, push, etc.).
 */
export class WasmPageBoundary {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageBoundary;
    constructor(byteStart: number, byteEnd: number, pageNumber: number);
    byteEnd: number;
    byteStart: number;
    pageNumber: number;
}

/**
 * Classification result for a single page.
 */
export class WasmPageClassification {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageClassification;
    constructor(pageNumber: number, labels: WasmClassificationLabel[]);
    labels: WasmClassificationLabel[];
    pageNumber: number;
}

/**
 * Configuration for the page-classification post-processor.
 */
export class WasmPageClassificationConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageClassificationConfig;
    constructor(labels: string[], multiLabel: boolean, llm: WasmLlmConfig, promptTemplate?: string | null);
    labels: string[];
    llm: WasmLlmConfig;
    multiLabel: boolean;
    get promptTemplate(): string | undefined;
    set promptTemplate(value: string | null | undefined);
}

/**
 * Page extraction and tracking configuration.
 *
 * Controls how pages are extracted, tracked, and represented in the extraction results.
 * When `None`, page tracking is disabled.
 *
 * Page range tracking in chunk metadata (first_page/last_page) is automatically enabled
 * when page boundaries are available and chunking is configured.
 */
export class WasmPageConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageConfig;
    constructor(extractPages?: boolean | null, insertPageMarkers?: boolean | null, markerFormat?: string | null);
    extractPages: boolean;
    insertPageMarkers: boolean;
    markerFormat: string;
}

/**
 * Content for a single page/slide.
 *
 * When page extraction is enabled, documents are split into per-page content
 * with associated tables and images mapped to each page.
 *
 * # Performance
 *
 * Uses Arc-wrapped tables and images for memory efficiency:
 * - `Vec<Arc<Table>>` enables zero-copy sharing of table data
 * - `Vec<Arc<ExtractedImage>>` enables zero-copy sharing of image data
 * - Maintains exact JSON compatibility via custom Serialize/Deserialize
 *
 * This reduces memory overhead for documents with shared tables/images
 * by avoiding redundant copies during serialization.
 */
export class WasmPageContent {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageContent;
    constructor(pageNumber: number, content: string, tables: WasmTable[], imageIndices: Uint32Array, hierarchy?: WasmPageHierarchy | null, isBlank?: boolean | null, layoutRegions?: WasmLayoutRegion[] | null, speakerNotes?: string | null, sectionName?: string | null, sheetName?: string | null);
    content: string;
    get hierarchy(): WasmPageHierarchy | undefined;
    set hierarchy(value: WasmPageHierarchy | null | undefined);
    imageIndices: Uint32Array;
    get isBlank(): boolean | undefined;
    set isBlank(value: boolean | null | undefined);
    get layoutRegions(): Array<any> | undefined;
    set layoutRegions(value: WasmLayoutRegion[] | null | undefined);
    pageNumber: number;
    get sectionName(): string | undefined;
    set sectionName(value: string | null | undefined);
    get sheetName(): string | undefined;
    set sheetName(value: string | null | undefined);
    get speakerNotes(): string | undefined;
    set speakerNotes(value: string | null | undefined);
    tables: WasmTable[];
}

/**
 * Page hierarchy structure containing heading levels and block information.
 *
 * Used when PDF text hierarchy extraction is enabled. Contains hierarchical
 * blocks with heading levels (H1-H6) for semantic document structure.
 */
export class WasmPageHierarchy {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageHierarchy;
    constructor(blockCount: number, blocks: WasmHierarchicalBlock[]);
    blockCount: number;
    blocks: WasmHierarchicalBlock[];
}

/**
 * Metadata for individual page/slide/sheet.
 *
 * Captures per-page information including dimensions, content counts,
 * and visibility state (for presentations).
 */
export class WasmPageInfo {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageInfo;
    constructor(number: number, hasVectorGraphics: boolean, title?: string | null, imageCount?: number | null, tableCount?: number | null, hidden?: boolean | null, isBlank?: boolean | null);
    hasVectorGraphics: boolean;
    get hidden(): boolean | undefined;
    set hidden(value: boolean | null | undefined);
    get imageCount(): number | undefined;
    set imageCount(value: number | null | undefined);
    get isBlank(): boolean | undefined;
    set isBlank(value: boolean | null | undefined);
    number: number;
    get tableCount(): number | undefined;
    set tableCount(value: number | null | undefined);
    get title(): string | undefined;
    set title(value: string | null | undefined);
}

/**
 * A single page covered by a chunk, with an optional bounding box on that page.
 *
 * See `ChunkMetadata.page_spans` (#1295) for population semantics.
 */
export class WasmPageSpan {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageSpan;
    constructor(page: number, bbox?: WasmBoundingBox | null);
    get bbox(): WasmBoundingBox | undefined;
    set bbox(value: WasmBoundingBox | null | undefined);
    page: number;
}

/**
 * Unified page structure for documents.
 *
 * Supports different page types (PDF pages, PPTX slides, Excel sheets)
 * with character offset boundaries for chunk-to-page mapping.
 */
export class WasmPageStructure {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPageStructure;
    constructor(totalCount: number, unitType: WasmPageUnitType, boundaries?: WasmPageBoundary[] | null, pages?: WasmPageInfo[] | null);
    get boundaries(): Array<any> | undefined;
    set boundaries(value: WasmPageBoundary[] | null | undefined);
    get pages(): Array<any> | undefined;
    set pages(value: WasmPageInfo[] | null | undefined);
    totalCount: number;
    get unitType(): string;
    set unitType(value: WasmPageUnitType);
}

/**
 * Type of paginated unit in a document.
 *
 * Distinguishes between different types of "pages" (PDF pages, presentation slides, spreadsheet sheets).
 */
export enum WasmPageUnitType {
    Page = 0,
    Slide = 1,
    Sheet = 2,
}

/**
 * One detected PII span in the input text.
 */
export class WasmPatternMatch {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPatternMatch;
    constructor(start: number, end: number, category: WasmPiiCategory, text: string);
    get category(): string;
    set category(value: WasmPiiCategory);
    end: number;
    start: number;
    text: string;
}

/**
 * A PDF annotation extracted from a document page.
 */
export class WasmPdfAnnotation {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPdfAnnotation;
    constructor(annotationType: WasmPdfAnnotationType, pageNumber: number, content?: string | null, boundingBox?: WasmBoundingBox | null);
    get annotationType(): string;
    set annotationType(value: WasmPdfAnnotationType);
    get boundingBox(): WasmBoundingBox | undefined;
    set boundingBox(value: WasmBoundingBox | null | undefined);
    get content(): string | undefined;
    set content(value: string | null | undefined);
    pageNumber: number;
}

/**
 * Type of PDF annotation.
 */
export enum WasmPdfAnnotationType {
    Text = 0,
    Highlight = 1,
    Link = 2,
    Stamp = 3,
    Underline = 4,
    StrikeOut = 5,
    Other = 6,
}

/**
 * A form field extracted from a PDF's AcroForm or XFA structure.
 *
 * Populated by the PDF extractor when `PdfConfig.extract_form_fields` is
 * enabled and the document is a fillable form. Supports both AcroForm (standard)
 * and XFA (XML Forms Architecture) layers. When both are present, AcroForm fields
 * take priority (canonical fallback per PDF spec), and XFA-only fields are appended.
 * The collection is empty for non-form PDFs and for non-PDF formats.
 *
 * `PdfConfig.extract_form_fields`: crate.core.config.PdfConfig.extract_form_fields
 */
export class WasmPdfFormField {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPdfFormField;
    constructor(name: string, fullName: string, fieldType: WasmFormFieldType, flags: number, value?: string | null, defaultValue?: string | null, page?: number | null, bbox?: WasmBoundingBox | null, maxLength?: number | null, tooltip?: string | null);
    get bbox(): WasmBoundingBox | undefined;
    set bbox(value: WasmBoundingBox | null | undefined);
    get defaultValue(): string | undefined;
    set defaultValue(value: string | null | undefined);
    get fieldType(): string;
    set fieldType(value: WasmFormFieldType);
    flags: number;
    fullName: string;
    get maxLength(): number | undefined;
    set maxLength(value: number | null | undefined);
    name: string;
    get page(): number | undefined;
    set page(value: number | null | undefined);
    get tooltip(): string | undefined;
    set tooltip(value: string | null | undefined);
    get value(): string | undefined;
    set value(value: string | null | undefined);
}

/**
 * PII categories the pattern engine recognises.
 */
export enum WasmPiiCategory {
    Email = 0,
    Phone = 1,
    Ssn = 2,
    CreditCard = 3,
    PostalCode = 4,
    IpAddress = 5,
    Iban = 6,
    SwiftBic = 7,
    DateOfBirth = 8,
    Person = 9,
    Organization = 10,
    Location = 11,
    Custom = 12,
}

/**
 * Post-processor configuration.
 */
export class WasmPostProcessorConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPostProcessorConfig;
    constructor(enabled?: boolean | null, enabledProcessors?: string[] | null, disabledProcessors?: string[] | null, enabledSet?: string[] | null, disabledSet?: string[] | null);
    get disabledProcessors(): string[] | undefined;
    set disabledProcessors(value: string[] | null | undefined);
    get disabledSet(): string[] | undefined;
    set disabledSet(value: string[] | null | undefined);
    enabled: boolean;
    get enabledProcessors(): string[] | undefined;
    set enabledProcessors(value: string[] | null | undefined);
    get enabledSet(): string[] | undefined;
    set enabledSet(value: string[] | null | undefined);
}

/**
 * Application properties from docProps/app.xml for PPTX
 *
 * Contains PowerPoint-specific document metadata.
 */
export class WasmPptxAppProperties {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPptxAppProperties;
    constructor(slideTitles?: string[] | null, application?: string | null, appVersion?: string | null, totalTime?: number | null, company?: string | null, docSecurity?: number | null, scaleCrop?: boolean | null, linksUpToDate?: boolean | null, sharedDoc?: boolean | null, hyperlinksChanged?: boolean | null, slides?: number | null, notes?: number | null, hiddenSlides?: number | null, multimediaClips?: number | null, presentationFormat?: string | null);
    get appVersion(): string | undefined;
    set appVersion(value: string | null | undefined);
    get application(): string | undefined;
    set application(value: string | null | undefined);
    get company(): string | undefined;
    set company(value: string | null | undefined);
    get docSecurity(): number | undefined;
    set docSecurity(value: number | null | undefined);
    get hiddenSlides(): number | undefined;
    set hiddenSlides(value: number | null | undefined);
    get hyperlinksChanged(): boolean | undefined;
    set hyperlinksChanged(value: boolean | null | undefined);
    get linksUpToDate(): boolean | undefined;
    set linksUpToDate(value: boolean | null | undefined);
    get multimediaClips(): number | undefined;
    set multimediaClips(value: number | null | undefined);
    get notes(): number | undefined;
    set notes(value: number | null | undefined);
    get presentationFormat(): string | undefined;
    set presentationFormat(value: string | null | undefined);
    get scaleCrop(): boolean | undefined;
    set scaleCrop(value: boolean | null | undefined);
    get sharedDoc(): boolean | undefined;
    set sharedDoc(value: boolean | null | undefined);
    slideTitles: string[];
    get slides(): number | undefined;
    set slides(value: number | null | undefined);
    get totalTime(): number | undefined;
    set totalTime(value: number | null | undefined);
}

/**
 * PowerPoint (PPTX) extraction result.
 *
 * Contains extracted slide content, metadata, and embedded images/tables.
 */
export class WasmPptxExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPptxExtractionResult;
    constructor(content: string, metadata: WasmPptxMetadata, slideCount: number, imageCount: number, tableCount: number, images: WasmExtractedImage[], officeMetadata: any, pageStructure?: WasmPageStructure | null, pageContents?: WasmPageContent[] | null, document?: WasmDocumentStructure | null, revisions?: WasmDocumentRevision[] | null);
    content: string;
    get document(): WasmDocumentStructure | undefined;
    set document(value: WasmDocumentStructure | null | undefined);
    imageCount: number;
    images: WasmExtractedImage[];
    metadata: WasmPptxMetadata;
    officeMetadata: any;
    get pageContents(): Array<any> | undefined;
    set pageContents(value: WasmPageContent[] | null | undefined);
    get pageStructure(): WasmPageStructure | undefined;
    set pageStructure(value: WasmPageStructure | null | undefined);
    get revisions(): Array<any> | undefined;
    set revisions(value: WasmDocumentRevision[] | null | undefined);
    slideCount: number;
    tableCount: number;
}

/**
 * PowerPoint presentation metadata.
 *
 * Extracted from PPTX files containing slide counts and presentation details.
 */
export class WasmPptxMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPptxMetadata;
    constructor(slideCount?: number | null, slideNames?: string[] | null, imageCount?: number | null, tableCount?: number | null);
    get imageCount(): number | undefined;
    set imageCount(value: number | null | undefined);
    slideCount: number;
    slideNames: string[];
    get tableCount(): number | undefined;
    set tableCount(value: number | null | undefined);
}

/**
 * HTML preprocessing options for document cleanup before conversion.
 */
export class WasmPreprocessingOptions {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPreprocessingOptions;
    constructor(enabled?: boolean | null, preset?: WasmPreprocessingPreset | null, removeNavigation?: boolean | null, removeForms?: boolean | null);
    enabled: boolean;
    get preset(): string;
    set preset(value: WasmPreprocessingPreset);
    removeForms: boolean;
    removeNavigation: boolean;
}

/**
 * HTML preprocessing aggressiveness level.
 *
 * Controls the extent of cleanup performed before conversion. Higher levels remove more elements.
 */
export enum WasmPreprocessingPreset {
    Minimal = 0,
    Standard = 1,
    Aggressive = 2,
}

/**
 * Processing stages for post-processors.
 *
 * Post-processors are executed in stage order (Early → Middle → Late).
 * Use stages to control the order of post-processing operations.
 */
export enum WasmProcessingStage {
    Early = 0,
    Middle = 1,
    Late = 2,
}

/**
 * A non-fatal warning from a processing pipeline stage.
 *
 * Captures errors from optional features that don't prevent extraction
 * but may indicate degraded results.
 */
export class WasmProcessingWarning {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmProcessingWarning;
    constructor(source: string, message: string);
    message: string;
    source: string;
}

/**
 * A single run-level or style-level property change.
 *
 * Used for revisions that change formatting rather than text content. `from`
 * and `to` store normalized property values when the source format exposes
 * them; either side may be absent when the format only records one side of the
 * change.
 */
export class WasmPropertyChange {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPropertyChange;
    constructor(name: string, from?: string | null, to?: string | null);
    get from(): string | undefined;
    set from(value: string | null | undefined);
    name: string;
    get to(): string | undefined;
    set to(value: string | null | undefined);
}

/**
 * Proxy configuration for HTTP requests.
 */
export class WasmProxyConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmProxyConfig;
    constructor(url?: string | null, username?: string | null, password?: string | null);
    get password(): string | undefined;
    set password(value: string | null | undefined);
    url: string;
    get username(): string | undefined;
    set username(value: string | null | undefined);
}

/**
 * Outlook PST archive metadata.
 */
export class WasmPstMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmPstMetadata;
    constructor(messageCount?: number | null);
    messageCount: number;
}

/**
 * Pixel-space bounding box of a QR code inside its source image.
 */
export class WasmQrBoundingBox {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmQrBoundingBox;
    constructor(x: number, y: number, width: number, height: number);
    height: number;
    width: number;
    x: number;
    y: number;
}

/**
 * One QR code decoded from an extracted image.
 */
export class WasmQrCode {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmQrCode;
    constructor(payload: string, confidence?: number | null, bbox?: WasmQrBoundingBox | null);
    get bbox(): WasmQrBoundingBox | undefined;
    set bbox(value: WasmQrBoundingBox | null | undefined);
    get confidence(): number | undefined;
    set confidence(value: number | null | undefined);
    payload: string;
}

/**
 * Configuration for the redaction post-processor.
 */
export class WasmRedactionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRedactionConfig;
    constructor(categories?: any[] | null, strategy?: WasmRedactionStrategy | null, preserveOffsets?: boolean | null, customTerms?: WasmRedactionTerm[] | null, customPatterns?: WasmRedactionPattern[] | null, ner?: WasmNerConfig | null);
    /**
     * Validate user-supplied terms and patterns at config-construction time.
     *
     * Compiles every `RedactionPattern.pattern` (with the case-insensitive
     * inline flag where applicable) and returns the first compilation error so
     * the caller can reject the config before the redaction pipeline runs.
     * Pure terms (regex-escaped) cannot fail to compile, but the function
     * still rejects empty values to avoid degenerate zero-length matches.
     */
    validate(): void;
    categories: string[];
    customPatterns: WasmRedactionPattern[];
    customTerms: WasmRedactionTerm[];
    get ner(): WasmNerConfig | undefined;
    set ner(value: WasmNerConfig | null | undefined);
    preserveOffsets: boolean;
    get strategy(): string;
    set strategy(value: WasmRedactionStrategy);
}

/**
 * One redaction event: which span was rewritten, why, and with what.
 */
export class WasmRedactionFinding {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRedactionFinding;
    constructor(start: number, end: number, category: WasmPiiCategory, strategy: WasmRedactionStrategy, replacementToken: string);
    get category(): string;
    set category(value: WasmPiiCategory);
    end: number;
    replacementToken: string;
    start: number;
    get strategy(): string;
    set strategy(value: WasmRedactionStrategy);
}

/**
 * One user-supplied regex pattern to redact.
 *
 * The pattern is compiled with the Rust `regex` crate (no look-around). Case
 * sensitivity is encoded in the pattern via the `(?i)` inline flag when
 * `Self.case_sensitive` is `false`.
 */
export class WasmRedactionPattern {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRedactionPattern;
    /**
     * Build a pattern with the given label (case-insensitive by default).
     */
    static labeled(label: string, pattern: string): WasmRedactionPattern;
    constructor(label: string, pattern: string, caseSensitive: boolean);
    caseSensitive: boolean;
    label: string;
    pattern: string;
}

/**
 * Audit report describing what the redaction processor found and how it replaced it.
 *
 * The redactor returns this alongside the rewritten content so compliance, replay, and
 * audit-log consumers can see exactly what fired. Offsets are relative to the *original*
 * pre-redaction `content` and are intended for audit reconstruction only — the original
 * bytes are dropped at the end of the pipeline.
 */
export class WasmRedactionReport {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRedactionReport;
    constructor(findings: WasmRedactionFinding[], totalRedacted: number);
    findings: WasmRedactionFinding[];
    totalRedacted: number;
}

/**
 * Strategy applied when a PII match is rewritten.
 */
export enum WasmRedactionStrategy {
    Mask = 0,
    Hash = 1,
    TokenReplace = 2,
    Drop = 3,
}

/**
 * One user-supplied literal term to redact.
 *
 * Matched as a regex-escaped substring (so callers do not need to escape
 * metacharacters themselves). Case-insensitive by default — set
 * `Self.case_sensitive` to `true` for exact byte-match semantics.
 */
export class WasmRedactionTerm {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRedactionTerm;
    /**
     * Build a term with a custom label.
     */
    static labeled(label: string, value: string): WasmRedactionTerm;
    /**
     * Build a term whose label is the literal value itself (case-insensitive).
     */
    static literal(value: string): WasmRedactionTerm;
    constructor(label: string, value: string, caseSensitive: boolean);
    caseSensitive: boolean;
    label: string;
    value: string;
}

/**
 * Semantic kind of a relationship between document elements.
 */
export enum WasmRelationshipKind {
    FootnoteReference = 0,
    CitationReference = 1,
    InternalLink = 2,
    Caption = 3,
    Label = 4,
    TocEntry = 5,
    CrossReference = 6,
}

/**
 * Configuration for the reranking pipeline.
 *
 * Controls which model to use, how many results to return, and download/cache
 * behavior for local ONNX models.
 *
 * Since v5.0.0.
 */
export class WasmRerankerConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRerankerConfig;
    constructor(model?: any | null, batchSize?: number | null, showDownloadProgress?: boolean | null, topK?: number | null, cacheDir?: string | null, acceleration?: WasmAccelerationConfig | null, maxRerankDurationSecs?: bigint | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    batchSize: number;
    get cacheDir(): string | undefined;
    set cacheDir(value: string | null | undefined);
    get maxRerankDurationSecs(): bigint | undefined;
    set maxRerankDurationSecs(value: bigint | null | undefined);
    model: any;
    showDownloadProgress: boolean;
    get topK(): number | undefined;
    set topK(value: number | null | undefined);
}

/**
 * Selects how a local ONNX reranker's raw output tensor is turned into a score.
 *
 * - `RerankerHead.CrossEncoder` — classic single-logit cross-encoder head:
 *   the model emits `[batch, 1]` (or `[batch]`) logits; the caller applies
 *   sigmoid to get a `[0, 1]` score. This is the original, unchanged path.
 * - `RerankerHead.Qwen3Generative` — Qwen3 generative-reranker head: the
 *   model emits `[batch, seq, vocab]` logits; the score is `P("yes")` read
 *   from the last token's logits over the "yes"/"no" vocabulary entries,
 *   via a softmax over those two logits. Already a `[0, 1]` probability —
 *   no sigmoid is applied.
 *
 * Since v5.0.0.
 */
export enum WasmRerankerHead {
    CrossEncoder = 0,
    Qwen3Generative = 1,
}

/**
 * Reranker model types supported by Xberg.
 *
 * Since v5.0.0.
 */
export class WasmRerankerModelType {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRerankerModelType;
    constructor();
    get additionalFiles(): string[] | undefined;
    set additionalFiles(value: string[] | null | undefined);
    get head(): WasmRerankerHead | undefined;
    set head(value: WasmRerankerHead | null | undefined);
    get llm(): WasmLlmConfig | undefined;
    set llm(value: WasmLlmConfig | null | undefined);
    get maxLength(): bigint | undefined;
    set maxLength(value: bigint | null | undefined);
    get modelFile(): string | undefined;
    set modelFile(value: string | null | undefined);
    get modelId(): string | undefined;
    set modelId(value: string | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    type: string;
}

/**
 * Result-shape selection for extraction results.
 *
 * Distinct from `OutputFormat` (which controls rendering — Plain, Markdown,
 * HTML, etc.). `ResultFormat` controls the *shape* of the result: a unified content
 * blob vs. an element-based decomposition.
 */
export enum WasmResultFormat {
    Unified = 0,
    ElementBased = 1,
}

/**
 * Best-effort document location for a revision.
 */
export class WasmRevisionAnchor {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRevisionAnchor;
    constructor();
    get col(): number | undefined;
    set col(value: number | null | undefined);
    get index(): number | undefined;
    set index(value: number | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    get row(): number | undefined;
    set row(value: number | null | undefined);
    get tableIndex(): number | undefined;
    set tableIndex(value: number | null | undefined);
    type: string;
}

/**
 * The content changes that make up a single revision.
 *
 * For insertions and deletions the `content` field carries the added/removed
 * lines as `DiffLine.Added` / `DiffLine.Removed` entries. For format
 * changes, `property_changes` carries normalized before/after formatting
 * values when the source document exposes them.
 */
export class WasmRevisionDelta {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmRevisionDelta;
    constructor(content?: any | null, tableChanges?: WasmCellChange[] | null, propertyChanges?: WasmPropertyChange[] | null);
    content: any;
    propertyChanges: WasmPropertyChange[];
    tableChanges: WasmCellChange[];
}

/**
 * Semantic classification of a tracked change.
 */
export enum WasmRevisionKind {
    Insertion = 0,
    Deletion = 1,
    FormatChange = 2,
    Comment = 3,
}

/**
 * Configuration for security limits across extractors.
 *
 * All limits are intentionally conservative to prevent DoS attacks
 * while still supporting legitimate documents.
 */
export class WasmSecurityLimits {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSecurityLimits;
    constructor(maxArchiveSize?: number | null, maxCompressionRatio?: number | null, maxFilesInArchive?: number | null, maxNestingDepth?: number | null, maxEntityLength?: number | null, maxContentSize?: number | null, maxIterations?: number | null, maxXmlDepth?: number | null, maxTableCells?: number | null);
    maxArchiveSize: number;
    maxCompressionRatio: number;
    maxContentSize: number;
    maxEntityLength: number;
    maxFilesInArchive: number;
    maxIterations: number;
    maxNestingDepth: number;
    maxTableCells: number;
    maxXmlDepth: number;
}

/**
 * A URL entry from a sitemap.
 */
export class WasmSitemapUrl {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSitemapUrl;
    constructor(url?: string | null, lastmod?: string | null, changefreq?: string | null, priority?: string | null);
    get changefreq(): string | undefined;
    set changefreq(value: string | null | undefined);
    get lastmod(): string | undefined;
    set lastmod(value: string | null | undefined);
    get priority(): string | undefined;
    set priority(value: string | null | undefined);
    url: string;
}

/**
 * Configuration for the sparse-embedding pipeline.
 *
 * Controls which model to use, batching, and download/cache behavior for the
 * local ONNX SPLADE model.
 *
 * Since v5.0.0.
 */
export class WasmSparseEmbeddingConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSparseEmbeddingConfig;
    constructor(model?: any | null, batchSize?: number | null, maxLength?: number | null, showDownloadProgress?: boolean | null, cacheDir?: string | null, acceleration?: WasmAccelerationConfig | null, maxEmbedDurationSecs?: bigint | null);
    get acceleration(): WasmAccelerationConfig | undefined;
    set acceleration(value: WasmAccelerationConfig | null | undefined);
    batchSize: number;
    get cacheDir(): string | undefined;
    set cacheDir(value: string | null | undefined);
    get maxEmbedDurationSecs(): bigint | undefined;
    set maxEmbedDurationSecs(value: bigint | null | undefined);
    maxLength: number;
    model: any;
    showDownloadProgress: boolean;
}

/**
 * Sparse-embedding model types supported by Xberg.
 *
 * Since v5.0.0.
 */
export class WasmSparseEmbeddingModelType {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSparseEmbeddingModelType;
    constructor();
    get additionalFiles(): string[] | undefined;
    set additionalFiles(value: string[] | null | undefined);
    get maxLength(): bigint | undefined;
    set maxLength(value: bigint | null | undefined);
    get modelFile(): string | undefined;
    set modelFile(value: string | null | undefined);
    get modelId(): string | undefined;
    set modelId(value: string | null | undefined);
    get name(): string | undefined;
    set name(value: string | null | undefined);
    type: string;
}

/**
 * SSRF policy configuration.
 */
export class WasmSsrfPolicy {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSsrfPolicy;
    constructor(denyPrivate?: boolean | null, maxRedirects?: number | null);
    denyPrivate: boolean;
    maxRedirects: number;
}

/**
 * Structured data (Schema.org, microdata, RDFa) block.
 */
export class WasmStructuredData {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmStructuredData;
    constructor(dataType: WasmStructuredDataType, rawJson: string, schemaType?: string | null);
    get dataType(): string;
    set dataType(value: WasmStructuredDataType);
    rawJson: string;
    get schemaType(): string | undefined;
    set schemaType(value: string | null | undefined);
}

/**
 * Result of parsing a structured data file (JSON, JSONL, YAML, or TOML).
 */
export class WasmStructuredDataResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmStructuredDataResult;
    constructor(content: string, format: string, metadata: any, textFields: string[]);
    content: string;
    format: string;
    metadata: any;
    textFields: string[];
}

/**
 * Structured data type classification.
 */
export enum WasmStructuredDataType {
    JsonLd = 0,
    Microdata = 1,
    RDFa = 2,
}

/**
 * Configuration for LLM-based structured data extraction.
 *
 * Sends extracted document content to a VLM with a JSON schema,
 * returning structured data that conforms to the schema.
 *
 * # Example
 *
 * ```toml
 * [structured_extraction]
 * schema_name = "invoice_data"
 * strict = true
 *
 * [structured_extraction.schema]
 * type = "object"
 * properties.vendor = { type = "string" }
 * properties.total = { type = "number" }
 * required = ["vendor", "total"]
 *
 * [structured_extraction.llm]
 * model = "openai/gpt-4o"
 * ```
 */
export class WasmStructuredExtractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmStructuredExtractionConfig;
    constructor(schema: any, schemaName: string, strict: boolean, llm: WasmLlmConfig, schemaDescription?: string | null, prompt?: string | null);
    llm: WasmLlmConfig;
    get prompt(): string | undefined;
    set prompt(value: string | null | undefined);
    schema: any;
    get schemaDescription(): string | undefined;
    set schemaDescription(value: string | null | undefined);
    schemaName: string;
    strict: boolean;
}

/**
 * Configuration for the summarisation post-processor.
 */
export class WasmSummarizationConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSummarizationConfig;
    constructor(strategy?: WasmSummaryStrategy | null, maxTokens?: number | null, llm?: WasmLlmConfig | null);
    get llm(): WasmLlmConfig | undefined;
    set llm(value: WasmLlmConfig | null | undefined);
    get maxTokens(): number | undefined;
    set maxTokens(value: number | null | undefined);
    get strategy(): string;
    set strategy(value: WasmSummaryStrategy);
}

/**
 * Summarisation strategy.
 */
export enum WasmSummaryStrategy {
    Extractive = 0,
    Abstractive = 1,
}

/**
 * A supported document format entry.
 *
 * Represents a file extension and its corresponding MIME type that Xberg can process.
 */
export class WasmSupportedFormat {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmSupportedFormat;
    constructor(extension: string, mimeType: string);
    extension: string;
    mimeType: string;
}

/**
 * Extracted table structure.
 *
 * Represents a table detected and extracted from a document (PDF, image, etc.).
 * Tables are converted to both structured cell data and Markdown format.
 */
export class WasmTable {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTable;
    constructor(cells?: any | null, markdown?: string | null, pageNumber?: number | null, boundingBox?: WasmBoundingBox | null, tableId?: string | null, columns?: string[] | null);
    get boundingBox(): WasmBoundingBox | undefined;
    set boundingBox(value: WasmBoundingBox | null | undefined);
    cells: any;
    get columns(): string[] | undefined;
    set columns(value: string[] | null | undefined);
    markdown: string;
    pageNumber: number;
    get tableId(): string | undefined;
    set tableId(value: string | null | undefined);
}

/**
 * Individual table cell with content and optional styling.
 *
 * Future extension point for rich table support with cell-level metadata.
 */
export class WasmTableCell {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTableCell;
    constructor(content?: string | null, rowSpan?: number | null, colSpan?: number | null, isHeader?: boolean | null);
    colSpan: number;
    content: string;
    isHeader: boolean;
    rowSpan: number;
}

/**
 * Controls how markdown tables are handled when they exceed the chunk size limit.
 *
 * Only applies when `chunker_type` is `Markdown`.
 *
 * # Variants
 *
 * * `Split` - Default behavior: tables are split at row boundaries like any
 *   other block element. Continuation chunks contain only data rows without
 *   the header, which can break downstream consumers that need column context.
 * * `RepeatHeader` - Prepend the table header (header row + separator row) to
 *   every continuation chunk that contains data rows from the same table.
 *   Adds a small amount of duplicate text but ensures each chunk is
 *   self-contained for extraction, search, and LLM consumption.
 */
export enum WasmTableChunkingMode {
    Split = 0,
    RepeatHeader = 1,
}

/**
 * Structured table grid with cell-level metadata.
 *
 * Stores row/column dimensions and a flat list of cells with position info.
 */
export class WasmTableGrid {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTableGrid;
    constructor(rows?: number | null, cols?: number | null, cells?: WasmGridCell[] | null);
    cells: WasmGridCell[];
    cols: number;
    rows: number;
}

/**
 * Tesseract OCR configuration.
 *
 * Provides fine-grained control over Tesseract OCR engine parameters.
 * Most users can use the defaults, but these settings allow optimization
 * for specific document types (invoices, handwriting, etc.).
 */
export class WasmTesseractConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTesseractConfig;
    constructor(language?: string[] | null, psm?: number | null, outputFormat?: string | null, oem?: number | null, minConfidence?: number | null, enableTableDetection?: boolean | null, tableMinConfidence?: number | null, tableColumnThreshold?: number | null, tableRowThresholdRatio?: number | null, useCache?: boolean | null, classifyUsePreAdaptedTemplates?: boolean | null, languageModelNgramOn?: boolean | null, tesseditDontBlkrejGoodWds?: boolean | null, tesseditDontRowrejGoodWds?: boolean | null, tesseditEnableDictCorrection?: boolean | null, tesseditCharWhitelist?: string | null, tesseditCharBlacklist?: string | null, tesseditUsePrimaryParamsModel?: boolean | null, textordSpaceSizeIsVariable?: boolean | null, thresholdingMethod?: boolean | null, preprocessing?: WasmImagePreprocessingConfig | null);
    classifyUsePreAdaptedTemplates: boolean;
    enableTableDetection: boolean;
    language: string[];
    languageModelNgramOn: boolean;
    minConfidence: number;
    oem: number;
    outputFormat: string;
    get preprocessing(): WasmImagePreprocessingConfig | undefined;
    set preprocessing(value: WasmImagePreprocessingConfig | null | undefined);
    psm: number;
    tableColumnThreshold: number;
    tableMinConfidence: number;
    tableRowThresholdRatio: number;
    tesseditCharBlacklist: string;
    tesseditCharWhitelist: string;
    tesseditDontBlkrejGoodWds: boolean;
    tesseditDontRowrejGoodWds: boolean;
    tesseditEnableDictCorrection: boolean;
    tesseditUsePrimaryParamsModel: boolean;
    textordSpaceSizeIsVariable: boolean;
    thresholdingMethod: boolean;
    useCache: boolean;
}

/**
 * Inline text annotation — byte-range based formatting and links.
 *
 * Annotations reference byte offsets into the node's text content,
 * enabling precise identification of formatted regions.
 */
export class WasmTextAnnotation {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTextAnnotation;
    constructor(start: number, end: number, kind: any);
    end: number;
    kind: any;
    start: number;
}

/**
 * Text direction enumeration for HTML documents.
 */
export enum WasmTextDirection {
    LeftToRight = 0,
    RightToLeft = 1,
    Auto = 2,
}

/**
 * Plain text and Markdown extraction result.
 *
 * Contains the extracted text along with statistics and,
 * for Markdown files, structural elements like headers and links.
 */
export class WasmTextExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTextExtractionResult;
    constructor(content: string, lineCount: number, wordCount: number, characterCount: number, headers?: string[] | null);
    characterCount: number;
    content: string;
    get headers(): string[] | undefined;
    set headers(value: string[] | null | undefined);
    lineCount: number;
    wordCount: number;
}

/**
 * Text/Markdown metadata.
 *
 * Extracted from plain text and Markdown files. Includes word counts and,
 * for Markdown, structural elements like headers and links.
 */
export class WasmTextMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTextMetadata;
    constructor(lineCount?: number | null, wordCount?: number | null, characterCount?: number | null, headers?: string[] | null);
    characterCount: number;
    get headers(): string[] | undefined;
    set headers(value: string[] | null | undefined);
    lineCount: number;
    wordCount: number;
}

/**
 * Token reduction configuration.
 */
export class WasmTokenReductionOptions {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTokenReductionOptions;
    constructor(mode?: string | null, preserveImportantWords?: boolean | null);
    mode: string;
    preserveImportantWords: boolean;
}

/**
 * Translation of the extracted content.
 *
 * Holds the translated rendition of `ExtractedDocument.content` and (when
 * `preserve_markup` was requested) the translated `formatted_content`. Chunks
 * are translated in place inside `ExtractedDocument.chunks[*].content` rather
 * than duplicated here.
 */
export class WasmTranslation {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTranslation;
    constructor(targetLang: string, content: string, sourceLang?: string | null, formattedContent?: string | null);
    content: string;
    get formattedContent(): string | undefined;
    set formattedContent(value: string | null | undefined);
    get sourceLang(): string | undefined;
    set sourceLang(value: string | null | undefined);
    targetLang: string;
}

/**
 * Configuration for the translation post-processor.
 */
export class WasmTranslationConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmTranslationConfig;
    constructor(targetLang: string, preserveMarkup: boolean, llm: WasmLlmConfig, sourceLang?: string | null);
    llm: WasmLlmConfig;
    preserveMarkup: boolean;
    get sourceLang(): string | undefined;
    set sourceLang(value: string | null | undefined);
    targetLang: string;
}

/**
 * Semantic classification of an extracted URI.
 */
export enum WasmUriKind {
    Hyperlink = 0,
    Image = 1,
    Anchor = 2,
    Citation = 3,
    Reference = 4,
    Email = 5,
}

/**
 * URL encoding strategy for link and image destinations.
 *
 * Controls how special characters in URL destinations are handled when they
 * require escaping to produce valid Markdown.
 *
 * The `Angle` variant (default) wraps the destination in angle brackets:
 * `[text](<url with spaces>)`. This is the CommonMark-specified escape hatch
 * but breaks when the URL itself contains `>`.
 *
 * The `Percent` variant percent-encodes every character that is not an RFC 3986
 * unreserved character or `/`, producing a destination safe for all Markdown
 * parsers: `[text](url%20with%20spaces)`.
 */
export enum WasmUrlEscapeStyle {
    Angle = 0,
    Percent = 1,
}

/**
 * URL ingestion and crawl configuration.
 */
export class WasmUrlExtractionConfig {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmUrlExtractionConfig;
    constructor(mode?: WasmUrlExtractionMode | null, crawl?: WasmCrawlConfig | null, allowLocalFileInputs?: boolean | null, allowFileUris?: boolean | null, documentUrlPattern?: string | null, maxDocumentUrlsPerResult?: number | null, maxTotalUrls?: number | null);
    allowFileUris: boolean;
    allowLocalFileInputs: boolean;
    crawl: WasmCrawlConfig;
    get documentUrlPattern(): string | undefined;
    set documentUrlPattern(value: string | null | undefined);
    get maxDocumentUrlsPerResult(): number | undefined;
    set maxDocumentUrlsPerResult(value: number | null | undefined);
    get maxTotalUrls(): number | undefined;
    set maxTotalUrls(value: number | null | undefined);
    get mode(): string;
    set mode(value: WasmUrlExtractionMode);
}

/**
 * URL extraction mode.
 */
export enum WasmUrlExtractionMode {
    Auto = 0,
    Document = 1,
    Crawl = 2,
}

/**
 * Policy controlling when VLM (Vision Language Model) OCR is used as a fallback.
 *
 * This knob is syntactic sugar over the explicit `OcrPipelineConfig` stage
 * ordering. When `vlm_fallback` is set and `pipeline` is `None`, an equivalent
 * pipeline is synthesised at extraction time:
 *
 * - `VlmFallbackPolicy.Disabled` — no synthesis; single-backend mode (default).
 * - `VlmFallbackPolicy.OnLowQuality` — tries the classical backend first; if the
 *   result scores below `quality_threshold`, tries VLM.
 * - `VlmFallbackPolicy.Always` — skips the classical backend and sends every page
 *   to the VLM.
 *
 * When `OcrConfig.pipeline` is explicitly set, `vlm_fallback` is ignored — the
 * explicit pipeline takes precedence.
 *
 * # Errors
 *
 * Both `OnLowQuality` and `Always` require `OcrConfig.vlm_config` to be `Some`.
 * Constructing an `OcrConfig` with one of these policies but no `vlm_config` is
 * detected by `OcrConfig.validate` and will surface as a
 * `Validation` error at extraction time, not a panic.
 *
 * # Example
 */
export class WasmVlmFallbackPolicy {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmVlmFallbackPolicy;
    constructor();
    mode: string;
    get qualityThreshold(): number | undefined;
    set qualityThreshold(value: number | null | undefined);
}

/**
 * Whitespace handling strategy during conversion.
 *
 * Determines how sequences of whitespace characters (spaces, tabs, newlines) are processed.
 */
export enum WasmWhitespaceMode {
    Normalized = 0,
    Strict = 1,
}

/**
 * Application properties from docProps/app.xml for XLSX
 *
 * Contains Excel-specific document metadata.
 */
export class WasmXlsxAppProperties {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmXlsxAppProperties;
    constructor(worksheetNames?: string[] | null, application?: string | null, appVersion?: string | null, docSecurity?: number | null, scaleCrop?: boolean | null, linksUpToDate?: boolean | null, sharedDoc?: boolean | null, hyperlinksChanged?: boolean | null, company?: string | null);
    get appVersion(): string | undefined;
    set appVersion(value: string | null | undefined);
    get application(): string | undefined;
    set application(value: string | null | undefined);
    get company(): string | undefined;
    set company(value: string | null | undefined);
    get docSecurity(): number | undefined;
    set docSecurity(value: number | null | undefined);
    get hyperlinksChanged(): boolean | undefined;
    set hyperlinksChanged(value: boolean | null | undefined);
    get linksUpToDate(): boolean | undefined;
    set linksUpToDate(value: boolean | null | undefined);
    get scaleCrop(): boolean | undefined;
    set scaleCrop(value: boolean | null | undefined);
    get sharedDoc(): boolean | undefined;
    set sharedDoc(value: boolean | null | undefined);
    worksheetNames: string[];
}

/**
 * XML extraction result.
 *
 * Contains extracted text content from XML files along with
 * structural statistics about the XML document.
 */
export class WasmXmlExtractionResult {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmXmlExtractionResult;
    constructor(content: string, elementCount: number, uniqueElements: string[]);
    content: string;
    elementCount: number;
    uniqueElements: string[];
}

/**
 * XML metadata extracted during XML parsing.
 *
 * Provides statistics about XML document structure.
 */
export class WasmXmlMetadata {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmXmlMetadata;
    constructor(elementCount?: number | null, uniqueElements?: string[] | null);
    elementCount: number;
    uniqueElements: string[];
}

/**
 * Year range for bibliographic metadata.
 */
export class WasmYearRange {
    free(): void;
    [Symbol.dispose](): void;
    static default(): WasmYearRange;
    constructor(years: Uint32Array, min?: number | null, max?: number | null);
    get max(): number | undefined;
    set max(value: number | null | undefined);
    get min(): number | undefined;
    set min(value: number | null | undefined);
    years: Uint32Array;
}

/**
 * Stateful engine handle exposed to JS.
 *
 * Constructed via `XbergEngine.new(config, injection)` where `config` may
 * contain optional settings (e.g. `bridgeTimeoutMs`) and `injection` is a
 * plain object with an optional `ocr` key.
 */
export class XbergEngine {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Extract content from a single bytes or URI input.
     */
    extract(input: any, config: any): Promise<any>;
    /**
     * Perform Named Entity Recognition on `text` through the injected NER
     * backend. `opts` may contain `categories`, an array of category names;
     * unknown names are treated as custom zero-shot labels.
     *
     * This is the injected-backend path. To run a model inside the browser
     * instead, load one with
     * [`NerModel`](crate::bridge::ner_model::NerModel) and call its `detect`
     * method directly — it needs none of the promise bridging or timeout this
     * path provides.
     */
    ner(text: string, opts: any): Promise<any>;
    /**
     * Create a new engine with injected bridges.
     *
     * `config` may contain:
     * - `bridgeTimeoutMs`; timeout in milliseconds for JS bridge calls
     *   (defaults to 30,000ms if not provided)
     *
     * `injection` may contain:
     * - `ner`; object with `ner(text, categories): Promise<Array<{ category, text, start, end, confidence? }>>`
     * - `ocr`; object with `ocr(imageBytes, opts): Promise<{ text: string, lines?: Array<{ text: string, confidence: number, bbox?: { x: number, y: number, width: number, height: number } }> }>`
     *
     * Unknown injection keys are ignored, so hosts can pass richer injection
     * objects shared with other engines.
     */
    constructor(config: any, injection: any);
    /**
     * Perform OCR on image bytes, returning extracted text with per-line
     * confidence and bounding-box geometry (when the backend provides it).
     */
    ocr(bytes: Uint8Array, opts: any): Promise<any>;
}

/**
 * Run chunk classification against an extraction result.
 *
 * Mutates `ChunkMetadata.classifications` on every chunk in
 * `result.chunks` and appends every LLM call's usage to `result.llm_usage`.
 * A chunk whose classification batch call fails (or that the model omitted
 * from its response) is simply left with an empty `classifications` vector for
 * that chunk, unless the failure is a validation error (empty config) or every
 * batch task fails, in which case the first error is returned.
 *
 * # Errors
 *
 * Returns `Validation` when `config.definitions` is empty.
 * Returns the first batch error encountered when rendering the prompt or
 * calling the LLM fails for every batch; partial failures on a subset of
 * batches are recorded as `ProcessingWarning`s by the caller instead of
 * aborting the whole run (see
 * `chunk_classification`).
 */
export function classifyChunks(result: any, config: any): Promise<void>;

export function clearDocumentExtractors(): void;

export function clearEmbeddingBackends(): void;

export function clearOcrBackends(): void;

export function clearPostProcessors(): void;

export function clearRenderers(): void;

export function clearRerankerBackends(): void;

export function clearTokenizerBackends(): void;

export function clearValidators(): void;

/**
 * Compresses multiple entries into a 7z archive in WebAssembly environment.
 *
 * This function creates a compressed archive from multiple file entries,
 * designed specifically for WASM targets.
 *
 * # Arguments
 * * `entries` - Vector of JavaScript strings representing file names/paths
 * * `datas` - Vector of Uint8Arrays containing the file data corresponding to entries
 */
export function compress(entries: string[], datas: Uint8Array[]): Uint8Array;

/**
 * Decompresses a 7z archive in WebAssembly environment.
 *
 * This function is specifically designed for WASM targets and uses JavaScript interop
 * to handle the decompression process with a callback function.
 *
 * # Arguments
 * * `src` - Uint8Array containing the compressed archive data
 * * `pwd` - Password string for encrypted archives (use empty string for unencrypted)
 * * `f` - JavaScript callback function to handle extracted entries
 */
export function decompress(src: Uint8Array, pwd: string, f: Function): void;

/**
 * Detect page layout (RT-DETR) from encoded image bytes using a caller-supplied
 * ONNX model.
 *
 * Both arguments are raw bytes: `imageBytes` is an encoded image (PNG/JPEG/…) and
 * `modelBytes` is the RT-DETR `.onnx` weights the JS host fetched. Weights are
 * never embedded in the `.wasm` (RT-DETR alone runs to hundreds of MB, far over
 * the CDN per-file cap), so the host fetches them and hands the bytes over here.
 * Inference runs entirely in Rust through the pure-Rust tract engine; the returned
 * value is a `DetectionResult` object (bounding boxes, classes, confidences).
 *
 * Only RT-DETR detection is available on WASM; the ORT-only layout models
 * (`PP-DocLayout-V3`, YOLO) and table-structure recognition (TATR, SLANeXT) are not.
 */
export function detectLayout(image_bytes: Uint8Array, model_bytes: Uint8Array): any;

/**
 * Detect document page orientation (PP-LCNet) from encoded image bytes using a
 * caller-supplied ONNX model.
 *
 * See [`detect_layout`] for the bytes contract. Returns an `OrientationResult`
 * object with the detected rotation (0/90/180/270 degrees) and its confidence.
 */
export function detectOrientation(image_bytes: Uint8Array, model_bytes: Uint8Array): any;

/**
 * Extract content from a single bytes or URI input.
 */
export function extract(input: any, config: any): Promise<WasmExtractionResult>;

/**
 * Extract content from multiple bytes or URI inputs.
 */
export function extractBatch(inputs: WasmExtractInput[], config: any): Promise<WasmExtractionResult>;

/**
 * List names of all registered document extractors.
 */
export function listDocumentExtractors(): string[];

/**
 * List the names of all registered embedding backends.
 *
 * Used by `xberg-cli`, the api/mcp endpoints, and generated language
 * bindings.
 */
export function listEmbeddingBackends(): string[];

/**
 * List all registered OCR backends.
 *
 * Returns the names of all OCR backends currently registered in the global registry.
 *
 * # Returns
 *
 * A vector of OCR backend names.
 *
 * # Example
 */
export function listOcrBackends(): string[];

/**
 * List all registered post-processor names.
 *
 * Returns a vector of all post-processor names currently registered in the
 * global registry.
 *
 * # Returns
 *
 * - `Ok(Vec<String>)` - Vector of post-processor names
 * - `Err(...)` if the registry lock is poisoned
 *
 * # Example
 */
export function listPostProcessors(): string[];

/**
 * List names of all registered renderers.
 *
 * # Errors
 *
 * Returns an error if the registry lock is poisoned.
 */
export function listRenderers(): string[];

/**
 * List the names of all registered reranker backends.
 *
 * Used by `xberg-cli`, the api/mcp endpoints, and generated language
 * bindings.
 *
 * Since v5.0.0.
 */
export function listRerankerBackends(): string[];

/**
 * List all supported document formats.
 *
 * Returns every file extension Xberg recognizes together with its
 * corresponding MIME type, derived from the central format registry.
 * Formats that have no registered file extension (such as source code,
 * which is detected dynamically) are not included.
 *
 * The list is sorted alphabetically by file extension.
 *
 * # Returns
 *
 * A vector of `SupportedFormat` entries sorted by extension.
 *
 * # Example
 */
export function listSupportedFormats(): WasmSupportedFormat[];

/**
 * List the names of all registered tokenizer backends.
 *
 * Used by `xberg-cli`, the api/mcp endpoints, and generated language
 * bindings.
 */
export function listTokenizerBackends(): string[];

/**
 * List names of all registered validators.
 */
export function listValidators(): string[];

export function registerDocumentExtractor(backend: any): void;

export function registerEmbeddingBackend(backend: any): void;

export function registerOcrBackend(backend: any): void;

export function registerPostProcessor(backend: any): void;

export function registerRenderer(backend: any): void;

export function registerRerankerBackend(backend: any): void;

export function registerTokenizerBackend(backend: any): void;

export function registerValidator(backend: any): void;

export function unregisterDocumentExtractor(name: string): void;

export function unregisterEmbeddingBackend(name: string): void;

export function unregisterOcrBackend(name: string): void;

export function unregisterPostProcessor(name: string): void;

export function unregisterRenderer(name: string): void;

export function unregisterRerankerBackend(name: string): void;

export function unregisterTokenizerBackend(name: string): void;

export function unregisterValidator(name: string): void;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_nermodel_free: (a: number, b: number) => void;
    readonly __wbg_wasmaccelerationconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmannotationkind_free: (a: number, b: number) => void;
    readonly __wbg_wasmarchiveentry_free: (a: number, b: number) => void;
    readonly __wbg_wasmarchivemetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmauthconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmbibtexmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmboundingbox_free: (a: number, b: number) => void;
    readonly __wbg_wasmbrowserconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmcachestats_free: (a: number, b: number) => void;
    readonly __wbg_wasmcaptioningconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmcellchange_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunk_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunkclassificationconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunkclassificationdefinition_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunkingconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunkmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmchunksizing_free: (a: number, b: number) => void;
    readonly __wbg_wasmcitationmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmclassificationlabel_free: (a: number, b: number) => void;
    readonly __wbg_wasmcontentconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmcontentfilterconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmcontributorrole_free: (a: number, b: number) => void;
    readonly __wbg_wasmconversionoptions_free: (a: number, b: number) => void;
    readonly __wbg_wasmcrawlconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmcsvmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmdbfmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmdiffline_free: (a: number, b: number) => void;
    readonly __wbg_wasmdjotcontent_free: (a: number, b: number) => void;
    readonly __wbg_wasmdjotimage_free: (a: number, b: number) => void;
    readonly __wbg_wasmdocumentcounts_free: (a: number, b: number) => void;
    readonly __wbg_wasmdocumentnode_free: (a: number, b: number) => void;
    readonly __wbg_wasmdocumentrevision_free: (a: number, b: number) => void;
    readonly __wbg_wasmdocumentstructure_free: (a: number, b: number) => void;
    readonly __wbg_wasmdocumentsummary_free: (a: number, b: number) => void;
    readonly __wbg_wasmelement_free: (a: number, b: number) => void;
    readonly __wbg_wasmelementmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmemailattachment_free: (a: number, b: number) => void;
    readonly __wbg_wasmemailconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmemailextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmemailmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmembeddingconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmembeddingmodeltype_free: (a: number, b: number) => void;
    readonly __wbg_wasmentity_free: (a: number, b: number) => void;
    readonly __wbg_wasmepubmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmexcelmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmexcelsheet_free: (a: number, b: number) => void;
    readonly __wbg_wasmexcelworkbook_free: (a: number, b: number) => void;
    readonly __wbg_wasmextracteddocument_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractedimage_free: (a: number, b: number) => void;
    readonly __wbg_wasmextracteduri_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractinput_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractionerroritem_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmextractionsummary_free: (a: number, b: number) => void;
    readonly __wbg_wasmfictionbookmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmfileextractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmfootnote_free: (a: number, b: number) => void;
    readonly __wbg_wasmformatmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmformattedblock_free: (a: number, b: number) => void;
    readonly __wbg_wasmformula_free: (a: number, b: number) => void;
    readonly __wbg_wasmgridcell_free: (a: number, b: number) => void;
    readonly __wbg_wasmheadermetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmheadingcontext_free: (a: number, b: number) => void;
    readonly __wbg_wasmheadinglevel_free: (a: number, b: number) => void;
    readonly __wbg_wasmhierarchicalblock_free: (a: number, b: number) => void;
    readonly __wbg_wasmhtmlmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmimageextractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmimagemetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmimagemetadatatype_free: (a: number, b: number) => void;
    readonly __wbg_wasmimageoutputformat_free: (a: number, b: number) => void;
    readonly __wbg_wasmimagepreprocessingconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmimagepreprocessingmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasminlineelement_free: (a: number, b: number) => void;
    readonly __wbg_wasmjatsmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmlanguagedetectionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmlateinteractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmlateinteractionmodeltype_free: (a: number, b: number) => void;
    readonly __wbg_wasmlayoutregion_free: (a: number, b: number) => void;
    readonly __wbg_wasmlinkmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmllmconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmllmusage_free: (a: number, b: number) => void;
    readonly __wbg_wasmmapresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmnerconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmnodecontent_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrboundinggeometry_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrconfidence_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrelement_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrpipelineconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrpipelinestage_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrqualitythresholds_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrstrategy_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrtable_free: (a: number, b: number) => void;
    readonly __wbg_wasmocrtableboundingbox_free: (a: number, b: number) => void;
    readonly __wbg_wasmpageclassification_free: (a: number, b: number) => void;
    readonly __wbg_wasmpageclassificationconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmpagecontent_free: (a: number, b: number) => void;
    readonly __wbg_wasmpagehierarchy_free: (a: number, b: number) => void;
    readonly __wbg_wasmpageinfo_free: (a: number, b: number) => void;
    readonly __wbg_wasmpagespan_free: (a: number, b: number) => void;
    readonly __wbg_wasmpagestructure_free: (a: number, b: number) => void;
    readonly __wbg_wasmpatternmatch_free: (a: number, b: number) => void;
    readonly __wbg_wasmpdfannotation_free: (a: number, b: number) => void;
    readonly __wbg_wasmpdfformfield_free: (a: number, b: number) => void;
    readonly __wbg_wasmpostprocessorconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmpptxappproperties_free: (a: number, b: number) => void;
    readonly __wbg_wasmpptxextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmpptxmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmpropertychange_free: (a: number, b: number) => void;
    readonly __wbg_wasmpstmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmqrcode_free: (a: number, b: number) => void;
    readonly __wbg_wasmredactionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmredactionreport_free: (a: number, b: number) => void;
    readonly __wbg_wasmrerankerconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmrerankermodeltype_free: (a: number, b: number) => void;
    readonly __wbg_wasmrevisionanchor_free: (a: number, b: number) => void;
    readonly __wbg_wasmrevisiondelta_free: (a: number, b: number) => void;
    readonly __wbg_wasmsecuritylimits_free: (a: number, b: number) => void;
    readonly __wbg_wasmsitemapurl_free: (a: number, b: number) => void;
    readonly __wbg_wasmssrfpolicy_free: (a: number, b: number) => void;
    readonly __wbg_wasmstructureddata_free: (a: number, b: number) => void;
    readonly __wbg_wasmstructureddataresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmstructuredextractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmsummarizationconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmsupportedformat_free: (a: number, b: number) => void;
    readonly __wbg_wasmtable_free: (a: number, b: number) => void;
    readonly __wbg_wasmtablegrid_free: (a: number, b: number) => void;
    readonly __wbg_wasmtesseractconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmtextannotation_free: (a: number, b: number) => void;
    readonly __wbg_wasmtextextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmtextmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmtranslation_free: (a: number, b: number) => void;
    readonly __wbg_wasmtranslationconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmurlextractionconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmxlsxappproperties_free: (a: number, b: number) => void;
    readonly __wbg_wasmxmlextractionresult_free: (a: number, b: number) => void;
    readonly __wbg_wasmxmlmetadata_free: (a: number, b: number) => void;
    readonly __wbg_wasmyearrange_free: (a: number, b: number) => void;
    readonly __wbg_xbergengine_free: (a: number, b: number) => void;
    readonly classifyChunks: (a: any, b: any) => any;
    readonly clearDocumentExtractors: () => [number, number];
    readonly clearEmbeddingBackends: () => [number, number];
    readonly clearOcrBackends: () => [number, number];
    readonly clearPostProcessors: () => [number, number];
    readonly clearRenderers: () => [number, number];
    readonly clearRerankerBackends: () => [number, number];
    readonly clearTokenizerBackends: () => [number, number];
    readonly clearValidators: () => [number, number];
    readonly detectLayout: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly detectOrientation: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly extract: (a: any, b: any) => any;
    readonly extractBatch: (a: number, b: number, c: any) => any;
    readonly listDocumentExtractors: () => [number, number, number, number];
    readonly listEmbeddingBackends: () => [number, number, number, number];
    readonly listOcrBackends: () => [number, number, number, number];
    readonly listPostProcessors: () => [number, number, number, number];
    readonly listRenderers: () => [number, number, number, number];
    readonly listRerankerBackends: () => [number, number, number, number];
    readonly listSupportedFormats: () => [number, number];
    readonly listTokenizerBackends: () => [number, number, number, number];
    readonly listValidators: () => [number, number, number, number];
    readonly nermodel_detect: (a: number, b: number, c: number, d: any) => any;
    readonly nermodel_load: (a: any) => any;
    readonly registerDocumentExtractor: (a: any) => [number, number];
    readonly registerEmbeddingBackend: (a: any) => [number, number];
    readonly registerOcrBackend: (a: any) => [number, number];
    readonly registerPostProcessor: (a: any) => [number, number];
    readonly registerRenderer: (a: any) => [number, number];
    readonly registerRerankerBackend: (a: any) => [number, number];
    readonly registerTokenizerBackend: (a: any) => [number, number];
    readonly registerValidator: (a: any) => [number, number];
    readonly unregisterDocumentExtractor: (a: number, b: number) => [number, number];
    readonly unregisterEmbeddingBackend: (a: number, b: number) => [number, number];
    readonly unregisterOcrBackend: (a: number, b: number) => [number, number];
    readonly unregisterPostProcessor: (a: number, b: number) => [number, number];
    readonly unregisterRenderer: (a: number, b: number) => [number, number];
    readonly unregisterRerankerBackend: (a: number, b: number) => [number, number];
    readonly unregisterTokenizerBackend: (a: number, b: number) => [number, number];
    readonly unregisterValidator: (a: number, b: number) => [number, number];
    readonly wasmaccelerationconfig_default: () => number;
    readonly wasmaccelerationconfig_deviceId: (a: number) => number;
    readonly wasmaccelerationconfig_new: (a: number, b: number) => number;
    readonly wasmaccelerationconfig_provider: (a: number) => [number, number];
    readonly wasmaccelerationconfig_set_deviceId: (a: number, b: number) => void;
    readonly wasmaccelerationconfig_set_provider: (a: number, b: number) => void;
    readonly wasmannotationkind_annotationType: (a: number) => [number, number];
    readonly wasmannotationkind_default: () => number;
    readonly wasmannotationkind_name: (a: number) => [number, number];
    readonly wasmannotationkind_set_annotationType: (a: number, b: number, c: number) => void;
    readonly wasmannotationkind_set_name: (a: number, b: number, c: number) => void;
    readonly wasmannotationkind_set_title: (a: number, b: number, c: number) => void;
    readonly wasmannotationkind_set_url: (a: number, b: number, c: number) => void;
    readonly wasmannotationkind_set_value: (a: number, b: number, c: number) => void;
    readonly wasmannotationkind_title: (a: number) => [number, number];
    readonly wasmannotationkind_url: (a: number) => [number, number];
    readonly wasmannotationkind_value: (a: number) => [number, number];
    readonly wasmarchiveentry_default: () => number;
    readonly wasmarchiveentry_mimeType: (a: number) => [number, number];
    readonly wasmarchiveentry_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmarchiveentry_path: (a: number) => [number, number];
    readonly wasmarchiveentry_result: (a: number) => number;
    readonly wasmarchiveentry_set_mimeType: (a: number, b: number, c: number) => void;
    readonly wasmarchiveentry_set_path: (a: number, b: number, c: number) => void;
    readonly wasmarchiveentry_set_result: (a: number, b: number) => void;
    readonly wasmarchivemetadata_compressedSize: (a: number) => [number, bigint];
    readonly wasmarchivemetadata_default: () => number;
    readonly wasmarchivemetadata_fileCount: (a: number) => number;
    readonly wasmarchivemetadata_fileList: (a: number) => [number, number];
    readonly wasmarchivemetadata_format: (a: number) => [number, number];
    readonly wasmarchivemetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: bigint, h: number, i: bigint) => number;
    readonly wasmarchivemetadata_set_compressedSize: (a: number, b: number, c: bigint) => void;
    readonly wasmarchivemetadata_set_fileCount: (a: number, b: number) => void;
    readonly wasmarchivemetadata_set_fileList: (a: number, b: number, c: number) => void;
    readonly wasmarchivemetadata_set_format: (a: number, b: number, c: number) => void;
    readonly wasmarchivemetadata_set_totalSize: (a: number, b: bigint) => void;
    readonly wasmarchivemetadata_totalSize: (a: number) => bigint;
    readonly wasmauthconfig_default: () => number;
    readonly wasmauthconfig_name: (a: number) => [number, number];
    readonly wasmauthconfig_password: (a: number) => [number, number];
    readonly wasmauthconfig_set_name: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_set_password: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_set_token: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_set_type: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_set_username: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_set_value: (a: number, b: number, c: number) => void;
    readonly wasmauthconfig_token: (a: number) => [number, number];
    readonly wasmauthconfig_type: (a: number) => [number, number];
    readonly wasmauthconfig_username: (a: number) => [number, number];
    readonly wasmauthconfig_value: (a: number) => [number, number];
    readonly wasmbibtexmetadata_authors: (a: number) => [number, number];
    readonly wasmbibtexmetadata_citationKeys: (a: number) => [number, number];
    readonly wasmbibtexmetadata_default: () => number;
    readonly wasmbibtexmetadata_entryCount: (a: number) => number;
    readonly wasmbibtexmetadata_entryTypes: (a: number) => any;
    readonly wasmbibtexmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmbibtexmetadata_set_authors: (a: number, b: number, c: number) => void;
    readonly wasmbibtexmetadata_set_citationKeys: (a: number, b: number, c: number) => void;
    readonly wasmbibtexmetadata_set_entryCount: (a: number, b: number) => void;
    readonly wasmbibtexmetadata_set_entryTypes: (a: number, b: number) => void;
    readonly wasmbibtexmetadata_set_yearRange: (a: number, b: number) => void;
    readonly wasmbibtexmetadata_yearRange: (a: number) => number;
    readonly wasmboundingbox_default: () => number;
    readonly wasmboundingbox_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmboundingbox_set_x0: (a: number, b: number) => void;
    readonly wasmboundingbox_set_x1: (a: number, b: number) => void;
    readonly wasmboundingbox_set_y0: (a: number, b: number) => void;
    readonly wasmboundingbox_set_y1: (a: number, b: number) => void;
    readonly wasmboundingbox_x0: (a: number) => number;
    readonly wasmboundingbox_x1: (a: number) => number;
    readonly wasmboundingbox_y0: (a: number) => number;
    readonly wasmboundingbox_y1: (a: number) => number;
    readonly wasmbrowserconfig_backend: (a: number) => [number, number];
    readonly wasmbrowserconfig_blockUrlPatterns: (a: number) => [number, number];
    readonly wasmbrowserconfig_captureNetworkEvents: (a: number) => number;
    readonly wasmbrowserconfig_default: () => number;
    readonly wasmbrowserconfig_endpoint: (a: number) => [number, number];
    readonly wasmbrowserconfig_evalScript: (a: number) => [number, number];
    readonly wasmbrowserconfig_extraWait: (a: number) => [number, bigint];
    readonly wasmbrowserconfig_mode: (a: number) => [number, number];
    readonly wasmbrowserconfig_new: (a: number, b: number, c: number, d: bigint, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: bigint, p: number, q: number, r: number, s: number, t: number) => number;
    readonly wasmbrowserconfig_proxy: (a: number) => number;
    readonly wasmbrowserconfig_robotsUserAgent: (a: number) => [number, number];
    readonly wasmbrowserconfig_sessionAffinity: (a: number) => number;
    readonly wasmbrowserconfig_set_backend: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_blockUrlPatterns: (a: number, b: number, c: number) => void;
    readonly wasmbrowserconfig_set_captureNetworkEvents: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_endpoint: (a: number, b: number, c: number) => void;
    readonly wasmbrowserconfig_set_evalScript: (a: number, b: number, c: number) => void;
    readonly wasmbrowserconfig_set_extraWait: (a: number, b: number, c: bigint) => void;
    readonly wasmbrowserconfig_set_mode: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_proxy: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_robotsUserAgent: (a: number, b: number, c: number) => void;
    readonly wasmbrowserconfig_set_sessionAffinity: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_timeout: (a: number, b: number, c: bigint) => void;
    readonly wasmbrowserconfig_set_wait: (a: number, b: number) => void;
    readonly wasmbrowserconfig_set_waitSelector: (a: number, b: number, c: number) => void;
    readonly wasmbrowserconfig_timeout: (a: number) => [number, bigint];
    readonly wasmbrowserconfig_wait: (a: number) => [number, number];
    readonly wasmbrowserconfig_waitSelector: (a: number) => [number, number];
    readonly wasmcachestats_availableSpaceMb: (a: number) => number;
    readonly wasmcachestats_default: () => number;
    readonly wasmcachestats_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmcachestats_newestFileAgeDays: (a: number) => number;
    readonly wasmcachestats_oldestFileAgeDays: (a: number) => number;
    readonly wasmcachestats_set_availableSpaceMb: (a: number, b: number) => void;
    readonly wasmcachestats_set_newestFileAgeDays: (a: number, b: number) => void;
    readonly wasmcachestats_set_oldestFileAgeDays: (a: number, b: number) => void;
    readonly wasmcachestats_set_totalFiles: (a: number, b: number) => void;
    readonly wasmcachestats_set_totalSizeMb: (a: number, b: number) => void;
    readonly wasmcachestats_totalFiles: (a: number) => number;
    readonly wasmcachestats_totalSizeMb: (a: number) => number;
    readonly wasmcaptioningconfig_default: () => number;
    readonly wasmcaptioningconfig_llm: (a: number) => number;
    readonly wasmcaptioningconfig_minImageArea: (a: number) => number;
    readonly wasmcaptioningconfig_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmcaptioningconfig_prompt: (a: number) => [number, number];
    readonly wasmcaptioningconfig_set_llm: (a: number, b: number) => void;
    readonly wasmcaptioningconfig_set_minImageArea: (a: number, b: number) => void;
    readonly wasmcaptioningconfig_set_prompt: (a: number, b: number, c: number) => void;
    readonly wasmcellchange_col: (a: number) => number;
    readonly wasmcellchange_default: () => number;
    readonly wasmcellchange_from: (a: number) => [number, number];
    readonly wasmcellchange_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmcellchange_row: (a: number) => number;
    readonly wasmcellchange_set_col: (a: number, b: number) => void;
    readonly wasmcellchange_set_from: (a: number, b: number, c: number) => void;
    readonly wasmcellchange_set_row: (a: number, b: number) => void;
    readonly wasmcellchange_set_to: (a: number, b: number, c: number) => void;
    readonly wasmcellchange_to: (a: number) => [number, number];
    readonly wasmchunk_chunkType: (a: number) => [number, number];
    readonly wasmchunk_content: (a: number) => [number, number];
    readonly wasmchunk_default: () => number;
    readonly wasmchunk_embedding: (a: number) => [number, number];
    readonly wasmchunk_metadata: (a: number) => number;
    readonly wasmchunk_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmchunk_set_chunkType: (a: number, b: number) => void;
    readonly wasmchunk_set_content: (a: number, b: number, c: number) => void;
    readonly wasmchunk_set_embedding: (a: number, b: number, c: number) => void;
    readonly wasmchunk_set_metadata: (a: number, b: number) => void;
    readonly wasmchunkclassificationconfig_batchSize: (a: number) => number;
    readonly wasmchunkclassificationconfig_default: () => number;
    readonly wasmchunkclassificationconfig_definitions: (a: number) => [number, number];
    readonly wasmchunkclassificationconfig_llm: (a: number) => number;
    readonly wasmchunkclassificationconfig_maxConcurrency: (a: number) => number;
    readonly wasmchunkclassificationconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmchunkclassificationconfig_promptTemplate: (a: number) => [number, number];
    readonly wasmchunkclassificationconfig_set_batchSize: (a: number, b: number) => void;
    readonly wasmchunkclassificationconfig_set_definitions: (a: number, b: number, c: number) => void;
    readonly wasmchunkclassificationconfig_set_llm: (a: number, b: number) => void;
    readonly wasmchunkclassificationconfig_set_maxConcurrency: (a: number, b: number) => void;
    readonly wasmchunkclassificationconfig_set_promptTemplate: (a: number, b: number, c: number) => void;
    readonly wasmchunkclassificationdefinition_default: () => number;
    readonly wasmchunkclassificationdefinition_description: (a: number) => [number, number];
    readonly wasmchunkclassificationdefinition_label: (a: number) => [number, number];
    readonly wasmchunkclassificationdefinition_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmchunkclassificationdefinition_set_description: (a: number, b: number, c: number) => void;
    readonly wasmchunkclassificationdefinition_set_label: (a: number, b: number, c: number) => void;
    readonly wasmchunkingconfig_chunkerType: (a: number) => [number, number];
    readonly wasmchunkingconfig_default: () => number;
    readonly wasmchunkingconfig_embedding: (a: number) => number;
    readonly wasmchunkingconfig_maxCharacters: (a: number) => number;
    readonly wasmchunkingconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => number;
    readonly wasmchunkingconfig_overlap: (a: number) => number;
    readonly wasmchunkingconfig_prependHeadingContext: (a: number) => number;
    readonly wasmchunkingconfig_preset: (a: number) => [number, number];
    readonly wasmchunkingconfig_set_chunkerType: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_embedding: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_maxCharacters: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_overlap: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_prependHeadingContext: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_preset: (a: number, b: number, c: number) => void;
    readonly wasmchunkingconfig_set_sizing: (a: number, b: any) => void;
    readonly wasmchunkingconfig_set_tableChunking: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_topicThreshold: (a: number, b: number) => void;
    readonly wasmchunkingconfig_set_trim: (a: number, b: number) => void;
    readonly wasmchunkingconfig_sizing: (a: number) => any;
    readonly wasmchunkingconfig_tableChunking: (a: number) => [number, number];
    readonly wasmchunkingconfig_topicThreshold: (a: number) => number;
    readonly wasmchunkingconfig_trim: (a: number) => number;
    readonly wasmchunkmetadata_byteEnd: (a: number) => number;
    readonly wasmchunkmetadata_byteStart: (a: number) => number;
    readonly wasmchunkmetadata_chunkIndex: (a: number) => number;
    readonly wasmchunkmetadata_classifications: (a: number) => [number, number];
    readonly wasmchunkmetadata_default: () => number;
    readonly wasmchunkmetadata_firstPage: (a: number) => number;
    readonly wasmchunkmetadata_headingContext: (a: number) => number;
    readonly wasmchunkmetadata_headingPath: (a: number) => [number, number];
    readonly wasmchunkmetadata_imageIndices: (a: number) => [number, number];
    readonly wasmchunkmetadata_lastPage: (a: number) => number;
    readonly wasmchunkmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number) => number;
    readonly wasmchunkmetadata_nodeIds: (a: number) => [number, number];
    readonly wasmchunkmetadata_pageSpans: (a: number) => [number, number];
    readonly wasmchunkmetadata_set_byteEnd: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_byteStart: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_chunkIndex: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_classifications: (a: number, b: number, c: number) => void;
    readonly wasmchunkmetadata_set_firstPage: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_headingContext: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_headingPath: (a: number, b: number, c: number) => void;
    readonly wasmchunkmetadata_set_imageIndices: (a: number, b: number, c: number) => void;
    readonly wasmchunkmetadata_set_lastPage: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_nodeIds: (a: number, b: number, c: number) => void;
    readonly wasmchunkmetadata_set_pageSpans: (a: number, b: number, c: number) => void;
    readonly wasmchunkmetadata_set_tokenCount: (a: number, b: number) => void;
    readonly wasmchunkmetadata_set_totalChunks: (a: number, b: number) => void;
    readonly wasmchunkmetadata_tokenCount: (a: number) => number;
    readonly wasmchunkmetadata_totalChunks: (a: number) => number;
    readonly wasmchunksizing_cacheDir: (a: number) => [number, number];
    readonly wasmchunksizing_default: () => number;
    readonly wasmchunksizing_model: (a: number) => [number, number];
    readonly wasmchunksizing_set_cacheDir: (a: number, b: number, c: number) => void;
    readonly wasmchunksizing_set_model: (a: number, b: number, c: number) => void;
    readonly wasmchunksizing_set_type: (a: number, b: number, c: number) => void;
    readonly wasmchunksizing_type: (a: number) => [number, number];
    readonly wasmcitationmetadata_authors: (a: number) => [number, number];
    readonly wasmcitationmetadata_citationCount: (a: number) => number;
    readonly wasmcitationmetadata_default: () => number;
    readonly wasmcitationmetadata_dois: (a: number) => [number, number];
    readonly wasmcitationmetadata_format: (a: number) => [number, number];
    readonly wasmcitationmetadata_keywords: (a: number) => [number, number];
    readonly wasmcitationmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmcitationmetadata_set_authors: (a: number, b: number, c: number) => void;
    readonly wasmcitationmetadata_set_citationCount: (a: number, b: number) => void;
    readonly wasmcitationmetadata_set_dois: (a: number, b: number, c: number) => void;
    readonly wasmcitationmetadata_set_format: (a: number, b: number, c: number) => void;
    readonly wasmcitationmetadata_set_keywords: (a: number, b: number, c: number) => void;
    readonly wasmcitationmetadata_set_yearRange: (a: number, b: number) => void;
    readonly wasmcitationmetadata_yearRange: (a: number) => number;
    readonly wasmclassificationlabel_confidence: (a: number) => number;
    readonly wasmclassificationlabel_default: () => number;
    readonly wasmclassificationlabel_label: (a: number) => [number, number];
    readonly wasmclassificationlabel_new: (a: number, b: number, c: number) => number;
    readonly wasmclassificationlabel_set_confidence: (a: number, b: number) => void;
    readonly wasmclassificationlabel_set_label: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_default: () => number;
    readonly wasmcontentconfig_excludeSelectors: (a: number) => [number, number];
    readonly wasmcontentconfig_includeDocumentStructure: (a: number) => number;
    readonly wasmcontentconfig_maxDepth: (a: number) => number;
    readonly wasmcontentconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number) => number;
    readonly wasmcontentconfig_outputFormat: (a: number) => [number, number];
    readonly wasmcontentconfig_preprocessingPreset: (a: number) => [number, number];
    readonly wasmcontentconfig_preserveTags: (a: number) => [number, number];
    readonly wasmcontentconfig_removeForms: (a: number) => number;
    readonly wasmcontentconfig_removeNavigation: (a: number) => number;
    readonly wasmcontentconfig_set_excludeSelectors: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_set_includeDocumentStructure: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_maxDepth: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_outputFormat: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_set_preprocessingPreset: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_set_preserveTags: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_set_removeForms: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_removeNavigation: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_skipImages: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_stripTags: (a: number, b: number, c: number) => void;
    readonly wasmcontentconfig_set_wrap: (a: number, b: number) => void;
    readonly wasmcontentconfig_set_wrapWidth: (a: number, b: number) => void;
    readonly wasmcontentconfig_skipImages: (a: number) => number;
    readonly wasmcontentconfig_stripTags: (a: number) => [number, number];
    readonly wasmcontentconfig_wrap: (a: number) => number;
    readonly wasmcontentconfig_wrapWidth: (a: number) => number;
    readonly wasmcontentfilterconfig_default: () => number;
    readonly wasmcontentfilterconfig_includeFooters: (a: number) => number;
    readonly wasmcontentfilterconfig_includeHeaders: (a: number) => number;
    readonly wasmcontentfilterconfig_includeWatermarks: (a: number) => number;
    readonly wasmcontentfilterconfig_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmcontentfilterconfig_set_includeFooters: (a: number, b: number) => void;
    readonly wasmcontentfilterconfig_set_includeHeaders: (a: number, b: number) => void;
    readonly wasmcontentfilterconfig_set_includeWatermarks: (a: number, b: number) => void;
    readonly wasmcontentfilterconfig_set_stripRepeatingText: (a: number, b: number) => void;
    readonly wasmcontentfilterconfig_stripRepeatingText: (a: number) => number;
    readonly wasmcontributorrole_default: () => number;
    readonly wasmcontributorrole_name: (a: number) => [number, number];
    readonly wasmcontributorrole_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmcontributorrole_role: (a: number) => [number, number];
    readonly wasmcontributorrole_set_name: (a: number, b: number, c: number) => void;
    readonly wasmcontributorrole_set_role: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_autolinks: (a: number) => number;
    readonly wasmconversionoptions_brInTables: (a: number) => number;
    readonly wasmconversionoptions_bullets: (a: number) => [number, number];
    readonly wasmconversionoptions_captureSvg: (a: number) => number;
    readonly wasmconversionoptions_codeBlockStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_codeLanguage: (a: number) => [number, number];
    readonly wasmconversionoptions_compactTables: (a: number) => number;
    readonly wasmconversionoptions_convertAsInline: (a: number) => number;
    readonly wasmconversionoptions_debug: (a: number) => number;
    readonly wasmconversionoptions_default: () => number;
    readonly wasmconversionoptions_defaultTitle: (a: number) => number;
    readonly wasmconversionoptions_encoding: (a: number) => [number, number];
    readonly wasmconversionoptions_escapeAscii: (a: number) => number;
    readonly wasmconversionoptions_escapeAsterisks: (a: number) => number;
    readonly wasmconversionoptions_escapeMisc: (a: number) => number;
    readonly wasmconversionoptions_escapeUnderscores: (a: number) => number;
    readonly wasmconversionoptions_excludeSelectors: (a: number) => [number, number];
    readonly wasmconversionoptions_extractMetadata: (a: number) => number;
    readonly wasmconversionoptions_headingStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_highlightStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_inferDimensions: (a: number) => number;
    readonly wasmconversionoptions_keepInlineImagesIn: (a: number) => [number, number];
    readonly wasmconversionoptions_linkStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_listIndentType: (a: number) => [number, number];
    readonly wasmconversionoptions_listIndentWidth: (a: number) => number;
    readonly wasmconversionoptions_maxDepth: (a: number) => number;
    readonly wasmconversionoptions_maxImageSize: (a: number) => bigint;
    readonly wasmconversionoptions_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number, c1: number, d1: number, e1: number, f1: number, g1: number, h1: number, i1: number, j1: number, k1: number, l1: number, m1: number, n1: number, o1: number, p1: number, q1: number, r1: number, s1: bigint, t1: number, u1: number, v1: number, w1: number, x1: number) => number;
    readonly wasmconversionoptions_newlineStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_preprocessing: (a: number) => number;
    readonly wasmconversionoptions_preserveTags: (a: number) => [number, number];
    readonly wasmconversionoptions_set_autolinks: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_brInTables: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_bullets: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_captureSvg: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_codeBlockStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_codeLanguage: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_compactTables: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_convertAsInline: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_debug: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_defaultTitle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_encoding: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_escapeAscii: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_escapeAsterisks: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_escapeMisc: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_escapeUnderscores: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_excludeSelectors: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_extractMetadata: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_headingStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_highlightStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_inferDimensions: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_keepInlineImagesIn: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_linkStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_listIndentType: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_listIndentWidth: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_maxDepth: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_maxImageSize: (a: number, b: bigint) => void;
    readonly wasmconversionoptions_set_newlineStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_preprocessing: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_preserveTags: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_skipImages: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_stripNewlines: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_stripTags: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_strongEmSymbol: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_subSymbol: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_supSymbol: (a: number, b: number, c: number) => void;
    readonly wasmconversionoptions_set_urlEscapeStyle: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_whitespaceMode: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_wrap: (a: number, b: number) => void;
    readonly wasmconversionoptions_set_wrapWidth: (a: number, b: number) => void;
    readonly wasmconversionoptions_skipImages: (a: number) => number;
    readonly wasmconversionoptions_stripNewlines: (a: number) => number;
    readonly wasmconversionoptions_stripTags: (a: number) => [number, number];
    readonly wasmconversionoptions_strongEmSymbol: (a: number) => [number, number];
    readonly wasmconversionoptions_subSymbol: (a: number) => [number, number];
    readonly wasmconversionoptions_supSymbol: (a: number) => [number, number];
    readonly wasmconversionoptions_urlEscapeStyle: (a: number) => [number, number];
    readonly wasmconversionoptions_whitespaceMode: (a: number) => [number, number];
    readonly wasmconversionoptions_wrap: (a: number) => number;
    readonly wasmconversionoptions_wrapWidth: (a: number) => number;
    readonly wasmcrawlconfig_allowSubdomains: (a: number) => number;
    readonly wasmcrawlconfig_assetTypes: (a: number) => [number, number];
    readonly wasmcrawlconfig_auth: (a: number) => any;
    readonly wasmcrawlconfig_browser: (a: number) => number;
    readonly wasmcrawlconfig_browserProfile: (a: number) => [number, number];
    readonly wasmcrawlconfig_captureScreenshot: (a: number) => number;
    readonly wasmcrawlconfig_content: (a: number) => number;
    readonly wasmcrawlconfig_cookiesEnabled: (a: number) => number;
    readonly wasmcrawlconfig_customHeaders: (a: number) => any;
    readonly wasmcrawlconfig_default: () => number;
    readonly wasmcrawlconfig_documentMaxSize: (a: number) => number;
    readonly wasmcrawlconfig_documentMimeTypes: (a: number) => [number, number];
    readonly wasmcrawlconfig_documentUrlDepth: (a: number) => number;
    readonly wasmcrawlconfig_downloadAssets: (a: number) => number;
    readonly wasmcrawlconfig_downloadDocuments: (a: number) => number;
    readonly wasmcrawlconfig_excludePaths: (a: number) => [number, number];
    readonly wasmcrawlconfig_followDocumentUrls: (a: number) => number;
    readonly wasmcrawlconfig_includePaths: (a: number) => [number, number];
    readonly wasmcrawlconfig_mapLimit: (a: number) => number;
    readonly wasmcrawlconfig_mapSearch: (a: number) => [number, number];
    readonly wasmcrawlconfig_maxAssetSize: (a: number) => number;
    readonly wasmcrawlconfig_maxBodySize: (a: number) => number;
    readonly wasmcrawlconfig_maxConcurrent: (a: number) => number;
    readonly wasmcrawlconfig_maxDepth: (a: number) => number;
    readonly wasmcrawlconfig_maxPages: (a: number) => number;
    readonly wasmcrawlconfig_maxRedirects: (a: number) => number;
    readonly wasmcrawlconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: bigint, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number, c1: number, d1: number, e1: number, f1: number, g1: number, h1: number, i1: number, j1: number, k1: number, l1: number, m1: bigint, n1: number, o1: number, p1: number, q1: number, r1: number, s1: number, t1: number, u1: number, v1: number, w1: number, x1: number, y1: number, z1: number) => number;
    readonly wasmcrawlconfig_proxy: (a: number) => number;
    readonly wasmcrawlconfig_rateLimitMs: (a: number) => [number, bigint];
    readonly wasmcrawlconfig_removeTags: (a: number) => [number, number];
    readonly wasmcrawlconfig_requestTimeout: (a: number) => [number, bigint];
    readonly wasmcrawlconfig_respectRobotsTxt: (a: number) => number;
    readonly wasmcrawlconfig_retryCodes: (a: number) => [number, number];
    readonly wasmcrawlconfig_retryCount: (a: number) => number;
    readonly wasmcrawlconfig_saveBrowserProfile: (a: number) => number;
    readonly wasmcrawlconfig_set_allowSubdomains: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_assetTypes: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_auth: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_browser: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_browserProfile: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_captureScreenshot: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_content: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_cookiesEnabled: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_customHeaders: (a: number, b: any) => void;
    readonly wasmcrawlconfig_set_documentMaxSize: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_documentMimeTypes: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_documentUrlDepth: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_downloadAssets: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_downloadDocuments: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_excludePaths: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_followDocumentUrls: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_includePaths: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_mapLimit: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_mapSearch: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_maxAssetSize: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_maxBodySize: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_maxConcurrent: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_maxDepth: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_maxPages: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_maxRedirects: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_proxy: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_rateLimitMs: (a: number, b: number, c: bigint) => void;
    readonly wasmcrawlconfig_set_removeTags: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_requestTimeout: (a: number, b: number, c: bigint) => void;
    readonly wasmcrawlconfig_set_respectRobotsTxt: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_retryCodes: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_retryCount: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_saveBrowserProfile: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_softHttpErrors: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_ssrf: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_stayOnDomain: (a: number, b: number) => void;
    readonly wasmcrawlconfig_set_userAgent: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_userAgents: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_set_warcOutput: (a: number, b: number, c: number) => void;
    readonly wasmcrawlconfig_softHttpErrors: (a: number) => number;
    readonly wasmcrawlconfig_ssrf: (a: number) => number;
    readonly wasmcrawlconfig_stayOnDomain: (a: number) => number;
    readonly wasmcrawlconfig_userAgent: (a: number) => [number, number];
    readonly wasmcrawlconfig_userAgents: (a: number) => [number, number];
    readonly wasmcrawlconfig_warcOutput: (a: number) => [number, number];
    readonly wasmcsvmetadata_columnCount: (a: number) => number;
    readonly wasmcsvmetadata_columnTypes: (a: number) => [number, number];
    readonly wasmcsvmetadata_default: () => number;
    readonly wasmcsvmetadata_delimiter: (a: number) => [number, number];
    readonly wasmcsvmetadata_hasHeader: (a: number) => number;
    readonly wasmcsvmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmcsvmetadata_rowCount: (a: number) => number;
    readonly wasmcsvmetadata_set_columnCount: (a: number, b: number) => void;
    readonly wasmcsvmetadata_set_columnTypes: (a: number, b: number, c: number) => void;
    readonly wasmcsvmetadata_set_delimiter: (a: number, b: number, c: number) => void;
    readonly wasmcsvmetadata_set_hasHeader: (a: number, b: number) => void;
    readonly wasmcsvmetadata_set_rowCount: (a: number, b: number) => void;
    readonly wasmdbfmetadata_default: () => number;
    readonly wasmdbfmetadata_fieldCount: (a: number) => number;
    readonly wasmdbfmetadata_fields: (a: number) => [number, number];
    readonly wasmdbfmetadata_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmdbfmetadata_recordCount: (a: number) => number;
    readonly wasmdbfmetadata_set_fieldCount: (a: number, b: number) => void;
    readonly wasmdbfmetadata_set_fields: (a: number, b: number, c: number) => void;
    readonly wasmdbfmetadata_set_recordCount: (a: number, b: number) => void;
    readonly wasmdiffline_default: () => number;
    readonly wasmdjotcontent_blocks: (a: number) => [number, number];
    readonly wasmdjotcontent_default: () => number;
    readonly wasmdjotcontent_footnotes: (a: number) => [number, number];
    readonly wasmdjotcontent_images: (a: number) => [number, number];
    readonly wasmdjotcontent_links: (a: number) => [number, number];
    readonly wasmdjotcontent_metadata: (a: number) => number;
    readonly wasmdjotcontent_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => number;
    readonly wasmdjotcontent_plainText: (a: number) => [number, number];
    readonly wasmdjotcontent_set_blocks: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_set_footnotes: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_set_images: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_set_links: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_set_metadata: (a: number, b: number) => void;
    readonly wasmdjotcontent_set_plainText: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_set_tables: (a: number, b: number, c: number) => void;
    readonly wasmdjotcontent_tables: (a: number) => [number, number];
    readonly wasmdjotimage_alt: (a: number) => [number, number];
    readonly wasmdjotimage_default: () => number;
    readonly wasmdjotimage_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmdjotimage_set_alt: (a: number, b: number, c: number) => void;
    readonly wasmdjotimage_set_src: (a: number, b: number, c: number) => void;
    readonly wasmdjotimage_set_title: (a: number, b: number, c: number) => void;
    readonly wasmdjotimage_src: (a: number) => [number, number];
    readonly wasmdjotimage_title: (a: number) => [number, number];
    readonly wasmdocumentcounts_default: () => number;
    readonly wasmdocumentcounts_images: (a: number) => number;
    readonly wasmdocumentcounts_new: (a: number, b: number, c: number) => number;
    readonly wasmdocumentcounts_pages: (a: number) => number;
    readonly wasmdocumentcounts_set_images: (a: number, b: number) => void;
    readonly wasmdocumentcounts_set_pages: (a: number, b: number) => void;
    readonly wasmdocumentcounts_set_tables: (a: number, b: number) => void;
    readonly wasmdocumentcounts_tables: (a: number) => number;
    readonly wasmdocumentnode_annotations: (a: number) => [number, number];
    readonly wasmdocumentnode_attributes: (a: number) => any;
    readonly wasmdocumentnode_bbox: (a: number) => number;
    readonly wasmdocumentnode_children: (a: number) => [number, number];
    readonly wasmdocumentnode_content: (a: number) => any;
    readonly wasmdocumentnode_contentLayer: (a: number) => [number, number];
    readonly wasmdocumentnode_default: () => number;
    readonly wasmdocumentnode_id: (a: number) => [number, number];
    readonly wasmdocumentnode_new: (a: number, b: number, c: any, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => number;
    readonly wasmdocumentnode_page: (a: number) => number;
    readonly wasmdocumentnode_pageEnd: (a: number) => number;
    readonly wasmdocumentnode_parent: (a: number) => number;
    readonly wasmdocumentnode_set_annotations: (a: number, b: number, c: number) => void;
    readonly wasmdocumentnode_set_attributes: (a: number, b: number) => void;
    readonly wasmdocumentnode_set_bbox: (a: number, b: number) => void;
    readonly wasmdocumentnode_set_children: (a: number, b: number, c: number) => void;
    readonly wasmdocumentnode_set_content: (a: number, b: any) => void;
    readonly wasmdocumentnode_set_contentLayer: (a: number, b: number) => void;
    readonly wasmdocumentnode_set_id: (a: number, b: number, c: number) => void;
    readonly wasmdocumentnode_set_page: (a: number, b: number) => void;
    readonly wasmdocumentnode_set_pageEnd: (a: number, b: number) => void;
    readonly wasmdocumentnode_set_parent: (a: number, b: number) => void;
    readonly wasmdocumentrelationship_default: () => number;
    readonly wasmdocumentrelationship_kind: (a: number) => [number, number];
    readonly wasmdocumentrelationship_new: (a: number, b: number, c: number) => number;
    readonly wasmdocumentrelationship_set_kind: (a: number, b: number) => void;
    readonly wasmdocumentrevision_anchor: (a: number) => any;
    readonly wasmdocumentrevision_author: (a: number) => [number, number];
    readonly wasmdocumentrevision_default: () => number;
    readonly wasmdocumentrevision_delta: (a: number) => number;
    readonly wasmdocumentrevision_kind: (a: number) => [number, number];
    readonly wasmdocumentrevision_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmdocumentrevision_revisionId: (a: number) => [number, number];
    readonly wasmdocumentrevision_set_anchor: (a: number, b: number) => void;
    readonly wasmdocumentrevision_set_author: (a: number, b: number, c: number) => void;
    readonly wasmdocumentrevision_set_delta: (a: number, b: number) => void;
    readonly wasmdocumentrevision_set_kind: (a: number, b: number) => void;
    readonly wasmdocumentrevision_set_revisionId: (a: number, b: number, c: number) => void;
    readonly wasmdocumentrevision_set_timestamp: (a: number, b: number, c: number) => void;
    readonly wasmdocumentrevision_timestamp: (a: number) => [number, number];
    readonly wasmdocumentstructure_default: () => number;
    readonly wasmdocumentstructure_finalizeNodeTypes: (a: number) => void;
    readonly wasmdocumentstructure_isEmpty: (a: number) => number;
    readonly wasmdocumentstructure_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmdocumentstructure_nodeTypes: (a: number) => [number, number];
    readonly wasmdocumentstructure_nodes: (a: number) => [number, number];
    readonly wasmdocumentstructure_relationships: (a: number) => [number, number];
    readonly wasmdocumentstructure_set_nodeTypes: (a: number, b: number, c: number) => void;
    readonly wasmdocumentstructure_set_nodes: (a: number, b: number, c: number) => void;
    readonly wasmdocumentstructure_set_relationships: (a: number, b: number, c: number) => void;
    readonly wasmdocumentstructure_set_sourceFormat: (a: number, b: number, c: number) => void;
    readonly wasmdocumentstructure_sourceFormat: (a: number) => [number, number];
    readonly wasmdocumentsummary_default: () => number;
    readonly wasmdocumentsummary_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmdocumentsummary_set_strategy: (a: number, b: number) => void;
    readonly wasmdocumentsummary_set_text: (a: number, b: number, c: number) => void;
    readonly wasmdocumentsummary_set_tokenCount: (a: number, b: number) => void;
    readonly wasmdocumentsummary_strategy: (a: number) => [number, number];
    readonly wasmdocumentsummary_text: (a: number) => [number, number];
    readonly wasmdocumentsummary_tokenCount: (a: number) => number;
    readonly wasmelement_default: () => number;
    readonly wasmelement_elementType: (a: number) => [number, number];
    readonly wasmelement_metadata: (a: number) => number;
    readonly wasmelement_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmelement_set_elementType: (a: number, b: number) => void;
    readonly wasmelement_set_metadata: (a: number, b: number) => void;
    readonly wasmelement_set_text: (a: number, b: number, c: number) => void;
    readonly wasmelement_text: (a: number) => [number, number];
    readonly wasmelementmetadata_additional: (a: number) => any;
    readonly wasmelementmetadata_coordinates: (a: number) => number;
    readonly wasmelementmetadata_default: () => number;
    readonly wasmelementmetadata_elementIndex: (a: number) => number;
    readonly wasmelementmetadata_filename: (a: number) => [number, number];
    readonly wasmelementmetadata_new: (a: any, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmelementmetadata_pageNumber: (a: number) => number;
    readonly wasmelementmetadata_set_additional: (a: number, b: any) => void;
    readonly wasmelementmetadata_set_coordinates: (a: number, b: number) => void;
    readonly wasmelementmetadata_set_elementIndex: (a: number, b: number) => void;
    readonly wasmelementmetadata_set_filename: (a: number, b: number, c: number) => void;
    readonly wasmelementmetadata_set_pageNumber: (a: number, b: number) => void;
    readonly wasmemailattachment_data: (a: number) => [number, number];
    readonly wasmemailattachment_default: () => number;
    readonly wasmemailattachment_filename: (a: number) => [number, number];
    readonly wasmemailattachment_isImage: (a: number) => number;
    readonly wasmemailattachment_mimeType: (a: number) => [number, number];
    readonly wasmemailattachment_name: (a: number) => [number, number];
    readonly wasmemailattachment_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmemailattachment_set_data: (a: number, b: number, c: number) => void;
    readonly wasmemailattachment_set_filename: (a: number, b: number, c: number) => void;
    readonly wasmemailattachment_set_isImage: (a: number, b: number) => void;
    readonly wasmemailattachment_set_mimeType: (a: number, b: number, c: number) => void;
    readonly wasmemailattachment_set_name: (a: number, b: number, c: number) => void;
    readonly wasmemailattachment_set_size: (a: number, b: number) => void;
    readonly wasmemailattachment_size: (a: number) => number;
    readonly wasmemailconfig_default: () => number;
    readonly wasmemailconfig_msgFallbackCodepage: (a: number) => number;
    readonly wasmemailconfig_new: (a: number) => number;
    readonly wasmemailconfig_set_msgFallbackCodepage: (a: number, b: number) => void;
    readonly wasmemailextractionresult_attachments: (a: number) => [number, number];
    readonly wasmemailextractionresult_bccEmails: (a: number) => [number, number];
    readonly wasmemailextractionresult_ccEmails: (a: number) => [number, number];
    readonly wasmemailextractionresult_content: (a: number) => [number, number];
    readonly wasmemailextractionresult_date: (a: number) => [number, number];
    readonly wasmemailextractionresult_default: () => number;
    readonly wasmemailextractionresult_fromEmail: (a: number) => [number, number];
    readonly wasmemailextractionresult_htmlContent: (a: number) => [number, number];
    readonly wasmemailextractionresult_messageId: (a: number) => [number, number];
    readonly wasmemailextractionresult_metadata: (a: number) => any;
    readonly wasmemailextractionresult_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: any, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number) => number;
    readonly wasmemailextractionresult_plainText: (a: number) => [number, number];
    readonly wasmemailextractionresult_set_attachments: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_bccEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_ccEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_date: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_fromEmail: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_htmlContent: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_messageId: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_metadata: (a: number, b: any) => void;
    readonly wasmemailextractionresult_set_plainText: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_subject: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_set_toEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailextractionresult_subject: (a: number) => [number, number];
    readonly wasmemailextractionresult_toEmails: (a: number) => [number, number];
    readonly wasmemailmetadata_attachments: (a: number) => [number, number];
    readonly wasmemailmetadata_bccEmails: (a: number) => [number, number];
    readonly wasmemailmetadata_ccEmails: (a: number) => [number, number];
    readonly wasmemailmetadata_default: () => number;
    readonly wasmemailmetadata_fromEmail: (a: number) => [number, number];
    readonly wasmemailmetadata_fromName: (a: number) => [number, number];
    readonly wasmemailmetadata_messageId: (a: number) => [number, number];
    readonly wasmemailmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number) => number;
    readonly wasmemailmetadata_set_attachments: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_bccEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_ccEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_fromEmail: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_fromName: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_messageId: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_set_toEmails: (a: number, b: number, c: number) => void;
    readonly wasmemailmetadata_toEmails: (a: number) => [number, number];
    readonly wasmembeddingconfig_acceleration: (a: number) => number;
    readonly wasmembeddingconfig_batchSize: (a: number) => number;
    readonly wasmembeddingconfig_cacheDir: (a: number) => [number, number];
    readonly wasmembeddingconfig_default: () => number;
    readonly wasmembeddingconfig_maxEmbedDurationSecs: (a: number) => [number, bigint];
    readonly wasmembeddingconfig_maxSequenceLength: (a: number) => number;
    readonly wasmembeddingconfig_model: (a: number) => any;
    readonly wasmembeddingconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: bigint, j: number) => number;
    readonly wasmembeddingconfig_normalize: (a: number) => number;
    readonly wasmembeddingconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmembeddingconfig_set_batchSize: (a: number, b: number) => void;
    readonly wasmembeddingconfig_set_cacheDir: (a: number, b: number, c: number) => void;
    readonly wasmembeddingconfig_set_maxEmbedDurationSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmembeddingconfig_set_maxSequenceLength: (a: number, b: number) => void;
    readonly wasmembeddingconfig_set_model: (a: number, b: any) => void;
    readonly wasmembeddingconfig_set_normalize: (a: number, b: number) => void;
    readonly wasmembeddingconfig_set_showDownloadProgress: (a: number, b: number) => void;
    readonly wasmembeddingconfig_showDownloadProgress: (a: number) => number;
    readonly wasmembeddingmodeltype_default: () => number;
    readonly wasmembeddingmodeltype_dimensions: (a: number) => number;
    readonly wasmembeddingmodeltype_llm: (a: number) => number;
    readonly wasmembeddingmodeltype_modelId: (a: number) => [number, number];
    readonly wasmembeddingmodeltype_name: (a: number) => [number, number];
    readonly wasmembeddingmodeltype_set_dimensions: (a: number, b: number) => void;
    readonly wasmembeddingmodeltype_set_llm: (a: number, b: number) => void;
    readonly wasmembeddingmodeltype_set_modelId: (a: number, b: number, c: number) => void;
    readonly wasmembeddingmodeltype_set_name: (a: number, b: number, c: number) => void;
    readonly wasmembeddingmodeltype_set_type: (a: number, b: number, c: number) => void;
    readonly wasmembeddingmodeltype_type: (a: number) => [number, number];
    readonly wasmentity_category: (a: number) => [number, number];
    readonly wasmentity_confidence: (a: number) => number;
    readonly wasmentity_default: () => number;
    readonly wasmentity_end: (a: number) => number;
    readonly wasmentity_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmentity_set_category: (a: number, b: number) => void;
    readonly wasmentity_set_confidence: (a: number, b: number) => void;
    readonly wasmentity_set_end: (a: number, b: number) => void;
    readonly wasmentity_set_start: (a: number, b: number) => void;
    readonly wasmentity_set_text: (a: number, b: number, c: number) => void;
    readonly wasmentity_start: (a: number) => number;
    readonly wasmentity_text: (a: number) => [number, number];
    readonly wasmepubmetadata_coverImage: (a: number) => [number, number];
    readonly wasmepubmetadata_coverage: (a: number) => [number, number];
    readonly wasmepubmetadata_dcFormat: (a: number) => [number, number];
    readonly wasmepubmetadata_dcType: (a: number) => [number, number];
    readonly wasmepubmetadata_default: () => number;
    readonly wasmepubmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number) => number;
    readonly wasmepubmetadata_relation: (a: number) => [number, number];
    readonly wasmepubmetadata_set_coverImage: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_set_coverage: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_set_dcFormat: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_set_dcType: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_set_relation: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_set_source: (a: number, b: number, c: number) => void;
    readonly wasmepubmetadata_source: (a: number) => [number, number];
    readonly wasmexcelmetadata_default: () => number;
    readonly wasmexcelmetadata_new: (a: number, b: number, c: number) => number;
    readonly wasmexcelmetadata_set_sheetCount: (a: number, b: number) => void;
    readonly wasmexcelmetadata_set_sheetNames: (a: number, b: number, c: number) => void;
    readonly wasmexcelmetadata_sheetCount: (a: number) => number;
    readonly wasmexcelmetadata_sheetNames: (a: number) => [number, number];
    readonly wasmexcelsheet_cellCount: (a: number) => number;
    readonly wasmexcelsheet_colCount: (a: number) => number;
    readonly wasmexcelsheet_default: () => number;
    readonly wasmexcelsheet_markdown: (a: number) => [number, number];
    readonly wasmexcelsheet_name: (a: number) => [number, number];
    readonly wasmexcelsheet_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmexcelsheet_rowCount: (a: number) => number;
    readonly wasmexcelsheet_set_cellCount: (a: number, b: number) => void;
    readonly wasmexcelsheet_set_colCount: (a: number, b: number) => void;
    readonly wasmexcelsheet_set_markdown: (a: number, b: number, c: number) => void;
    readonly wasmexcelsheet_set_name: (a: number, b: number, c: number) => void;
    readonly wasmexcelsheet_set_rowCount: (a: number, b: number) => void;
    readonly wasmexcelsheet_set_tableCells: (a: number, b: number) => void;
    readonly wasmexcelsheet_tableCells: (a: number) => any;
    readonly wasmexcelworkbook_default: () => number;
    readonly wasmexcelworkbook_metadata: (a: number) => any;
    readonly wasmexcelworkbook_new: (a: number, b: number, c: any, d: number, e: number) => number;
    readonly wasmexcelworkbook_revisions: (a: number) => any;
    readonly wasmexcelworkbook_set_metadata: (a: number, b: any) => void;
    readonly wasmexcelworkbook_set_revisions: (a: number, b: number, c: number) => void;
    readonly wasmexcelworkbook_set_sheets: (a: number, b: number, c: number) => void;
    readonly wasmexcelworkbook_sheets: (a: number) => [number, number];
    readonly wasmextracteddocument_annotations: (a: number) => any;
    readonly wasmextracteddocument_children: (a: number) => any;
    readonly wasmextracteddocument_chunks: (a: number) => any;
    readonly wasmextracteddocument_content: (a: number) => [number, number];
    readonly wasmextracteddocument_counts: (a: number) => number;
    readonly wasmextracteddocument_default: () => number;
    readonly wasmextracteddocument_detectedLanguages: (a: number) => [number, number];
    readonly wasmextracteddocument_djotContent: (a: number) => number;
    readonly wasmextracteddocument_document: (a: number) => number;
    readonly wasmextracteddocument_elements: (a: number) => any;
    readonly wasmextracteddocument_entities: (a: number) => any;
    readonly wasmextracteddocument_extractionMethod: (a: number) => [number, number];
    readonly wasmextracteddocument_formFields: (a: number) => [number, number];
    readonly wasmextracteddocument_formattedContent: (a: number) => [number, number];
    readonly wasmextracteddocument_formulas: (a: number) => [number, number];
    readonly wasmextracteddocument_images: (a: number) => any;
    readonly wasmextracteddocument_llmUsage: (a: number) => any;
    readonly wasmextracteddocument_metadata: (a: number) => number;
    readonly wasmextracteddocument_mimeType: (a: number) => [number, number];
    readonly wasmextracteddocument_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number, c1: number, d1: number, e1: number, f1: number, g1: number, h1: number, i1: number, j1: number, k1: number, l1: number, m1: number, n1: number, o1: number, p1: number, q1: number, r1: number, s1: number, t1: number, u1: number, v1: number, w1: number, x1: number, y1: number) => number;
    readonly wasmextracteddocument_ocrElements: (a: number) => any;
    readonly wasmextracteddocument_pageClassifications: (a: number) => any;
    readonly wasmextracteddocument_pages: (a: number) => any;
    readonly wasmextracteddocument_processingWarnings: (a: number) => [number, number];
    readonly wasmextracteddocument_qualityScore: (a: number) => [number, number];
    readonly wasmextracteddocument_redactionReport: (a: number) => number;
    readonly wasmextracteddocument_revisions: (a: number) => any;
    readonly wasmextracteddocument_set_annotations: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_children: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_chunks: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_content: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_counts: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_detectedLanguages: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_djotContent: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_document: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_elements: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_entities: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_extractionMethod: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_formFields: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_formattedContent: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_formulas: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_images: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_llmUsage: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_metadata: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_mimeType: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_ocrElements: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_pageClassifications: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_pages: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_processingWarnings: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_qualityScore: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_redactionReport: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_revisions: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_structuredOutput: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_summary: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_tables: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_set_translation: (a: number, b: number) => void;
    readonly wasmextracteddocument_set_uris: (a: number, b: number, c: number) => void;
    readonly wasmextracteddocument_structuredOutput: (a: number) => any;
    readonly wasmextracteddocument_summary: (a: number) => number;
    readonly wasmextracteddocument_tables: (a: number) => [number, number];
    readonly wasmextracteddocument_translation: (a: number) => number;
    readonly wasmextracteddocument_uris: (a: number) => any;
    readonly wasmextractedimage_bitsPerComponent: (a: number) => number;
    readonly wasmextractedimage_boundingBox: (a: number) => number;
    readonly wasmextractedimage_caption: (a: number) => [number, number];
    readonly wasmextractedimage_clusterId: (a: number) => number;
    readonly wasmextractedimage_colorspace: (a: number) => [number, number];
    readonly wasmextractedimage_data: (a: number) => [number, number];
    readonly wasmextractedimage_dataBase64: (a: number) => [number, number];
    readonly wasmextractedimage_default: () => number;
    readonly wasmextractedimage_description: (a: number) => [number, number];
    readonly wasmextractedimage_format: (a: number) => [number, number];
    readonly wasmextractedimage_height: (a: number) => number;
    readonly wasmextractedimage_imageIndex: (a: number) => number;
    readonly wasmextractedimage_imageKind: (a: number) => [number, number];
    readonly wasmextractedimage_isMask: (a: number) => number;
    readonly wasmextractedimage_kindConfidence: (a: number) => number;
    readonly wasmextractedimage_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number) => number;
    readonly wasmextractedimage_ocrResult: (a: number) => number;
    readonly wasmextractedimage_pageNumber: (a: number) => number;
    readonly wasmextractedimage_qrCodes: (a: number) => any;
    readonly wasmextractedimage_set_bitsPerComponent: (a: number, b: number) => void;
    readonly wasmextractedimage_set_boundingBox: (a: number, b: number) => void;
    readonly wasmextractedimage_set_caption: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_clusterId: (a: number, b: number) => void;
    readonly wasmextractedimage_set_colorspace: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_data: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_dataBase64: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_description: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_format: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_height: (a: number, b: number) => void;
    readonly wasmextractedimage_set_imageIndex: (a: number, b: number) => void;
    readonly wasmextractedimage_set_imageKind: (a: number, b: number) => void;
    readonly wasmextractedimage_set_isMask: (a: number, b: number) => void;
    readonly wasmextractedimage_set_kindConfidence: (a: number, b: number) => void;
    readonly wasmextractedimage_set_ocrResult: (a: number, b: number) => void;
    readonly wasmextractedimage_set_pageNumber: (a: number, b: number) => void;
    readonly wasmextractedimage_set_qrCodes: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_sourcePath: (a: number, b: number, c: number) => void;
    readonly wasmextractedimage_set_width: (a: number, b: number) => void;
    readonly wasmextractedimage_sourcePath: (a: number) => [number, number];
    readonly wasmextractedimage_width: (a: number) => number;
    readonly wasmextracteduri_default: () => number;
    readonly wasmextracteduri_kind: (a: number) => [number, number];
    readonly wasmextracteduri_label: (a: number) => [number, number];
    readonly wasmextracteduri_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmextracteduri_page: (a: number) => number;
    readonly wasmextracteduri_set_kind: (a: number, b: number) => void;
    readonly wasmextracteduri_set_label: (a: number, b: number, c: number) => void;
    readonly wasmextracteduri_set_page: (a: number, b: number) => void;
    readonly wasmextracteduri_set_url: (a: number, b: number, c: number) => void;
    readonly wasmextracteduri_url: (a: number) => [number, number];
    readonly wasmextractinput_bytes: (a: number) => [number, number];
    readonly wasmextractinput_config: (a: number) => number;
    readonly wasmextractinput_default: () => number;
    readonly wasmextractinput_filename: (a: number) => [number, number];
    readonly wasmextractinput_fromBytes: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmextractinput_fromUri: (a: number, b: number) => number;
    readonly wasmextractinput_kind: (a: number) => [number, number];
    readonly wasmextractinput_mimeType: (a: number) => [number, number];
    readonly wasmextractinput_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmextractinput_set_bytes: (a: number, b: number, c: number) => void;
    readonly wasmextractinput_set_config: (a: number, b: number) => void;
    readonly wasmextractinput_set_filename: (a: number, b: number, c: number) => void;
    readonly wasmextractinput_set_kind: (a: number, b: number) => void;
    readonly wasmextractinput_set_mimeType: (a: number, b: number, c: number) => void;
    readonly wasmextractinput_set_uri: (a: number, b: number, c: number) => void;
    readonly wasmextractinput_uri: (a: number) => [number, number];
    readonly wasmextractionconfig_acceleration: (a: number) => number;
    readonly wasmextractionconfig_cacheNamespace: (a: number) => [number, number];
    readonly wasmextractionconfig_cacheTtlSecs: (a: number) => [number, bigint];
    readonly wasmextractionconfig_captioning: (a: number) => number;
    readonly wasmextractionconfig_chunkClassification: (a: number) => number;
    readonly wasmextractionconfig_chunking: (a: number) => number;
    readonly wasmextractionconfig_contentFilter: (a: number) => number;
    readonly wasmextractionconfig_default: () => number;
    readonly wasmextractionconfig_disableOcr: (a: number) => number;
    readonly wasmextractionconfig_email: (a: number) => number;
    readonly wasmextractionconfig_enableQualityProcessing: (a: number) => number;
    readonly wasmextractionconfig_escapeMarkdown: (a: number) => number;
    readonly wasmextractionconfig_extractionTimeoutSecs: (a: number) => [number, bigint];
    readonly wasmextractionconfig_forceOcr: (a: number) => number;
    readonly wasmextractionconfig_forceOcrPages: (a: number) => [number, number];
    readonly wasmextractionconfig_images: (a: number) => number;
    readonly wasmextractionconfig_includeDocumentStructure: (a: number) => number;
    readonly wasmextractionconfig_jupyterCellRendering: (a: number) => [number, number];
    readonly wasmextractionconfig_languageDetection: (a: number) => number;
    readonly wasmextractionconfig_maxArchiveDepth: (a: number) => number;
    readonly wasmextractionconfig_maxConcurrentExtractions: (a: number) => number;
    readonly wasmextractionconfig_maxEmbeddedFileBytes: (a: number) => [number, bigint];
    readonly wasmextractionconfig_needsImageData: (a: number) => number;
    readonly wasmextractionconfig_needsImageProcessing: (a: number) => number;
    readonly wasmextractionconfig_ner: (a: number) => number;
    readonly wasmextractionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: bigint, a1: number, b1: number, c1: number, d1: bigint, e1: number, f1: number, g1: number, h1: number, i1: bigint, j1: number, k1: number, l1: number, m1: number, n1: number, o1: number, p1: number, q1: number, r1: number, s1: number) => number;
    readonly wasmextractionconfig_ocr: (a: number) => number;
    readonly wasmextractionconfig_ocrStrategy: (a: number) => any;
    readonly wasmextractionconfig_outputFormat: (a: number) => [number, number];
    readonly wasmextractionconfig_pageClassification: (a: number) => number;
    readonly wasmextractionconfig_pages: (a: number) => number;
    readonly wasmextractionconfig_postprocessor: (a: number) => number;
    readonly wasmextractionconfig_qrCodes: (a: number) => number;
    readonly wasmextractionconfig_redaction: (a: number) => number;
    readonly wasmextractionconfig_resultFormat: (a: number) => [number, number];
    readonly wasmextractionconfig_securityLimits: (a: number) => number;
    readonly wasmextractionconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_cacheNamespace: (a: number, b: number, c: number) => void;
    readonly wasmextractionconfig_set_cacheTtlSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmextractionconfig_set_captioning: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_chunkClassification: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_chunking: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_contentFilter: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_disableOcr: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_email: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_enableQualityProcessing: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_escapeMarkdown: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_extractionTimeoutSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmextractionconfig_set_forceOcr: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_forceOcrPages: (a: number, b: number, c: number) => void;
    readonly wasmextractionconfig_set_images: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_includeDocumentStructure: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_jupyterCellRendering: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_languageDetection: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_maxArchiveDepth: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_maxConcurrentExtractions: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_maxEmbeddedFileBytes: (a: number, b: number, c: bigint) => void;
    readonly wasmextractionconfig_set_ner: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_ocr: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_ocrStrategy: (a: number, b: any) => void;
    readonly wasmextractionconfig_set_outputFormat: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_pageClassification: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_pages: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_postprocessor: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_qrCodes: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_redaction: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_resultFormat: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_securityLimits: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_structuredExtraction: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_summarization: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_tableAnchors: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_tokenReduction: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_translation: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_url: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_useCache: (a: number, b: number) => void;
    readonly wasmextractionconfig_set_useLayoutForMarkdown: (a: number, b: number) => void;
    readonly wasmextractionconfig_structuredExtraction: (a: number) => number;
    readonly wasmextractionconfig_summarization: (a: number) => number;
    readonly wasmextractionconfig_tableAnchors: (a: number) => number;
    readonly wasmextractionconfig_tokenReduction: (a: number) => number;
    readonly wasmextractionconfig_translation: (a: number) => number;
    readonly wasmextractionconfig_url: (a: number) => number;
    readonly wasmextractionconfig_useCache: (a: number) => number;
    readonly wasmextractionconfig_useLayoutForMarkdown: (a: number) => number;
    readonly wasmextractionerroritem_code: (a: number) => number;
    readonly wasmextractionerroritem_default: () => number;
    readonly wasmextractionerroritem_errorType: (a: number) => [number, number];
    readonly wasmextractionerroritem_index: (a: number) => number;
    readonly wasmextractionerroritem_message: (a: number) => [number, number];
    readonly wasmextractionerroritem_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmextractionerroritem_set_code: (a: number, b: number) => void;
    readonly wasmextractionerroritem_set_errorType: (a: number, b: number, c: number) => void;
    readonly wasmextractionerroritem_set_index: (a: number, b: number) => void;
    readonly wasmextractionerroritem_set_message: (a: number, b: number, c: number) => void;
    readonly wasmextractionerroritem_set_source: (a: number, b: number, c: number) => void;
    readonly wasmextractionerroritem_source: (a: number) => [number, number];
    readonly wasmextractionresult_crawlFinalUrls: (a: number) => [number, number];
    readonly wasmextractionresult_crawlRedirectCount: (a: number) => number;
    readonly wasmextractionresult_crawlUniqueNormalizedUrls: (a: number) => [number, number];
    readonly wasmextractionresult_default: () => number;
    readonly wasmextractionresult_errors: (a: number) => [number, number];
    readonly wasmextractionresult_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmextractionresult_results: (a: number) => [number, number];
    readonly wasmextractionresult_set_crawlFinalUrls: (a: number, b: number, c: number) => void;
    readonly wasmextractionresult_set_crawlRedirectCount: (a: number, b: number) => void;
    readonly wasmextractionresult_set_crawlUniqueNormalizedUrls: (a: number, b: number, c: number) => void;
    readonly wasmextractionresult_set_errors: (a: number, b: number, c: number) => void;
    readonly wasmextractionresult_set_results: (a: number, b: number, c: number) => void;
    readonly wasmextractionresult_set_summary: (a: number, b: number) => void;
    readonly wasmextractionresult_single: (a: number) => number;
    readonly wasmextractionresult_summary: (a: number) => number;
    readonly wasmextractionsummary_default: () => number;
    readonly wasmextractionsummary_documentsDownloaded: (a: number) => number;
    readonly wasmextractionsummary_errors: (a: number) => number;
    readonly wasmextractionsummary_inputs: (a: number) => number;
    readonly wasmextractionsummary_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmextractionsummary_pagesCrawled: (a: number) => number;
    readonly wasmextractionsummary_remoteUrls: (a: number) => number;
    readonly wasmextractionsummary_results: (a: number) => number;
    readonly wasmextractionsummary_set_documentsDownloaded: (a: number, b: number) => void;
    readonly wasmextractionsummary_set_errors: (a: number, b: number) => void;
    readonly wasmextractionsummary_set_inputs: (a: number, b: number) => void;
    readonly wasmextractionsummary_set_pagesCrawled: (a: number, b: number) => void;
    readonly wasmextractionsummary_set_remoteUrls: (a: number, b: number) => void;
    readonly wasmextractionsummary_set_results: (a: number, b: number) => void;
    readonly wasmfictionbookmetadata_annotation: (a: number) => [number, number];
    readonly wasmfictionbookmetadata_default: () => number;
    readonly wasmfictionbookmetadata_genres: (a: number) => [number, number];
    readonly wasmfictionbookmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmfictionbookmetadata_sequences: (a: number) => [number, number];
    readonly wasmfictionbookmetadata_set_annotation: (a: number, b: number, c: number) => void;
    readonly wasmfictionbookmetadata_set_genres: (a: number, b: number, c: number) => void;
    readonly wasmfictionbookmetadata_set_sequences: (a: number, b: number, c: number) => void;
    readonly wasmfileextractionconfig_captioning: (a: number) => number;
    readonly wasmfileextractionconfig_chunkClassification: (a: number) => number;
    readonly wasmfileextractionconfig_chunking: (a: number) => number;
    readonly wasmfileextractionconfig_contentFilter: (a: number) => number;
    readonly wasmfileextractionconfig_default: () => number;
    readonly wasmfileextractionconfig_disableOcr: (a: number) => number;
    readonly wasmfileextractionconfig_enableQualityProcessing: (a: number) => number;
    readonly wasmfileextractionconfig_forceOcr: (a: number) => number;
    readonly wasmfileextractionconfig_forceOcrPages: (a: number) => [number, number];
    readonly wasmfileextractionconfig_images: (a: number) => number;
    readonly wasmfileextractionconfig_includeDocumentStructure: (a: number) => number;
    readonly wasmfileextractionconfig_languageDetection: (a: number) => number;
    readonly wasmfileextractionconfig_ner: (a: number) => number;
    readonly wasmfileextractionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: bigint, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number, c1: number) => number;
    readonly wasmfileextractionconfig_ocr: (a: number) => number;
    readonly wasmfileextractionconfig_ocrStrategy: (a: number) => any;
    readonly wasmfileextractionconfig_outputFormat: (a: number) => [number, number];
    readonly wasmfileextractionconfig_pageClassification: (a: number) => number;
    readonly wasmfileextractionconfig_pages: (a: number) => number;
    readonly wasmfileextractionconfig_postprocessor: (a: number) => number;
    readonly wasmfileextractionconfig_qrCodes: (a: number) => number;
    readonly wasmfileextractionconfig_redaction: (a: number) => number;
    readonly wasmfileextractionconfig_resultFormat: (a: number) => [number, number];
    readonly wasmfileextractionconfig_set_captioning: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_chunkClassification: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_chunking: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_contentFilter: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_disableOcr: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_enableQualityProcessing: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_forceOcr: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_forceOcrPages: (a: number, b: number, c: number) => void;
    readonly wasmfileextractionconfig_set_images: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_includeDocumentStructure: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_languageDetection: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_ner: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_ocr: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_ocrStrategy: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_outputFormat: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_pageClassification: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_pages: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_postprocessor: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_qrCodes: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_redaction: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_resultFormat: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_structuredExtraction: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_summarization: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_timeoutSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmfileextractionconfig_set_tokenReduction: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_translation: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_set_url: (a: number, b: number) => void;
    readonly wasmfileextractionconfig_structuredExtraction: (a: number) => number;
    readonly wasmfileextractionconfig_summarization: (a: number) => number;
    readonly wasmfileextractionconfig_timeoutSecs: (a: number) => [number, bigint];
    readonly wasmfileextractionconfig_tokenReduction: (a: number) => number;
    readonly wasmfileextractionconfig_translation: (a: number) => number;
    readonly wasmfileextractionconfig_url: (a: number) => number;
    readonly wasmfootnote_content: (a: number) => [number, number];
    readonly wasmfootnote_default: () => number;
    readonly wasmfootnote_label: (a: number) => [number, number];
    readonly wasmfootnote_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmfootnote_set_content: (a: number, b: number, c: number) => void;
    readonly wasmfootnote_set_label: (a: number, b: number, c: number) => void;
    readonly wasmformatmetadata_0: (a: number) => any;
    readonly wasmformatmetadata_default: () => number;
    readonly wasmformatmetadata_formatType: (a: number) => [number, number];
    readonly wasmformatmetadata_set_0: (a: number, b: number) => void;
    readonly wasmformatmetadata_set_formatType: (a: number, b: number, c: number) => void;
    readonly wasmformattedblock_blockType: (a: number) => [number, number];
    readonly wasmformattedblock_children: (a: number) => [number, number];
    readonly wasmformattedblock_code: (a: number) => [number, number];
    readonly wasmformattedblock_default: () => number;
    readonly wasmformattedblock_inlineContent: (a: number) => [number, number];
    readonly wasmformattedblock_language: (a: number) => [number, number];
    readonly wasmformattedblock_level: (a: number) => number;
    readonly wasmformattedblock_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmformattedblock_set_blockType: (a: number, b: number) => void;
    readonly wasmformattedblock_set_children: (a: number, b: number, c: number) => void;
    readonly wasmformattedblock_set_code: (a: number, b: number, c: number) => void;
    readonly wasmformattedblock_set_inlineContent: (a: number, b: number, c: number) => void;
    readonly wasmformattedblock_set_language: (a: number, b: number, c: number) => void;
    readonly wasmformattedblock_set_level: (a: number, b: number) => void;
    readonly wasmformula_bbox: (a: number) => number;
    readonly wasmformula_default: () => number;
    readonly wasmformula_latex: (a: number) => [number, number];
    readonly wasmformula_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmformula_page: (a: number) => number;
    readonly wasmformula_set_bbox: (a: number, b: number) => void;
    readonly wasmformula_set_latex: (a: number, b: number, c: number) => void;
    readonly wasmformula_set_page: (a: number, b: number) => void;
    readonly wasmgridcell_bbox: (a: number) => number;
    readonly wasmgridcell_col: (a: number) => number;
    readonly wasmgridcell_colSpan: (a: number) => number;
    readonly wasmgridcell_content: (a: number) => [number, number];
    readonly wasmgridcell_default: () => number;
    readonly wasmgridcell_isHeader: (a: number) => number;
    readonly wasmgridcell_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmgridcell_row: (a: number) => number;
    readonly wasmgridcell_rowSpan: (a: number) => number;
    readonly wasmgridcell_set_bbox: (a: number, b: number) => void;
    readonly wasmgridcell_set_col: (a: number, b: number) => void;
    readonly wasmgridcell_set_colSpan: (a: number, b: number) => void;
    readonly wasmgridcell_set_content: (a: number, b: number, c: number) => void;
    readonly wasmgridcell_set_isHeader: (a: number, b: number) => void;
    readonly wasmgridcell_set_row: (a: number, b: number) => void;
    readonly wasmgridcell_set_rowSpan: (a: number, b: number) => void;
    readonly wasmheadermetadata_default: () => number;
    readonly wasmheadermetadata_depth: (a: number) => number;
    readonly wasmheadermetadata_htmlOffset: (a: number) => number;
    readonly wasmheadermetadata_id: (a: number) => [number, number];
    readonly wasmheadermetadata_level: (a: number) => number;
    readonly wasmheadermetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmheadermetadata_set_depth: (a: number, b: number) => void;
    readonly wasmheadermetadata_set_htmlOffset: (a: number, b: number) => void;
    readonly wasmheadermetadata_set_id: (a: number, b: number, c: number) => void;
    readonly wasmheadermetadata_set_level: (a: number, b: number) => void;
    readonly wasmheadermetadata_set_text: (a: number, b: number, c: number) => void;
    readonly wasmheadermetadata_text: (a: number) => [number, number];
    readonly wasmheadingcontext_default: () => number;
    readonly wasmheadingcontext_headings: (a: number) => [number, number];
    readonly wasmheadingcontext_new: (a: number, b: number) => number;
    readonly wasmheadingcontext_set_headings: (a: number, b: number, c: number) => void;
    readonly wasmheadinglevel_default: () => number;
    readonly wasmheadinglevel_level: (a: number) => number;
    readonly wasmheadinglevel_new: (a: number, b: number, c: number) => number;
    readonly wasmheadinglevel_set_level: (a: number, b: number) => void;
    readonly wasmheadinglevel_set_text: (a: number, b: number, c: number) => void;
    readonly wasmheadinglevel_text: (a: number) => [number, number];
    readonly wasmhierarchicalblock_default: () => number;
    readonly wasmhierarchicalblock_fontSize: (a: number) => number;
    readonly wasmhierarchicalblock_level: (a: number) => [number, number];
    readonly wasmhierarchicalblock_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmhierarchicalblock_set_fontSize: (a: number, b: number) => void;
    readonly wasmhierarchicalblock_set_level: (a: number, b: number, c: number) => void;
    readonly wasmhierarchicalblock_set_text: (a: number, b: number, c: number) => void;
    readonly wasmhierarchicalblock_text: (a: number) => [number, number];
    readonly wasmhtmlmetadata_author: (a: number) => [number, number];
    readonly wasmhtmlmetadata_baseHref: (a: number) => [number, number];
    readonly wasmhtmlmetadata_canonicalUrl: (a: number) => [number, number];
    readonly wasmhtmlmetadata_default: () => number;
    readonly wasmhtmlmetadata_description: (a: number) => [number, number];
    readonly wasmhtmlmetadata_headers: (a: number) => [number, number];
    readonly wasmhtmlmetadata_images: (a: number) => [number, number];
    readonly wasmhtmlmetadata_keywords: (a: number) => [number, number];
    readonly wasmhtmlmetadata_language: (a: number) => [number, number];
    readonly wasmhtmlmetadata_links: (a: number) => [number, number];
    readonly wasmhtmlmetadata_metaTags: (a: number) => any;
    readonly wasmhtmlmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number) => number;
    readonly wasmhtmlmetadata_openGraph: (a: number) => any;
    readonly wasmhtmlmetadata_set_author: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_baseHref: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_canonicalUrl: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_description: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_headers: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_images: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_keywords: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_language: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_links: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_metaTags: (a: number, b: any) => void;
    readonly wasmhtmlmetadata_set_openGraph: (a: number, b: any) => void;
    readonly wasmhtmlmetadata_set_structuredData: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_textDirection: (a: number, b: number) => void;
    readonly wasmhtmlmetadata_set_title: (a: number, b: number, c: number) => void;
    readonly wasmhtmlmetadata_set_twitterCard: (a: number, b: any) => void;
    readonly wasmhtmlmetadata_structuredData: (a: number) => [number, number];
    readonly wasmhtmlmetadata_textDirection: (a: number) => [number, number];
    readonly wasmhtmlmetadata_title: (a: number) => [number, number];
    readonly wasmhtmlmetadata_twitterCard: (a: number) => any;
    readonly wasmimageextractionconfig_appendOcrText: (a: number) => number;
    readonly wasmimageextractionconfig_autoAdjustDpi: (a: number) => number;
    readonly wasmimageextractionconfig_classify: (a: number) => number;
    readonly wasmimageextractionconfig_default: () => number;
    readonly wasmimageextractionconfig_extractImages: (a: number) => number;
    readonly wasmimageextractionconfig_includeDataBase64: (a: number) => number;
    readonly wasmimageextractionconfig_includePageRasters: (a: number) => number;
    readonly wasmimageextractionconfig_injectPlaceholders: (a: number) => number;
    readonly wasmimageextractionconfig_maxDpi: (a: number) => number;
    readonly wasmimageextractionconfig_maxImageDimension: (a: number) => number;
    readonly wasmimageextractionconfig_maxImagesPerPage: (a: number) => number;
    readonly wasmimageextractionconfig_minDpi: (a: number) => number;
    readonly wasmimageextractionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number) => number;
    readonly wasmimageextractionconfig_ocrTextOnly: (a: number) => number;
    readonly wasmimageextractionconfig_outputFormat: (a: number) => any;
    readonly wasmimageextractionconfig_runOcrOnImages: (a: number) => number;
    readonly wasmimageextractionconfig_set_appendOcrText: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_autoAdjustDpi: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_classify: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_extractImages: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_includeDataBase64: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_includePageRasters: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_injectPlaceholders: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_maxDpi: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_maxImageDimension: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_maxImagesPerPage: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_minDpi: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_ocrTextOnly: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_outputFormat: (a: number, b: any) => void;
    readonly wasmimageextractionconfig_set_runOcrOnImages: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_set_targetDpi: (a: number, b: number) => void;
    readonly wasmimageextractionconfig_targetDpi: (a: number) => number;
    readonly wasmimagemetadata_default: () => number;
    readonly wasmimagemetadata_exif: (a: number) => any;
    readonly wasmimagemetadata_format: (a: number) => [number, number];
    readonly wasmimagemetadata_height: (a: number) => number;
    readonly wasmimagemetadata_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmimagemetadata_set_exif: (a: number, b: any) => void;
    readonly wasmimagemetadata_set_format: (a: number, b: number, c: number) => void;
    readonly wasmimagemetadata_set_height: (a: number, b: number) => void;
    readonly wasmimagemetadata_set_width: (a: number, b: number) => void;
    readonly wasmimagemetadata_width: (a: number) => number;
    readonly wasmimagemetadatatype_alt: (a: number) => [number, number];
    readonly wasmimagemetadatatype_default: () => number;
    readonly wasmimagemetadatatype_imageType: (a: number) => [number, number];
    readonly wasmimagemetadatatype_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmimagemetadatatype_set_alt: (a: number, b: number, c: number) => void;
    readonly wasmimagemetadatatype_set_imageType: (a: number, b: number) => void;
    readonly wasmimagemetadatatype_set_src: (a: number, b: number, c: number) => void;
    readonly wasmimagemetadatatype_set_title: (a: number, b: number, c: number) => void;
    readonly wasmimagemetadatatype_src: (a: number) => [number, number];
    readonly wasmimagemetadatatype_title: (a: number) => [number, number];
    readonly wasmimageoutputformat_default: () => number;
    readonly wasmimageoutputformat_quality: (a: number) => number;
    readonly wasmimageoutputformat_set_quality: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_autoRotate: (a: number) => number;
    readonly wasmimagepreprocessingconfig_binarizationMethod: (a: number) => [number, number];
    readonly wasmimagepreprocessingconfig_contrastEnhance: (a: number) => number;
    readonly wasmimagepreprocessingconfig_default: () => number;
    readonly wasmimagepreprocessingconfig_denoise: (a: number) => number;
    readonly wasmimagepreprocessingconfig_deskew: (a: number) => number;
    readonly wasmimagepreprocessingconfig_invertColors: (a: number) => number;
    readonly wasmimagepreprocessingconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmimagepreprocessingconfig_set_autoRotate: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_set_binarizationMethod: (a: number, b: number, c: number) => void;
    readonly wasmimagepreprocessingconfig_set_contrastEnhance: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_set_denoise: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_set_deskew: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_set_invertColors: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_set_targetDpi: (a: number, b: number) => void;
    readonly wasmimagepreprocessingconfig_targetDpi: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_autoAdjusted: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_calculatedDpi: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_default: () => number;
    readonly wasmimagepreprocessingmetadata_dimensionClamped: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_finalDpi: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => number;
    readonly wasmimagepreprocessingmetadata_resampleMethod: (a: number) => [number, number];
    readonly wasmimagepreprocessingmetadata_resizeError: (a: number) => [number, number];
    readonly wasmimagepreprocessingmetadata_scaleFactor: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_set_autoAdjusted: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_calculatedDpi: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_dimensionClamped: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_finalDpi: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_resampleMethod: (a: number, b: number, c: number) => void;
    readonly wasmimagepreprocessingmetadata_set_resizeError: (a: number, b: number, c: number) => void;
    readonly wasmimagepreprocessingmetadata_set_scaleFactor: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_skippedResize: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_set_targetDpi: (a: number, b: number) => void;
    readonly wasmimagepreprocessingmetadata_skippedResize: (a: number) => number;
    readonly wasmimagepreprocessingmetadata_targetDpi: (a: number) => number;
    readonly wasminlineelement_content: (a: number) => [number, number];
    readonly wasminlineelement_default: () => number;
    readonly wasminlineelement_elementType: (a: number) => [number, number];
    readonly wasminlineelement_metadata: (a: number) => any;
    readonly wasminlineelement_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasminlineelement_set_content: (a: number, b: number, c: number) => void;
    readonly wasminlineelement_set_elementType: (a: number, b: number) => void;
    readonly wasminlineelement_set_metadata: (a: number, b: number) => void;
    readonly wasmjatsmetadata_contributorRoles: (a: number) => [number, number];
    readonly wasmjatsmetadata_copyright: (a: number) => [number, number];
    readonly wasmjatsmetadata_default: () => number;
    readonly wasmjatsmetadata_historyDates: (a: number) => any;
    readonly wasmjatsmetadata_license: (a: number) => [number, number];
    readonly wasmjatsmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmjatsmetadata_set_contributorRoles: (a: number, b: number, c: number) => void;
    readonly wasmjatsmetadata_set_copyright: (a: number, b: number, c: number) => void;
    readonly wasmjatsmetadata_set_historyDates: (a: number, b: any) => void;
    readonly wasmjatsmetadata_set_license: (a: number, b: number, c: number) => void;
    readonly wasmlanguagedetectionconfig_default: () => number;
    readonly wasmlanguagedetectionconfig_detectMultiple: (a: number) => number;
    readonly wasmlanguagedetectionconfig_enabled: (a: number) => number;
    readonly wasmlanguagedetectionconfig_minConfidence: (a: number) => number;
    readonly wasmlanguagedetectionconfig_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmlanguagedetectionconfig_set_detectMultiple: (a: number, b: number) => void;
    readonly wasmlanguagedetectionconfig_set_enabled: (a: number, b: number) => void;
    readonly wasmlanguagedetectionconfig_set_minConfidence: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_acceleration: (a: number) => number;
    readonly wasmlateinteractionconfig_batchSize: (a: number) => number;
    readonly wasmlateinteractionconfig_cacheDir: (a: number) => [number, number];
    readonly wasmlateinteractionconfig_default: () => number;
    readonly wasmlateinteractionconfig_maxEmbedDurationSecs: (a: number) => [number, bigint];
    readonly wasmlateinteractionconfig_maxLength: (a: number) => number;
    readonly wasmlateinteractionconfig_model: (a: number) => any;
    readonly wasmlateinteractionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: bigint) => number;
    readonly wasmlateinteractionconfig_queryMaxLength: (a: number) => number;
    readonly wasmlateinteractionconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_set_batchSize: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_set_cacheDir: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionconfig_set_maxEmbedDurationSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmlateinteractionconfig_set_maxLength: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_set_model: (a: number, b: any) => void;
    readonly wasmlateinteractionconfig_set_queryMaxLength: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_set_showDownloadProgress: (a: number, b: number) => void;
    readonly wasmlateinteractionconfig_showDownloadProgress: (a: number) => number;
    readonly wasmlateinteractionmodeltype_additionalFiles: (a: number) => [number, number];
    readonly wasmlateinteractionmodeltype_default: () => number;
    readonly wasmlateinteractionmodeltype_maxLength: (a: number) => [number, bigint];
    readonly wasmlateinteractionmodeltype_modelFile: (a: number) => [number, number];
    readonly wasmlateinteractionmodeltype_modelId: (a: number) => [number, number];
    readonly wasmlateinteractionmodeltype_name: (a: number) => [number, number];
    readonly wasmlateinteractionmodeltype_set_additionalFiles: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionmodeltype_set_maxLength: (a: number, b: number, c: bigint) => void;
    readonly wasmlateinteractionmodeltype_set_modelFile: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionmodeltype_set_modelId: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionmodeltype_set_name: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionmodeltype_set_type: (a: number, b: number, c: number) => void;
    readonly wasmlateinteractionmodeltype_type: (a: number) => [number, number];
    readonly wasmlayoutregion_areaFraction: (a: number) => number;
    readonly wasmlayoutregion_boundingBox: (a: number) => number;
    readonly wasmlayoutregion_className: (a: number) => [number, number];
    readonly wasmlayoutregion_confidence: (a: number) => number;
    readonly wasmlayoutregion_default: () => number;
    readonly wasmlayoutregion_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmlayoutregion_set_areaFraction: (a: number, b: number) => void;
    readonly wasmlayoutregion_set_boundingBox: (a: number, b: number) => void;
    readonly wasmlayoutregion_set_className: (a: number, b: number, c: number) => void;
    readonly wasmlayoutregion_set_confidence: (a: number, b: number) => void;
    readonly wasmlinkmetadata_default: () => number;
    readonly wasmlinkmetadata_href: (a: number) => [number, number];
    readonly wasmlinkmetadata_linkType: (a: number) => [number, number];
    readonly wasmlinkmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmlinkmetadata_rel: (a: number) => [number, number];
    readonly wasmlinkmetadata_set_href: (a: number, b: number, c: number) => void;
    readonly wasmlinkmetadata_set_linkType: (a: number, b: number) => void;
    readonly wasmlinkmetadata_set_rel: (a: number, b: number, c: number) => void;
    readonly wasmlinkmetadata_set_text: (a: number, b: number, c: number) => void;
    readonly wasmlinkmetadata_set_title: (a: number, b: number, c: number) => void;
    readonly wasmlinkmetadata_text: (a: number) => [number, number];
    readonly wasmlinkmetadata_title: (a: number) => [number, number];
    readonly wasmllmconfig_apiKey: (a: number) => [number, number];
    readonly wasmllmconfig_baseUrl: (a: number) => [number, number];
    readonly wasmllmconfig_default: () => number;
    readonly wasmllmconfig_headers: (a: number) => any;
    readonly wasmllmconfig_loadEnv: (a: number) => number;
    readonly wasmllmconfig_maxRetries: (a: number) => number;
    readonly wasmllmconfig_maxTokens: (a: number) => [number, bigint];
    readonly wasmllmconfig_model: (a: number) => [number, number];
    readonly wasmllmconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: bigint, i: number, j: number, k: number, l: number, m: bigint, n: number, o: number) => number;
    readonly wasmllmconfig_set_apiKey: (a: number, b: number, c: number) => void;
    readonly wasmllmconfig_set_baseUrl: (a: number, b: number, c: number) => void;
    readonly wasmllmconfig_set_headers: (a: number, b: number) => void;
    readonly wasmllmconfig_set_loadEnv: (a: number, b: number) => void;
    readonly wasmllmconfig_set_maxRetries: (a: number, b: number) => void;
    readonly wasmllmconfig_set_maxTokens: (a: number, b: number, c: bigint) => void;
    readonly wasmllmconfig_set_model: (a: number, b: number, c: number) => void;
    readonly wasmllmconfig_set_temperature: (a: number, b: number, c: number) => void;
    readonly wasmllmconfig_set_timeoutSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmllmconfig_temperature: (a: number) => [number, number];
    readonly wasmllmconfig_timeoutSecs: (a: number) => [number, bigint];
    readonly wasmllmusage_default: () => number;
    readonly wasmllmusage_estimatedCost: (a: number) => [number, number];
    readonly wasmllmusage_finishReason: (a: number) => [number, number];
    readonly wasmllmusage_inputTokens: (a: number) => [number, bigint];
    readonly wasmllmusage_model: (a: number) => [number, number];
    readonly wasmllmusage_new: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: number, h: bigint, i: number, j: bigint, k: number, l: number, m: number, n: number) => number;
    readonly wasmllmusage_outputTokens: (a: number) => [number, bigint];
    readonly wasmllmusage_set_estimatedCost: (a: number, b: number, c: number) => void;
    readonly wasmllmusage_set_finishReason: (a: number, b: number, c: number) => void;
    readonly wasmllmusage_set_inputTokens: (a: number, b: number, c: bigint) => void;
    readonly wasmllmusage_set_model: (a: number, b: number, c: number) => void;
    readonly wasmllmusage_set_outputTokens: (a: number, b: number, c: bigint) => void;
    readonly wasmllmusage_set_source: (a: number, b: number, c: number) => void;
    readonly wasmllmusage_set_totalTokens: (a: number, b: number, c: bigint) => void;
    readonly wasmllmusage_source: (a: number) => [number, number];
    readonly wasmllmusage_totalTokens: (a: number) => [number, bigint];
    readonly wasmmapresult_default: () => number;
    readonly wasmmapresult_new: (a: number, b: number) => number;
    readonly wasmmapresult_set_urls: (a: number, b: number, c: number) => void;
    readonly wasmmapresult_urls: (a: number) => [number, number];
    readonly wasmmetadata_abstractText: (a: number) => [number, number];
    readonly wasmmetadata_additional: (a: number) => any;
    readonly wasmmetadata_authors: (a: number) => [number, number];
    readonly wasmmetadata_category: (a: number) => [number, number];
    readonly wasmmetadata_createdAt: (a: number) => [number, number];
    readonly wasmmetadata_createdBy: (a: number) => [number, number];
    readonly wasmmetadata_default: () => number;
    readonly wasmmetadata_documentVersion: (a: number) => [number, number];
    readonly wasmmetadata_error: (a: number) => number;
    readonly wasmmetadata_extractionDurationMs: (a: number) => [number, bigint];
    readonly wasmmetadata_format: (a: number) => any;
    readonly wasmmetadata_imagePreprocessing: (a: number) => number;
    readonly wasmmetadata_isEmpty: (a: number) => number;
    readonly wasmmetadata_jsonSchema: (a: number) => any;
    readonly wasmmetadata_keywords: (a: number) => [number, number];
    readonly wasmmetadata_language: (a: number) => [number, number];
    readonly wasmmetadata_modifiedAt: (a: number) => [number, number];
    readonly wasmmetadata_modifiedBy: (a: number) => [number, number];
    readonly wasmmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: bigint, b1: number, c1: number, d1: number, e1: number, f1: number, g1: number, h1: number, i1: number, j1: number, k1: number) => number;
    readonly wasmmetadata_ocrUsed: (a: number) => number;
    readonly wasmmetadata_outputFormat: (a: number) => [number, number];
    readonly wasmmetadata_pages: (a: number) => number;
    readonly wasmmetadata_set_abstractText: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_additional: (a: number, b: any) => void;
    readonly wasmmetadata_set_authors: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_category: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_createdAt: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_createdBy: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_documentVersion: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_error: (a: number, b: number) => void;
    readonly wasmmetadata_set_extractionDurationMs: (a: number, b: number, c: bigint) => void;
    readonly wasmmetadata_set_format: (a: number, b: number) => void;
    readonly wasmmetadata_set_imagePreprocessing: (a: number, b: number) => void;
    readonly wasmmetadata_set_jsonSchema: (a: number, b: number) => void;
    readonly wasmmetadata_set_keywords: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_language: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_modifiedAt: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_modifiedBy: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_ocrUsed: (a: number, b: number) => void;
    readonly wasmmetadata_set_outputFormat: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_pages: (a: number, b: number) => void;
    readonly wasmmetadata_set_subject: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_tags: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_set_title: (a: number, b: number, c: number) => void;
    readonly wasmmetadata_subject: (a: number) => [number, number];
    readonly wasmmetadata_tags: (a: number) => [number, number];
    readonly wasmmetadata_title: (a: number) => [number, number];
    readonly wasmnerconfig_backend: (a: number) => [number, number];
    readonly wasmnerconfig_categories: (a: number) => [number, number];
    readonly wasmnerconfig_customLabels: (a: number) => [number, number];
    readonly wasmnerconfig_default: () => number;
    readonly wasmnerconfig_llm: (a: number) => number;
    readonly wasmnerconfig_model: (a: number) => [number, number];
    readonly wasmnerconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmnerconfig_set_backend: (a: number, b: number) => void;
    readonly wasmnerconfig_set_categories: (a: number, b: number, c: number) => void;
    readonly wasmnerconfig_set_customLabels: (a: number, b: number, c: number) => void;
    readonly wasmnerconfig_set_llm: (a: number, b: number) => void;
    readonly wasmnerconfig_set_model: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_content: (a: number) => [number, number];
    readonly wasmnodecontent_default: () => number;
    readonly wasmnodecontent_definition: (a: number) => [number, number];
    readonly wasmnodecontent_description: (a: number) => [number, number];
    readonly wasmnodecontent_entries: (a: number) => any;
    readonly wasmnodecontent_format: (a: number) => [number, number];
    readonly wasmnodecontent_grid: (a: number) => number;
    readonly wasmnodecontent_headingLevel: (a: number) => number;
    readonly wasmnodecontent_headingText: (a: number) => [number, number];
    readonly wasmnodecontent_imageIndex: (a: number) => number;
    readonly wasmnodecontent_key: (a: number) => [number, number];
    readonly wasmnodecontent_kind: (a: number) => [number, number];
    readonly wasmnodecontent_label: (a: number) => [number, number];
    readonly wasmnodecontent_language: (a: number) => [number, number];
    readonly wasmnodecontent_level: (a: number) => number;
    readonly wasmnodecontent_nodeType: (a: number) => [number, number];
    readonly wasmnodecontent_number: (a: number) => number;
    readonly wasmnodecontent_ordered: (a: number) => number;
    readonly wasmnodecontent_set_content: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_definition: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_description: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_entries: (a: number, b: number) => void;
    readonly wasmnodecontent_set_format: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_grid: (a: number, b: number) => void;
    readonly wasmnodecontent_set_headingLevel: (a: number, b: number) => void;
    readonly wasmnodecontent_set_headingText: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_imageIndex: (a: number, b: number) => void;
    readonly wasmnodecontent_set_key: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_kind: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_label: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_language: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_level: (a: number, b: number) => void;
    readonly wasmnodecontent_set_nodeType: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_number: (a: number, b: number) => void;
    readonly wasmnodecontent_set_ordered: (a: number, b: number) => void;
    readonly wasmnodecontent_set_src: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_term: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_text: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_set_title: (a: number, b: number, c: number) => void;
    readonly wasmnodecontent_src: (a: number) => [number, number];
    readonly wasmnodecontent_term: (a: number) => [number, number];
    readonly wasmnodecontent_text: (a: number) => [number, number];
    readonly wasmnodecontent_title: (a: number) => [number, number];
    readonly wasmocrboundinggeometry_default: () => number;
    readonly wasmocrboundinggeometry_height: (a: number) => number;
    readonly wasmocrboundinggeometry_left: (a: number) => number;
    readonly wasmocrboundinggeometry_points: (a: number) => any;
    readonly wasmocrboundinggeometry_set_height: (a: number, b: number) => void;
    readonly wasmocrboundinggeometry_set_left: (a: number, b: number) => void;
    readonly wasmocrboundinggeometry_set_points: (a: number, b: number) => void;
    readonly wasmocrboundinggeometry_set_top: (a: number, b: number) => void;
    readonly wasmocrboundinggeometry_set_type: (a: number, b: number, c: number) => void;
    readonly wasmocrboundinggeometry_set_width: (a: number, b: number) => void;
    readonly wasmocrboundinggeometry_top: (a: number) => number;
    readonly wasmocrboundinggeometry_type: (a: number) => [number, number];
    readonly wasmocrboundinggeometry_width: (a: number) => number;
    readonly wasmocrconfidence_default: () => number;
    readonly wasmocrconfidence_detection: (a: number) => [number, number];
    readonly wasmocrconfidence_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmocrconfidence_recognition: (a: number) => number;
    readonly wasmocrconfidence_set_detection: (a: number, b: number, c: number) => void;
    readonly wasmocrconfidence_set_recognition: (a: number, b: number) => void;
    readonly wasmocrconfig_acceleration: (a: number) => number;
    readonly wasmocrconfig_autoRotate: (a: number) => number;
    readonly wasmocrconfig_backend: (a: number) => [number, number];
    readonly wasmocrconfig_backendOptions: (a: number) => any;
    readonly wasmocrconfig_default: () => number;
    readonly wasmocrconfig_elementConfig: (a: number) => number;
    readonly wasmocrconfig_enabled: (a: number) => number;
    readonly wasmocrconfig_language: (a: number) => [number, number];
    readonly wasmocrconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number) => number;
    readonly wasmocrconfig_outputFormat: (a: number) => [number, number];
    readonly wasmocrconfig_paddleOcrConfig: (a: number) => any;
    readonly wasmocrconfig_pipeline: (a: number) => number;
    readonly wasmocrconfig_qualityThresholds: (a: number) => number;
    readonly wasmocrconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmocrconfig_set_autoRotate: (a: number, b: number) => void;
    readonly wasmocrconfig_set_backend: (a: number, b: number, c: number) => void;
    readonly wasmocrconfig_set_backendOptions: (a: number, b: number) => void;
    readonly wasmocrconfig_set_elementConfig: (a: number, b: number) => void;
    readonly wasmocrconfig_set_enabled: (a: number, b: number) => void;
    readonly wasmocrconfig_set_language: (a: number, b: number, c: number) => void;
    readonly wasmocrconfig_set_outputFormat: (a: number, b: number) => void;
    readonly wasmocrconfig_set_paddleOcrConfig: (a: number, b: number) => void;
    readonly wasmocrconfig_set_pipeline: (a: number, b: number) => void;
    readonly wasmocrconfig_set_qualityThresholds: (a: number, b: number) => void;
    readonly wasmocrconfig_set_tessdataBytes: (a: number, b: number) => void;
    readonly wasmocrconfig_set_tessdataPath: (a: number, b: number, c: number) => void;
    readonly wasmocrconfig_set_tesseractConfig: (a: number, b: number) => void;
    readonly wasmocrconfig_set_vlmConfig: (a: number, b: number) => void;
    readonly wasmocrconfig_set_vlmFallback: (a: number, b: any) => void;
    readonly wasmocrconfig_set_vlmPrompt: (a: number, b: number, c: number) => void;
    readonly wasmocrconfig_tessdataBytes: (a: number) => any;
    readonly wasmocrconfig_tessdataPath: (a: number) => [number, number];
    readonly wasmocrconfig_tesseractConfig: (a: number) => number;
    readonly wasmocrconfig_vlmConfig: (a: number) => number;
    readonly wasmocrconfig_vlmFallback: (a: number) => any;
    readonly wasmocrconfig_vlmPrompt: (a: number) => [number, number];
    readonly wasmocrelement_backendMetadata: (a: number) => any;
    readonly wasmocrelement_confidence: (a: number) => number;
    readonly wasmocrelement_default: () => number;
    readonly wasmocrelement_geometry: (a: number) => any;
    readonly wasmocrelement_level: (a: number) => [number, number];
    readonly wasmocrelement_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => number;
    readonly wasmocrelement_pageNumber: (a: number) => number;
    readonly wasmocrelement_parentId: (a: number) => [number, number];
    readonly wasmocrelement_rotation: (a: number) => number;
    readonly wasmocrelement_set_backendMetadata: (a: number, b: any) => void;
    readonly wasmocrelement_set_confidence: (a: number, b: number) => void;
    readonly wasmocrelement_set_geometry: (a: number, b: any) => void;
    readonly wasmocrelement_set_level: (a: number, b: number) => void;
    readonly wasmocrelement_set_pageNumber: (a: number, b: number) => void;
    readonly wasmocrelement_set_parentId: (a: number, b: number, c: number) => void;
    readonly wasmocrelement_set_rotation: (a: number, b: number) => void;
    readonly wasmocrelement_set_text: (a: number, b: number, c: number) => void;
    readonly wasmocrelement_text: (a: number) => [number, number];
    readonly wasmocrelementconfig_buildHierarchy: (a: number) => number;
    readonly wasmocrelementconfig_default: () => number;
    readonly wasmocrelementconfig_minLevel: (a: number) => [number, number];
    readonly wasmocrelementconfig_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmocrelementconfig_set_buildHierarchy: (a: number, b: number) => void;
    readonly wasmocrelementconfig_set_minLevel: (a: number, b: number) => void;
    readonly wasmocrextractionresult_content: (a: number) => [number, number];
    readonly wasmocrextractionresult_default: () => number;
    readonly wasmocrextractionresult_metadata: (a: number) => any;
    readonly wasmocrextractionresult_mimeType: (a: number) => [number, number];
    readonly wasmocrextractionresult_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmocrextractionresult_ocrElements: (a: number) => any;
    readonly wasmocrextractionresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmocrextractionresult_set_metadata: (a: number, b: any) => void;
    readonly wasmocrextractionresult_set_mimeType: (a: number, b: number, c: number) => void;
    readonly wasmocrextractionresult_set_ocrElements: (a: number, b: number, c: number) => void;
    readonly wasmocrextractionresult_set_tables: (a: number, b: number, c: number) => void;
    readonly wasmocrextractionresult_tables: (a: number) => [number, number];
    readonly wasmocrmetadata_default: () => number;
    readonly wasmocrmetadata_language: (a: number) => [number, number];
    readonly wasmocrmetadata_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmocrmetadata_outputFormat: (a: number) => [number, number];
    readonly wasmocrmetadata_psm: (a: number) => number;
    readonly wasmocrmetadata_set_language: (a: number, b: number, c: number) => void;
    readonly wasmocrmetadata_set_outputFormat: (a: number, b: number, c: number) => void;
    readonly wasmocrmetadata_set_psm: (a: number, b: number) => void;
    readonly wasmocrmetadata_set_tableCols: (a: number, b: number) => void;
    readonly wasmocrmetadata_set_tableCount: (a: number, b: number) => void;
    readonly wasmocrmetadata_set_tableRows: (a: number, b: number) => void;
    readonly wasmocrmetadata_tableCols: (a: number) => number;
    readonly wasmocrmetadata_tableCount: (a: number) => number;
    readonly wasmocrmetadata_tableRows: (a: number) => number;
    readonly wasmocrpipelineconfig_default: () => number;
    readonly wasmocrpipelineconfig_new: (a: number, b: number, c: number) => number;
    readonly wasmocrpipelineconfig_qualityThresholds: (a: number) => number;
    readonly wasmocrpipelineconfig_set_qualityThresholds: (a: number, b: number) => void;
    readonly wasmocrpipelineconfig_set_stages: (a: number, b: number, c: number) => void;
    readonly wasmocrpipelineconfig_stages: (a: number) => [number, number];
    readonly wasmocrpipelinestage_backend: (a: number) => [number, number];
    readonly wasmocrpipelinestage_backendOptions: (a: number) => any;
    readonly wasmocrpipelinestage_default: () => number;
    readonly wasmocrpipelinestage_language: (a: number) => [number, number];
    readonly wasmocrpipelinestage_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmocrpipelinestage_paddleOcrConfig: (a: number) => any;
    readonly wasmocrpipelinestage_priority: (a: number) => number;
    readonly wasmocrpipelinestage_set_backend: (a: number, b: number, c: number) => void;
    readonly wasmocrpipelinestage_set_backendOptions: (a: number, b: number) => void;
    readonly wasmocrpipelinestage_set_language: (a: number, b: number, c: number) => void;
    readonly wasmocrpipelinestage_set_paddleOcrConfig: (a: number, b: number) => void;
    readonly wasmocrpipelinestage_set_priority: (a: number, b: number) => void;
    readonly wasmocrpipelinestage_set_tesseractConfig: (a: number, b: number) => void;
    readonly wasmocrpipelinestage_set_vlmConfig: (a: number, b: number) => void;
    readonly wasmocrpipelinestage_tesseractConfig: (a: number) => number;
    readonly wasmocrpipelinestage_vlmConfig: (a: number) => number;
    readonly wasmocrqualitythresholds_alnumWsRatioThreshold: (a: number) => number;
    readonly wasmocrqualitythresholds_criticalFragmentedWordRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_default: () => number;
    readonly wasmocrqualitythresholds_enableProvenanceOcrRouting: (a: number) => number;
    readonly wasmocrqualitythresholds_maxFragmentedWordRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_minAlnumRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_minAvgWordLength: (a: number) => number;
    readonly wasmocrqualitythresholds_minConsecutiveRepeatRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_minGarbageChars: (a: number) => number;
    readonly wasmocrqualitythresholds_minMeaningfulWordLen: (a: number) => number;
    readonly wasmocrqualitythresholds_minMeaningfulWords: (a: number) => number;
    readonly wasmocrqualitythresholds_minNonWhitespacePerPage: (a: number) => number;
    readonly wasmocrqualitythresholds_minProvenanceFallbackRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_minTotalNonWhitespace: (a: number) => number;
    readonly wasmocrqualitythresholds_minUndecodableRatio: (a: number) => number;
    readonly wasmocrqualitythresholds_minWordsForAvgLengthCheck: (a: number) => number;
    readonly wasmocrqualitythresholds_minWordsForRepeatCheck: (a: number) => number;
    readonly wasmocrqualitythresholds_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number, c1: number) => number;
    readonly wasmocrqualitythresholds_nonTextMinChars: (a: number) => number;
    readonly wasmocrqualitythresholds_pipelineMinQuality: (a: number) => number;
    readonly wasmocrqualitythresholds_set_alnumWsRatioThreshold: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_criticalFragmentedWordRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_enableProvenanceOcrRouting: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_maxFragmentedWordRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minAlnumRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minAvgWordLength: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minConsecutiveRepeatRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minGarbageChars: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minMeaningfulWordLen: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minMeaningfulWords: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minNonWhitespacePerPage: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minProvenanceFallbackRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minTotalNonWhitespace: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minUndecodableRatio: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minWordsForAvgLengthCheck: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_minWordsForRepeatCheck: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_nonTextMinChars: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_pipelineMinQuality: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_set_substantiveMinChars: (a: number, b: number) => void;
    readonly wasmocrqualitythresholds_substantiveMinChars: (a: number) => number;
    readonly wasmocrrotation_default: () => number;
    readonly wasmocrrotation_new: (a: number, b: number, c: number) => number;
    readonly wasmocrstrategy_default: () => number;
    readonly wasmocrstrategy_minConfidence: (a: number) => [number, number];
    readonly wasmocrstrategy_mode: (a: number) => [number, number];
    readonly wasmocrstrategy_set_minConfidence: (a: number, b: number, c: number) => void;
    readonly wasmocrstrategy_set_mode: (a: number, b: number, c: number) => void;
    readonly wasmocrtable_boundingBox: (a: number) => number;
    readonly wasmocrtable_cells: (a: number) => any;
    readonly wasmocrtable_default: () => number;
    readonly wasmocrtable_markdown: (a: number) => [number, number];
    readonly wasmocrtable_new: (a: any, b: number, c: number, d: number, e: number) => number;
    readonly wasmocrtable_pageNumber: (a: number) => number;
    readonly wasmocrtable_set_boundingBox: (a: number, b: number) => void;
    readonly wasmocrtable_set_cells: (a: number, b: any) => void;
    readonly wasmocrtable_set_markdown: (a: number, b: number, c: number) => void;
    readonly wasmocrtable_set_pageNumber: (a: number, b: number) => void;
    readonly wasmocrtableboundingbox_bottom: (a: number) => number;
    readonly wasmocrtableboundingbox_default: () => number;
    readonly wasmocrtableboundingbox_left: (a: number) => number;
    readonly wasmocrtableboundingbox_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmocrtableboundingbox_right: (a: number) => number;
    readonly wasmocrtableboundingbox_set_bottom: (a: number, b: number) => void;
    readonly wasmocrtableboundingbox_set_left: (a: number, b: number) => void;
    readonly wasmocrtableboundingbox_set_right: (a: number, b: number) => void;
    readonly wasmocrtableboundingbox_set_top: (a: number, b: number) => void;
    readonly wasmocrtableboundingbox_top: (a: number) => number;
    readonly wasmpageboundary_default: () => number;
    readonly wasmpageboundary_new: (a: number, b: number, c: number) => number;
    readonly wasmpageclassification_default: () => number;
    readonly wasmpageclassification_labels: (a: number) => [number, number];
    readonly wasmpageclassification_new: (a: number, b: number, c: number) => number;
    readonly wasmpageclassification_pageNumber: (a: number) => number;
    readonly wasmpageclassification_set_labels: (a: number, b: number, c: number) => void;
    readonly wasmpageclassification_set_pageNumber: (a: number, b: number) => void;
    readonly wasmpageclassificationconfig_default: () => number;
    readonly wasmpageclassificationconfig_labels: (a: number) => [number, number];
    readonly wasmpageclassificationconfig_llm: (a: number) => number;
    readonly wasmpageclassificationconfig_multiLabel: (a: number) => number;
    readonly wasmpageclassificationconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmpageclassificationconfig_promptTemplate: (a: number) => [number, number];
    readonly wasmpageclassificationconfig_set_labels: (a: number, b: number, c: number) => void;
    readonly wasmpageclassificationconfig_set_llm: (a: number, b: number) => void;
    readonly wasmpageclassificationconfig_set_multiLabel: (a: number, b: number) => void;
    readonly wasmpageclassificationconfig_set_promptTemplate: (a: number, b: number, c: number) => void;
    readonly wasmpageconfig_default: () => number;
    readonly wasmpageconfig_extractPages: (a: number) => number;
    readonly wasmpageconfig_insertPageMarkers: (a: number) => number;
    readonly wasmpageconfig_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmpageconfig_set_extractPages: (a: number, b: number) => void;
    readonly wasmpageconfig_set_insertPageMarkers: (a: number, b: number) => void;
    readonly wasmpagecontent_content: (a: number) => [number, number];
    readonly wasmpagecontent_default: () => number;
    readonly wasmpagecontent_hierarchy: (a: number) => number;
    readonly wasmpagecontent_imageIndices: (a: number) => [number, number];
    readonly wasmpagecontent_isBlank: (a: number) => number;
    readonly wasmpagecontent_layoutRegions: (a: number) => any;
    readonly wasmpagecontent_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number) => number;
    readonly wasmpagecontent_pageNumber: (a: number) => number;
    readonly wasmpagecontent_sectionName: (a: number) => [number, number];
    readonly wasmpagecontent_set_content: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_hierarchy: (a: number, b: number) => void;
    readonly wasmpagecontent_set_imageIndices: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_isBlank: (a: number, b: number) => void;
    readonly wasmpagecontent_set_layoutRegions: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_pageNumber: (a: number, b: number) => void;
    readonly wasmpagecontent_set_sectionName: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_sheetName: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_speakerNotes: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_set_tables: (a: number, b: number, c: number) => void;
    readonly wasmpagecontent_sheetName: (a: number) => [number, number];
    readonly wasmpagecontent_speakerNotes: (a: number) => [number, number];
    readonly wasmpagecontent_tables: (a: number) => [number, number];
    readonly wasmpagehierarchy_blockCount: (a: number) => number;
    readonly wasmpagehierarchy_blocks: (a: number) => [number, number];
    readonly wasmpagehierarchy_new: (a: number, b: number, c: number) => number;
    readonly wasmpagehierarchy_set_blockCount: (a: number, b: number) => void;
    readonly wasmpagehierarchy_set_blocks: (a: number, b: number, c: number) => void;
    readonly wasmpageinfo_default: () => number;
    readonly wasmpageinfo_hasVectorGraphics: (a: number) => number;
    readonly wasmpageinfo_hidden: (a: number) => number;
    readonly wasmpageinfo_imageCount: (a: number) => number;
    readonly wasmpageinfo_isBlank: (a: number) => number;
    readonly wasmpageinfo_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmpageinfo_number: (a: number) => number;
    readonly wasmpageinfo_set_hasVectorGraphics: (a: number, b: number) => void;
    readonly wasmpageinfo_set_hidden: (a: number, b: number) => void;
    readonly wasmpageinfo_set_imageCount: (a: number, b: number) => void;
    readonly wasmpageinfo_set_isBlank: (a: number, b: number) => void;
    readonly wasmpageinfo_set_number: (a: number, b: number) => void;
    readonly wasmpageinfo_set_tableCount: (a: number, b: number) => void;
    readonly wasmpageinfo_set_title: (a: number, b: number, c: number) => void;
    readonly wasmpageinfo_tableCount: (a: number) => number;
    readonly wasmpageinfo_title: (a: number) => [number, number];
    readonly wasmpagespan_bbox: (a: number) => number;
    readonly wasmpagespan_default: () => number;
    readonly wasmpagespan_new: (a: number, b: number) => number;
    readonly wasmpagespan_page: (a: number) => number;
    readonly wasmpagespan_set_bbox: (a: number, b: number) => void;
    readonly wasmpagespan_set_page: (a: number, b: number) => void;
    readonly wasmpagestructure_boundaries: (a: number) => any;
    readonly wasmpagestructure_default: () => number;
    readonly wasmpagestructure_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmpagestructure_pages: (a: number) => any;
    readonly wasmpagestructure_set_boundaries: (a: number, b: number, c: number) => void;
    readonly wasmpagestructure_set_pages: (a: number, b: number, c: number) => void;
    readonly wasmpagestructure_set_totalCount: (a: number, b: number) => void;
    readonly wasmpagestructure_set_unitType: (a: number, b: number) => void;
    readonly wasmpagestructure_totalCount: (a: number) => number;
    readonly wasmpagestructure_unitType: (a: number) => [number, number];
    readonly wasmpatternmatch_category: (a: number) => [number, number];
    readonly wasmpatternmatch_default: () => number;
    readonly wasmpatternmatch_end: (a: number) => number;
    readonly wasmpatternmatch_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmpatternmatch_set_category: (a: number, b: number) => void;
    readonly wasmpatternmatch_set_end: (a: number, b: number) => void;
    readonly wasmpdfannotation_annotationType: (a: number) => [number, number];
    readonly wasmpdfannotation_boundingBox: (a: number) => number;
    readonly wasmpdfannotation_content: (a: number) => [number, number];
    readonly wasmpdfannotation_default: () => number;
    readonly wasmpdfannotation_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmpdfannotation_pageNumber: (a: number) => number;
    readonly wasmpdfannotation_set_annotationType: (a: number, b: number) => void;
    readonly wasmpdfannotation_set_boundingBox: (a: number, b: number) => void;
    readonly wasmpdfannotation_set_content: (a: number, b: number, c: number) => void;
    readonly wasmpdfannotation_set_pageNumber: (a: number, b: number) => void;
    readonly wasmpdfformfield_bbox: (a: number) => number;
    readonly wasmpdfformfield_default: () => number;
    readonly wasmpdfformfield_defaultValue: (a: number) => [number, number];
    readonly wasmpdfformfield_fieldType: (a: number) => [number, number];
    readonly wasmpdfformfield_flags: (a: number) => number;
    readonly wasmpdfformfield_fullName: (a: number) => [number, number];
    readonly wasmpdfformfield_maxLength: (a: number) => number;
    readonly wasmpdfformfield_name: (a: number) => [number, number];
    readonly wasmpdfformfield_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number) => number;
    readonly wasmpdfformfield_page: (a: number) => number;
    readonly wasmpdfformfield_set_bbox: (a: number, b: number) => void;
    readonly wasmpdfformfield_set_defaultValue: (a: number, b: number, c: number) => void;
    readonly wasmpdfformfield_set_fieldType: (a: number, b: number) => void;
    readonly wasmpdfformfield_set_flags: (a: number, b: number) => void;
    readonly wasmpdfformfield_set_fullName: (a: number, b: number, c: number) => void;
    readonly wasmpdfformfield_set_maxLength: (a: number, b: number) => void;
    readonly wasmpdfformfield_set_name: (a: number, b: number, c: number) => void;
    readonly wasmpdfformfield_set_page: (a: number, b: number) => void;
    readonly wasmpdfformfield_set_tooltip: (a: number, b: number, c: number) => void;
    readonly wasmpdfformfield_set_value: (a: number, b: number, c: number) => void;
    readonly wasmpdfformfield_tooltip: (a: number) => [number, number];
    readonly wasmpdfformfield_value: (a: number) => [number, number];
    readonly wasmpostprocessorconfig_default: () => number;
    readonly wasmpostprocessorconfig_disabledProcessors: (a: number) => [number, number];
    readonly wasmpostprocessorconfig_disabledSet: (a: number) => [number, number];
    readonly wasmpostprocessorconfig_enabled: (a: number) => number;
    readonly wasmpostprocessorconfig_enabledProcessors: (a: number) => [number, number];
    readonly wasmpostprocessorconfig_enabledSet: (a: number) => [number, number];
    readonly wasmpostprocessorconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmpostprocessorconfig_set_disabledProcessors: (a: number, b: number, c: number) => void;
    readonly wasmpostprocessorconfig_set_disabledSet: (a: number, b: number, c: number) => void;
    readonly wasmpostprocessorconfig_set_enabled: (a: number, b: number) => void;
    readonly wasmpostprocessorconfig_set_enabledProcessors: (a: number, b: number, c: number) => void;
    readonly wasmpostprocessorconfig_set_enabledSet: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_appVersion: (a: number) => [number, number];
    readonly wasmpptxappproperties_application: (a: number) => [number, number];
    readonly wasmpptxappproperties_company: (a: number) => [number, number];
    readonly wasmpptxappproperties_default: () => number;
    readonly wasmpptxappproperties_docSecurity: (a: number) => number;
    readonly wasmpptxappproperties_hiddenSlides: (a: number) => number;
    readonly wasmpptxappproperties_hyperlinksChanged: (a: number) => number;
    readonly wasmpptxappproperties_linksUpToDate: (a: number) => number;
    readonly wasmpptxappproperties_multimediaClips: (a: number) => number;
    readonly wasmpptxappproperties_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number) => number;
    readonly wasmpptxappproperties_notes: (a: number) => number;
    readonly wasmpptxappproperties_presentationFormat: (a: number) => [number, number];
    readonly wasmpptxappproperties_scaleCrop: (a: number) => number;
    readonly wasmpptxappproperties_set_appVersion: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_set_application: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_set_company: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_set_docSecurity: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_hiddenSlides: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_hyperlinksChanged: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_linksUpToDate: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_multimediaClips: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_notes: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_presentationFormat: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_set_scaleCrop: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_sharedDoc: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_slideTitles: (a: number, b: number, c: number) => void;
    readonly wasmpptxappproperties_set_slides: (a: number, b: number) => void;
    readonly wasmpptxappproperties_set_totalTime: (a: number, b: number) => void;
    readonly wasmpptxappproperties_sharedDoc: (a: number) => number;
    readonly wasmpptxappproperties_slideTitles: (a: number) => [number, number];
    readonly wasmpptxappproperties_slides: (a: number) => number;
    readonly wasmpptxappproperties_totalTime: (a: number) => number;
    readonly wasmpptxextractionresult_content: (a: number) => [number, number];
    readonly wasmpptxextractionresult_default: () => number;
    readonly wasmpptxextractionresult_document: (a: number) => number;
    readonly wasmpptxextractionresult_imageCount: (a: number) => number;
    readonly wasmpptxextractionresult_images: (a: number) => [number, number];
    readonly wasmpptxextractionresult_metadata: (a: number) => number;
    readonly wasmpptxextractionresult_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: any, j: number, k: number, l: number, m: number, n: number, o: number) => number;
    readonly wasmpptxextractionresult_officeMetadata: (a: number) => any;
    readonly wasmpptxextractionresult_pageContents: (a: number) => any;
    readonly wasmpptxextractionresult_pageStructure: (a: number) => number;
    readonly wasmpptxextractionresult_revisions: (a: number) => any;
    readonly wasmpptxextractionresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmpptxextractionresult_set_document: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_set_imageCount: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_set_images: (a: number, b: number, c: number) => void;
    readonly wasmpptxextractionresult_set_metadata: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_set_officeMetadata: (a: number, b: any) => void;
    readonly wasmpptxextractionresult_set_pageContents: (a: number, b: number, c: number) => void;
    readonly wasmpptxextractionresult_set_pageStructure: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_set_revisions: (a: number, b: number, c: number) => void;
    readonly wasmpptxextractionresult_set_slideCount: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_set_tableCount: (a: number, b: number) => void;
    readonly wasmpptxextractionresult_slideCount: (a: number) => number;
    readonly wasmpptxextractionresult_tableCount: (a: number) => number;
    readonly wasmpptxmetadata_default: () => number;
    readonly wasmpptxmetadata_imageCount: (a: number) => number;
    readonly wasmpptxmetadata_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmpptxmetadata_set_imageCount: (a: number, b: number) => void;
    readonly wasmpptxmetadata_set_slideCount: (a: number, b: number) => void;
    readonly wasmpptxmetadata_set_slideNames: (a: number, b: number, c: number) => void;
    readonly wasmpptxmetadata_set_tableCount: (a: number, b: number) => void;
    readonly wasmpptxmetadata_slideCount: (a: number) => number;
    readonly wasmpptxmetadata_slideNames: (a: number) => [number, number];
    readonly wasmpptxmetadata_tableCount: (a: number) => number;
    readonly wasmpreprocessingoptions_default: () => number;
    readonly wasmpreprocessingoptions_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmpreprocessingoptions_preset: (a: number) => [number, number];
    readonly wasmpreprocessingoptions_set_preset: (a: number, b: number) => void;
    readonly wasmpropertychange_default: () => number;
    readonly wasmpropertychange_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmproxyconfig_default: () => number;
    readonly wasmproxyconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmpstmetadata_default: () => number;
    readonly wasmpstmetadata_messageCount: (a: number) => number;
    readonly wasmpstmetadata_new: (a: number) => number;
    readonly wasmpstmetadata_set_messageCount: (a: number, b: number) => void;
    readonly wasmqrcode_bbox: (a: number) => number;
    readonly wasmqrcode_confidence: (a: number) => number;
    readonly wasmqrcode_default: () => number;
    readonly wasmqrcode_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmqrcode_payload: (a: number) => [number, number];
    readonly wasmqrcode_set_bbox: (a: number, b: number) => void;
    readonly wasmqrcode_set_confidence: (a: number, b: number) => void;
    readonly wasmqrcode_set_payload: (a: number, b: number, c: number) => void;
    readonly wasmredactionconfig_categories: (a: number) => [number, number];
    readonly wasmredactionconfig_customPatterns: (a: number) => [number, number];
    readonly wasmredactionconfig_customTerms: (a: number) => [number, number];
    readonly wasmredactionconfig_default: () => number;
    readonly wasmredactionconfig_ner: (a: number) => number;
    readonly wasmredactionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmredactionconfig_preserveOffsets: (a: number) => number;
    readonly wasmredactionconfig_set_categories: (a: number, b: number, c: number) => void;
    readonly wasmredactionconfig_set_customPatterns: (a: number, b: number, c: number) => void;
    readonly wasmredactionconfig_set_customTerms: (a: number, b: number, c: number) => void;
    readonly wasmredactionconfig_set_ner: (a: number, b: number) => void;
    readonly wasmredactionconfig_set_preserveOffsets: (a: number, b: number) => void;
    readonly wasmredactionconfig_set_strategy: (a: number, b: number) => void;
    readonly wasmredactionconfig_strategy: (a: number) => [number, number];
    readonly wasmredactionconfig_validate: (a: number) => [number, number];
    readonly wasmredactionfinding_category: (a: number) => [number, number];
    readonly wasmredactionfinding_default: () => number;
    readonly wasmredactionfinding_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmredactionfinding_set_category: (a: number, b: number) => void;
    readonly wasmredactionfinding_set_strategy: (a: number, b: number) => void;
    readonly wasmredactionfinding_strategy: (a: number) => [number, number];
    readonly wasmredactionpattern_caseSensitive: (a: number) => number;
    readonly wasmredactionpattern_default: () => number;
    readonly wasmredactionpattern_labeled: (a: number, b: number, c: number, d: number) => number;
    readonly wasmredactionpattern_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmredactionpattern_set_caseSensitive: (a: number, b: number) => void;
    readonly wasmredactionreport_default: () => number;
    readonly wasmredactionreport_findings: (a: number) => [number, number];
    readonly wasmredactionreport_new: (a: number, b: number, c: number) => number;
    readonly wasmredactionreport_set_findings: (a: number, b: number, c: number) => void;
    readonly wasmredactionreport_set_totalRedacted: (a: number, b: number) => void;
    readonly wasmredactionreport_totalRedacted: (a: number) => number;
    readonly wasmredactionterm_literal: (a: number, b: number) => number;
    readonly wasmrerankerconfig_default: () => number;
    readonly wasmrerankerconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: bigint) => number;
    readonly wasmrerankermodeltype_additionalFiles: (a: number) => [number, number];
    readonly wasmrerankermodeltype_default: () => number;
    readonly wasmrerankermodeltype_head: (a: number) => number;
    readonly wasmrerankermodeltype_llm: (a: number) => number;
    readonly wasmrerankermodeltype_maxLength: (a: number) => [number, bigint];
    readonly wasmrerankermodeltype_modelFile: (a: number) => [number, number];
    readonly wasmrerankermodeltype_modelId: (a: number) => [number, number];
    readonly wasmrerankermodeltype_name: (a: number) => [number, number];
    readonly wasmrerankermodeltype_set_additionalFiles: (a: number, b: number, c: number) => void;
    readonly wasmrerankermodeltype_set_head: (a: number, b: number) => void;
    readonly wasmrerankermodeltype_set_llm: (a: number, b: number) => void;
    readonly wasmrerankermodeltype_set_maxLength: (a: number, b: number, c: bigint) => void;
    readonly wasmrerankermodeltype_set_modelFile: (a: number, b: number, c: number) => void;
    readonly wasmrerankermodeltype_set_modelId: (a: number, b: number, c: number) => void;
    readonly wasmrerankermodeltype_set_name: (a: number, b: number, c: number) => void;
    readonly wasmrerankermodeltype_set_type: (a: number, b: number, c: number) => void;
    readonly wasmrerankermodeltype_type: (a: number) => [number, number];
    readonly wasmrevisionanchor_col: (a: number) => number;
    readonly wasmrevisionanchor_default: () => number;
    readonly wasmrevisionanchor_index: (a: number) => number;
    readonly wasmrevisionanchor_name: (a: number) => [number, number];
    readonly wasmrevisionanchor_row: (a: number) => number;
    readonly wasmrevisionanchor_set_col: (a: number, b: number) => void;
    readonly wasmrevisionanchor_set_index: (a: number, b: number) => void;
    readonly wasmrevisionanchor_set_name: (a: number, b: number, c: number) => void;
    readonly wasmrevisionanchor_set_row: (a: number, b: number) => void;
    readonly wasmrevisionanchor_set_tableIndex: (a: number, b: number) => void;
    readonly wasmrevisionanchor_set_type: (a: number, b: number, c: number) => void;
    readonly wasmrevisionanchor_tableIndex: (a: number) => number;
    readonly wasmrevisionanchor_type: (a: number) => [number, number];
    readonly wasmrevisiondelta_content: (a: number) => any;
    readonly wasmrevisiondelta_default: () => number;
    readonly wasmrevisiondelta_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmrevisiondelta_propertyChanges: (a: number) => [number, number];
    readonly wasmrevisiondelta_set_content: (a: number, b: any) => void;
    readonly wasmrevisiondelta_set_propertyChanges: (a: number, b: number, c: number) => void;
    readonly wasmrevisiondelta_set_tableChanges: (a: number, b: number, c: number) => void;
    readonly wasmrevisiondelta_tableChanges: (a: number) => [number, number];
    readonly wasmsecuritylimits_default: () => number;
    readonly wasmsecuritylimits_maxArchiveSize: (a: number) => number;
    readonly wasmsecuritylimits_maxCompressionRatio: (a: number) => number;
    readonly wasmsecuritylimits_maxContentSize: (a: number) => number;
    readonly wasmsecuritylimits_maxEntityLength: (a: number) => number;
    readonly wasmsecuritylimits_maxFilesInArchive: (a: number) => number;
    readonly wasmsecuritylimits_maxIterations: (a: number) => number;
    readonly wasmsecuritylimits_maxNestingDepth: (a: number) => number;
    readonly wasmsecuritylimits_maxTableCells: (a: number) => number;
    readonly wasmsecuritylimits_maxXmlDepth: (a: number) => number;
    readonly wasmsecuritylimits_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmsecuritylimits_set_maxArchiveSize: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxCompressionRatio: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxContentSize: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxEntityLength: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxFilesInArchive: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxIterations: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxNestingDepth: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxTableCells: (a: number, b: number) => void;
    readonly wasmsecuritylimits_set_maxXmlDepth: (a: number, b: number) => void;
    readonly wasmsitemapurl_changefreq: (a: number) => [number, number];
    readonly wasmsitemapurl_default: () => number;
    readonly wasmsitemapurl_lastmod: (a: number) => [number, number];
    readonly wasmsitemapurl_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmsitemapurl_priority: (a: number) => [number, number];
    readonly wasmsitemapurl_set_changefreq: (a: number, b: number, c: number) => void;
    readonly wasmsitemapurl_set_lastmod: (a: number, b: number, c: number) => void;
    readonly wasmsitemapurl_set_priority: (a: number, b: number, c: number) => void;
    readonly wasmsitemapurl_set_url: (a: number, b: number, c: number) => void;
    readonly wasmsitemapurl_url: (a: number) => [number, number];
    readonly wasmsparseembeddingconfig_default: () => number;
    readonly wasmsparseembeddingconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: bigint) => number;
    readonly wasmsparseembeddingconfig_set_showDownloadProgress: (a: number, b: number) => void;
    readonly wasmsparseembeddingconfig_showDownloadProgress: (a: number) => number;
    readonly wasmssrfpolicy_default: () => number;
    readonly wasmssrfpolicy_maxRedirects: (a: number) => number;
    readonly wasmssrfpolicy_new: (a: number, b: number) => number;
    readonly wasmssrfpolicy_set_maxRedirects: (a: number, b: number) => void;
    readonly wasmstructureddata_dataType: (a: number) => [number, number];
    readonly wasmstructureddata_default: () => number;
    readonly wasmstructureddata_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmstructureddata_rawJson: (a: number) => [number, number];
    readonly wasmstructureddata_schemaType: (a: number) => [number, number];
    readonly wasmstructureddata_set_dataType: (a: number, b: number) => void;
    readonly wasmstructureddata_set_rawJson: (a: number, b: number, c: number) => void;
    readonly wasmstructureddata_set_schemaType: (a: number, b: number, c: number) => void;
    readonly wasmstructureddataresult_content: (a: number) => [number, number];
    readonly wasmstructureddataresult_default: () => number;
    readonly wasmstructureddataresult_format: (a: number) => [number, number];
    readonly wasmstructureddataresult_metadata: (a: number) => any;
    readonly wasmstructureddataresult_new: (a: number, b: number, c: number, d: number, e: any, f: number, g: number) => number;
    readonly wasmstructureddataresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmstructureddataresult_set_format: (a: number, b: number, c: number) => void;
    readonly wasmstructureddataresult_set_metadata: (a: number, b: any) => void;
    readonly wasmstructureddataresult_set_textFields: (a: number, b: number, c: number) => void;
    readonly wasmstructureddataresult_textFields: (a: number) => [number, number];
    readonly wasmstructuredextractionconfig_default: () => number;
    readonly wasmstructuredextractionconfig_llm: (a: number) => number;
    readonly wasmstructuredextractionconfig_new: (a: any, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmstructuredextractionconfig_prompt: (a: number) => [number, number];
    readonly wasmstructuredextractionconfig_schema: (a: number) => any;
    readonly wasmstructuredextractionconfig_schemaDescription: (a: number) => [number, number];
    readonly wasmstructuredextractionconfig_schemaName: (a: number) => [number, number];
    readonly wasmstructuredextractionconfig_set_llm: (a: number, b: number) => void;
    readonly wasmstructuredextractionconfig_set_prompt: (a: number, b: number, c: number) => void;
    readonly wasmstructuredextractionconfig_set_schema: (a: number, b: any) => void;
    readonly wasmstructuredextractionconfig_set_schemaDescription: (a: number, b: number, c: number) => void;
    readonly wasmstructuredextractionconfig_set_schemaName: (a: number, b: number, c: number) => void;
    readonly wasmstructuredextractionconfig_set_strict: (a: number, b: number) => void;
    readonly wasmstructuredextractionconfig_strict: (a: number) => number;
    readonly wasmsummarizationconfig_default: () => number;
    readonly wasmsummarizationconfig_llm: (a: number) => number;
    readonly wasmsummarizationconfig_maxTokens: (a: number) => number;
    readonly wasmsummarizationconfig_new: (a: number, b: number, c: number) => number;
    readonly wasmsummarizationconfig_set_llm: (a: number, b: number) => void;
    readonly wasmsummarizationconfig_set_maxTokens: (a: number, b: number) => void;
    readonly wasmsummarizationconfig_set_strategy: (a: number, b: number) => void;
    readonly wasmsummarizationconfig_strategy: (a: number) => [number, number];
    readonly wasmtable_boundingBox: (a: number) => number;
    readonly wasmtable_cells: (a: number) => any;
    readonly wasmtable_columns: (a: number) => [number, number];
    readonly wasmtable_default: () => number;
    readonly wasmtable_markdown: (a: number) => [number, number];
    readonly wasmtable_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly wasmtable_pageNumber: (a: number) => number;
    readonly wasmtable_set_boundingBox: (a: number, b: number) => void;
    readonly wasmtable_set_cells: (a: number, b: any) => void;
    readonly wasmtable_set_columns: (a: number, b: number, c: number) => void;
    readonly wasmtable_set_markdown: (a: number, b: number, c: number) => void;
    readonly wasmtable_set_pageNumber: (a: number, b: number) => void;
    readonly wasmtable_set_tableId: (a: number, b: number, c: number) => void;
    readonly wasmtable_tableId: (a: number) => [number, number];
    readonly wasmtablecell_default: () => number;
    readonly wasmtablecell_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmtablegrid_cells: (a: number) => [number, number];
    readonly wasmtablegrid_cols: (a: number) => number;
    readonly wasmtablegrid_default: () => number;
    readonly wasmtablegrid_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmtablegrid_rows: (a: number) => number;
    readonly wasmtablegrid_set_cells: (a: number, b: number, c: number) => void;
    readonly wasmtablegrid_set_cols: (a: number, b: number) => void;
    readonly wasmtablegrid_set_rows: (a: number, b: number) => void;
    readonly wasmtesseractconfig_classifyUsePreAdaptedTemplates: (a: number) => number;
    readonly wasmtesseractconfig_default: () => number;
    readonly wasmtesseractconfig_enableTableDetection: (a: number) => number;
    readonly wasmtesseractconfig_language: (a: number) => [number, number];
    readonly wasmtesseractconfig_languageModelNgramOn: (a: number) => number;
    readonly wasmtesseractconfig_minConfidence: (a: number) => number;
    readonly wasmtesseractconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number, p: number, q: number, r: number, s: number, t: number, u: number, v: number, w: number, x: number, y: number, z: number, a1: number, b1: number) => number;
    readonly wasmtesseractconfig_oem: (a: number) => number;
    readonly wasmtesseractconfig_outputFormat: (a: number) => [number, number];
    readonly wasmtesseractconfig_preprocessing: (a: number) => number;
    readonly wasmtesseractconfig_psm: (a: number) => number;
    readonly wasmtesseractconfig_set_classifyUsePreAdaptedTemplates: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_enableTableDetection: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_language: (a: number, b: number, c: number) => void;
    readonly wasmtesseractconfig_set_languageModelNgramOn: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_minConfidence: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_oem: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_outputFormat: (a: number, b: number, c: number) => void;
    readonly wasmtesseractconfig_set_preprocessing: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_psm: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tableColumnThreshold: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tableMinConfidence: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tableRowThresholdRatio: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tesseditCharBlacklist: (a: number, b: number, c: number) => void;
    readonly wasmtesseractconfig_set_tesseditCharWhitelist: (a: number, b: number, c: number) => void;
    readonly wasmtesseractconfig_set_tesseditDontBlkrejGoodWds: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tesseditDontRowrejGoodWds: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tesseditEnableDictCorrection: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_tesseditUsePrimaryParamsModel: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_textordSpaceSizeIsVariable: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_thresholdingMethod: (a: number, b: number) => void;
    readonly wasmtesseractconfig_set_useCache: (a: number, b: number) => void;
    readonly wasmtesseractconfig_tableColumnThreshold: (a: number) => number;
    readonly wasmtesseractconfig_tableMinConfidence: (a: number) => number;
    readonly wasmtesseractconfig_tableRowThresholdRatio: (a: number) => number;
    readonly wasmtesseractconfig_tesseditCharBlacklist: (a: number) => [number, number];
    readonly wasmtesseractconfig_tesseditCharWhitelist: (a: number) => [number, number];
    readonly wasmtesseractconfig_tesseditDontBlkrejGoodWds: (a: number) => number;
    readonly wasmtesseractconfig_tesseditDontRowrejGoodWds: (a: number) => number;
    readonly wasmtesseractconfig_tesseditEnableDictCorrection: (a: number) => number;
    readonly wasmtesseractconfig_tesseditUsePrimaryParamsModel: (a: number) => number;
    readonly wasmtesseractconfig_textordSpaceSizeIsVariable: (a: number) => number;
    readonly wasmtesseractconfig_thresholdingMethod: (a: number) => number;
    readonly wasmtesseractconfig_useCache: (a: number) => number;
    readonly wasmtextannotation_default: () => number;
    readonly wasmtextannotation_end: (a: number) => number;
    readonly wasmtextannotation_kind: (a: number) => any;
    readonly wasmtextannotation_set_end: (a: number, b: number) => void;
    readonly wasmtextannotation_set_kind: (a: number, b: any) => void;
    readonly wasmtextannotation_set_start: (a: number, b: number) => void;
    readonly wasmtextannotation_start: (a: number) => number;
    readonly wasmtextextractionresult_characterCount: (a: number) => number;
    readonly wasmtextextractionresult_content: (a: number) => [number, number];
    readonly wasmtextextractionresult_default: () => number;
    readonly wasmtextextractionresult_headers: (a: number) => [number, number];
    readonly wasmtextextractionresult_lineCount: (a: number) => number;
    readonly wasmtextextractionresult_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly wasmtextextractionresult_set_characterCount: (a: number, b: number) => void;
    readonly wasmtextextractionresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmtextextractionresult_set_headers: (a: number, b: number, c: number) => void;
    readonly wasmtextextractionresult_set_lineCount: (a: number, b: number) => void;
    readonly wasmtextextractionresult_set_wordCount: (a: number, b: number) => void;
    readonly wasmtextextractionresult_wordCount: (a: number) => number;
    readonly wasmtextmetadata_characterCount: (a: number) => number;
    readonly wasmtextmetadata_default: () => number;
    readonly wasmtextmetadata_headers: (a: number) => [number, number];
    readonly wasmtextmetadata_lineCount: (a: number) => number;
    readonly wasmtextmetadata_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmtextmetadata_set_characterCount: (a: number, b: number) => void;
    readonly wasmtextmetadata_set_headers: (a: number, b: number, c: number) => void;
    readonly wasmtextmetadata_set_lineCount: (a: number, b: number) => void;
    readonly wasmtextmetadata_set_wordCount: (a: number, b: number) => void;
    readonly wasmtextmetadata_wordCount: (a: number) => number;
    readonly wasmtokenreductionoptions_default: () => number;
    readonly wasmtokenreductionoptions_new: (a: number, b: number, c: number) => number;
    readonly wasmtranslation_content: (a: number) => [number, number];
    readonly wasmtranslation_default: () => number;
    readonly wasmtranslation_formattedContent: (a: number) => [number, number];
    readonly wasmtranslation_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmtranslation_set_content: (a: number, b: number, c: number) => void;
    readonly wasmtranslation_set_formattedContent: (a: number, b: number, c: number) => void;
    readonly wasmtranslation_set_sourceLang: (a: number, b: number, c: number) => void;
    readonly wasmtranslation_set_targetLang: (a: number, b: number, c: number) => void;
    readonly wasmtranslation_sourceLang: (a: number) => [number, number];
    readonly wasmtranslation_targetLang: (a: number) => [number, number];
    readonly wasmtranslationconfig_default: () => number;
    readonly wasmtranslationconfig_llm: (a: number) => number;
    readonly wasmtranslationconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmtranslationconfig_preserveMarkup: (a: number) => number;
    readonly wasmtranslationconfig_set_llm: (a: number, b: number) => void;
    readonly wasmtranslationconfig_set_preserveMarkup: (a: number, b: number) => void;
    readonly wasmtranslationconfig_set_sourceLang: (a: number, b: number, c: number) => void;
    readonly wasmtranslationconfig_set_targetLang: (a: number, b: number, c: number) => void;
    readonly wasmtranslationconfig_sourceLang: (a: number) => [number, number];
    readonly wasmtranslationconfig_targetLang: (a: number) => [number, number];
    readonly wasmurlextractionconfig_allowFileUris: (a: number) => number;
    readonly wasmurlextractionconfig_allowLocalFileInputs: (a: number) => number;
    readonly wasmurlextractionconfig_crawl: (a: number) => number;
    readonly wasmurlextractionconfig_default: () => number;
    readonly wasmurlextractionconfig_documentUrlPattern: (a: number) => [number, number];
    readonly wasmurlextractionconfig_maxDocumentUrlsPerResult: (a: number) => number;
    readonly wasmurlextractionconfig_maxTotalUrls: (a: number) => number;
    readonly wasmurlextractionconfig_mode: (a: number) => [number, number];
    readonly wasmurlextractionconfig_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly wasmurlextractionconfig_set_allowFileUris: (a: number, b: number) => void;
    readonly wasmurlextractionconfig_set_allowLocalFileInputs: (a: number, b: number) => void;
    readonly wasmurlextractionconfig_set_crawl: (a: number, b: number) => void;
    readonly wasmurlextractionconfig_set_documentUrlPattern: (a: number, b: number, c: number) => void;
    readonly wasmurlextractionconfig_set_maxDocumentUrlsPerResult: (a: number, b: number) => void;
    readonly wasmurlextractionconfig_set_maxTotalUrls: (a: number, b: number) => void;
    readonly wasmurlextractionconfig_set_mode: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_appVersion: (a: number) => [number, number];
    readonly wasmxlsxappproperties_application: (a: number) => [number, number];
    readonly wasmxlsxappproperties_company: (a: number) => [number, number];
    readonly wasmxlsxappproperties_default: () => number;
    readonly wasmxlsxappproperties_docSecurity: (a: number) => number;
    readonly wasmxlsxappproperties_hyperlinksChanged: (a: number) => number;
    readonly wasmxlsxappproperties_linksUpToDate: (a: number) => number;
    readonly wasmxlsxappproperties_new: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => number;
    readonly wasmxlsxappproperties_scaleCrop: (a: number) => number;
    readonly wasmxlsxappproperties_set_appVersion: (a: number, b: number, c: number) => void;
    readonly wasmxlsxappproperties_set_application: (a: number, b: number, c: number) => void;
    readonly wasmxlsxappproperties_set_company: (a: number, b: number, c: number) => void;
    readonly wasmxlsxappproperties_set_docSecurity: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_set_hyperlinksChanged: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_set_linksUpToDate: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_set_scaleCrop: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_set_sharedDoc: (a: number, b: number) => void;
    readonly wasmxlsxappproperties_set_worksheetNames: (a: number, b: number, c: number) => void;
    readonly wasmxlsxappproperties_sharedDoc: (a: number) => number;
    readonly wasmxlsxappproperties_worksheetNames: (a: number) => [number, number];
    readonly wasmxmlextractionresult_content: (a: number) => [number, number];
    readonly wasmxmlextractionresult_default: () => number;
    readonly wasmxmlextractionresult_elementCount: (a: number) => number;
    readonly wasmxmlextractionresult_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmxmlextractionresult_set_content: (a: number, b: number, c: number) => void;
    readonly wasmxmlextractionresult_set_elementCount: (a: number, b: number) => void;
    readonly wasmxmlextractionresult_set_uniqueElements: (a: number, b: number, c: number) => void;
    readonly wasmxmlextractionresult_uniqueElements: (a: number) => [number, number];
    readonly wasmxmlmetadata_default: () => number;
    readonly wasmxmlmetadata_elementCount: (a: number) => number;
    readonly wasmxmlmetadata_new: (a: number, b: number, c: number) => number;
    readonly wasmxmlmetadata_set_elementCount: (a: number, b: number) => void;
    readonly wasmxmlmetadata_set_uniqueElements: (a: number, b: number, c: number) => void;
    readonly wasmxmlmetadata_uniqueElements: (a: number) => [number, number];
    readonly wasmyearrange_default: () => number;
    readonly wasmyearrange_max: (a: number) => number;
    readonly wasmyearrange_min: (a: number) => number;
    readonly wasmyearrange_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmyearrange_set_max: (a: number, b: number) => void;
    readonly wasmyearrange_set_min: (a: number, b: number) => void;
    readonly wasmyearrange_set_years: (a: number, b: number, c: number) => void;
    readonly wasmyearrange_years: (a: number) => [number, number];
    readonly xbergengine_extract: (a: number, b: any, c: any) => any;
    readonly xbergengine_ner: (a: number, b: number, c: number, d: any) => any;
    readonly xbergengine_new: (a: any, b: any) => [number, number, number];
    readonly xbergengine_ocr: (a: number, b: number, c: number, d: any) => any;
    readonly wasmocrboundinggeometry_new: () => number;
    readonly wasmrevisionanchor_new: () => number;
    readonly wasmrerankermodeltype_new: () => number;
    readonly wasmauthconfig_new: () => number;
    readonly wasmdiffline_new: () => number;
    readonly wasmimageoutputformat_new: () => number;
    readonly wasmembeddingmodeltype_new: () => number;
    readonly wasmannotationkind_new: () => number;
    readonly wasmchunksizing_new: () => number;
    readonly wasmqrboundingbox_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmdbffieldinfo_default: () => number;
    readonly wasmerrormetadata_default: () => number;
    readonly wasmprocessingwarning_default: () => number;
    readonly wasmsupportedformat_default: () => number;
    readonly wasmformatmetadata_new: () => number;
    readonly wasmocrstrategy_new: () => number;
    readonly wasmvlmfallbackpolicy_new: () => number;
    readonly wasmredactionterm_labeled: (a: number, b: number, c: number, d: number) => number;
    readonly wasmnodecontent_new: () => number;
    readonly wasmtextannotation_new: (a: number, b: number, c: any) => number;
    readonly wasmlateinteractionmodeltype_new: () => number;
    readonly wasmsparseembeddingmodeltype_default: () => number;
    readonly wasmsparseembeddingmodeltype_new: () => number;
    readonly wasmdjotlink_new: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly wasmdjotlink_default: () => number;
    readonly wasmdbffieldinfo_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmerrormetadata_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmsupportedformat_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmredactionterm_new: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly wasmredactionterm_default: () => number;
    readonly wasmprocessingwarning_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmpagehierarchy_default: () => number;
    readonly wasmqrboundingbox_default: () => number;
    readonly __wbg_wasmvlmfallbackpolicy_free: (a: number, b: number) => void;
    readonly __wbg_wasmsparseembeddingconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmtablecell_free: (a: number, b: number) => void;
    readonly wasmvlmfallbackpolicy_default: () => number;
    readonly wasmproxyconfig_url: (a: number) => [number, number];
    readonly wasmsupportedformat_extension: (a: number) => [number, number];
    readonly wasmredactionfinding_replacementToken: (a: number) => [number, number];
    readonly wasmerrormetadata_set_errorType: (a: number, b: number, c: number) => void;
    readonly wasmredactionpattern_label: (a: number) => [number, number];
    readonly wasmimageoutputformat_type: (a: number) => [number, number];
    readonly wasmprocessingwarning_source: (a: number) => [number, number];
    readonly wasmproxyconfig_set_url: (a: number, b: number, c: number) => void;
    readonly wasmsupportedformat_set_extension: (a: number, b: number, c: number) => void;
    readonly wasmredactionfinding_set_start: (a: number, b: number) => void;
    readonly wasmredactionfinding_set_replacementToken: (a: number, b: number, c: number) => void;
    readonly wasmredactionfinding_set_end: (a: number, b: number) => void;
    readonly wasmdocumentrelationship_source: (a: number) => number;
    readonly wasmpreprocessingoptions_removeNavigation: (a: number) => number;
    readonly wasmpreprocessingoptions_removeForms: (a: number) => number;
    readonly wasmredactionpattern_set_label: (a: number, b: number, c: number) => void;
    readonly wasmimageoutputformat_set_type: (a: number, b: number, c: number) => void;
    readonly wasmprocessingwarning_set_source: (a: number, b: number, c: number) => void;
    readonly wasmtokenreductionoptions_mode: (a: number) => [number, number];
    readonly wasmdocumentrelationship_set_target: (a: number, b: number) => void;
    readonly wasmdocumentrelationship_set_source: (a: number, b: number) => void;
    readonly wasmpreprocessingoptions_set_removeNavigation: (a: number, b: number) => void;
    readonly wasmpreprocessingoptions_set_removeForms: (a: number, b: number) => void;
    readonly wasmpreprocessingoptions_set_enabled: (a: number, b: number) => void;
    readonly wasmpatternmatch_text: (a: number) => [number, number];
    readonly wasmtokenreductionoptions_set_mode: (a: number, b: number, c: number) => void;
    readonly wasmpatternmatch_set_start: (a: number, b: number) => void;
    readonly wasmpatternmatch_set_text: (a: number, b: number, c: number) => void;
    readonly wasmerrormetadata_errorType: (a: number) => [number, number];
    readonly wasmdiffline_set_kind: (a: number, b: number, c: number) => void;
    readonly wasmdiffline_set_0: (a: number, b: number, c: number) => void;
    readonly wasmdiffline_kind: (a: number) => [number, number];
    readonly wasmdbffieldinfo_name: (a: number) => [number, number];
    readonly __wbg_wasmredactionfinding_free: (a: number, b: number) => void;
    readonly wasmrerankerconfig_set_topK: (a: number, b: number) => void;
    readonly wasmrerankerconfig_set_showDownloadProgress: (a: number, b: number) => void;
    readonly wasmrerankerconfig_set_model: (a: number, b: any) => void;
    readonly wasmrerankerconfig_set_maxRerankDurationSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmrerankerconfig_set_cacheDir: (a: number, b: number, c: number) => void;
    readonly wasmrerankerconfig_set_batchSize: (a: number, b: number) => void;
    readonly wasmrerankerconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmrerankerconfig_topK: (a: number) => number;
    readonly wasmrerankerconfig_showDownloadProgress: (a: number) => number;
    readonly wasmrerankerconfig_model: (a: number) => any;
    readonly wasmrerankerconfig_maxRerankDurationSecs: (a: number) => [number, bigint];
    readonly wasmrerankerconfig_cacheDir: (a: number) => [number, number];
    readonly wasmrerankerconfig_batchSize: (a: number) => number;
    readonly wasmrerankerconfig_acceleration: (a: number) => number;
    readonly wasmdiffline_0: (a: number) => [number, number];
    readonly wasmsupportedformat_mimeType: (a: number) => [number, number];
    readonly wasmsupportedformat_set_mimeType: (a: number, b: number, c: number) => void;
    readonly __wbg_wasmdjotlink_free: (a: number, b: number) => void;
    readonly wasmredactionfinding_start: (a: number) => number;
    readonly wasmredactionfinding_end: (a: number) => number;
    readonly wasmqrboundingbox_set_y: (a: number, b: number) => void;
    readonly wasmqrboundingbox_set_x: (a: number, b: number) => void;
    readonly wasmqrboundingbox_set_width: (a: number, b: number) => void;
    readonly wasmqrboundingbox_set_height: (a: number, b: number) => void;
    readonly wasmqrboundingbox_y: (a: number) => number;
    readonly wasmqrboundingbox_x: (a: number) => number;
    readonly wasmqrboundingbox_width: (a: number) => number;
    readonly wasmqrboundingbox_height: (a: number) => number;
    readonly __wbg_wasmqrboundingbox_free: (a: number, b: number) => void;
    readonly wasmproxyconfig_set_password: (a: number, b: number, c: number) => void;
    readonly wasmproxyconfig_set_username: (a: number, b: number, c: number) => void;
    readonly wasmproxyconfig_password: (a: number) => [number, number];
    readonly wasmproxyconfig_username: (a: number) => [number, number];
    readonly __wbg_wasmproxyconfig_free: (a: number, b: number) => void;
    readonly wasmssrfpolicy_set_denyPrivate: (a: number, b: number) => void;
    readonly wasmssrfpolicy_denyPrivate: (a: number) => number;
    readonly wasmvlmfallbackpolicy_set_mode: (a: number, b: number, c: number) => void;
    readonly wasmvlmfallbackpolicy_set_qualityThreshold: (a: number, b: number, c: number) => void;
    readonly wasmvlmfallbackpolicy_mode: (a: number) => [number, number];
    readonly wasmvlmfallbackpolicy_qualityThreshold: (a: number) => [number, number];
    readonly __wbg_wasmtokenreductionoptions_free: (a: number, b: number) => void;
    readonly wasmerrormetadata_set_message: (a: number, b: number, c: number) => void;
    readonly wasmerrormetadata_message: (a: number) => [number, number];
    readonly __wbg_wasmerrormetadata_free: (a: number, b: number) => void;
    readonly wasmpropertychange_set_name: (a: number, b: number, c: number) => void;
    readonly wasmpropertychange_set_from: (a: number, b: number, c: number) => void;
    readonly wasmpropertychange_set_to: (a: number, b: number, c: number) => void;
    readonly wasmpropertychange_name: (a: number) => [number, number];
    readonly wasmpropertychange_from: (a: number) => [number, number];
    readonly wasmpropertychange_to: (a: number) => [number, number];
    readonly wasmdbffieldinfo_set_name: (a: number, b: number, c: number) => void;
    readonly wasmdbffieldinfo_set_fieldType: (a: number, b: number, c: number) => void;
    readonly wasmdbffieldinfo_fieldType: (a: number) => [number, number];
    readonly wasmsparseembeddingconfig_set_model: (a: number, b: any) => void;
    readonly wasmsparseembeddingconfig_set_maxLength: (a: number, b: number) => void;
    readonly wasmsparseembeddingconfig_set_batchSize: (a: number, b: number) => void;
    readonly wasmsparseembeddingconfig_set_acceleration: (a: number, b: number) => void;
    readonly wasmsparseembeddingconfig_model: (a: number) => any;
    readonly wasmsparseembeddingconfig_acceleration: (a: number) => number;
    readonly wasmredactionterm_set_value: (a: number, b: number, c: number) => void;
    readonly wasmredactionterm_set_label: (a: number, b: number, c: number) => void;
    readonly wasmredactionterm_set_caseSensitive: (a: number, b: number) => void;
    readonly wasmredactionterm_value: (a: number) => [number, number];
    readonly wasmredactionterm_label: (a: number) => [number, number];
    readonly wasmredactionterm_caseSensitive: (a: number) => number;
    readonly __wbg_wasmredactionterm_free: (a: number, b: number) => void;
    readonly wasmsparseembeddingconfig_set_maxEmbedDurationSecs: (a: number, b: number, c: bigint) => void;
    readonly wasmsparseembeddingconfig_set_cacheDir: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingconfig_maxLength: (a: number) => number;
    readonly wasmsparseembeddingconfig_maxEmbedDurationSecs: (a: number) => [number, bigint];
    readonly wasmsparseembeddingconfig_cacheDir: (a: number) => [number, number];
    readonly wasmsparseembeddingconfig_batchSize: (a: number) => number;
    readonly wasmocrelementconfig_minConfidence: (a: number) => number;
    readonly wasmocrelementconfig_includeElements: (a: number) => number;
    readonly __wbg_wasmocrelementconfig_free: (a: number, b: number) => void;
    readonly wasmocrelementconfig_set_minConfidence: (a: number, b: number) => void;
    readonly wasmocrelementconfig_set_includeElements: (a: number, b: number) => void;
    readonly wasmdjotlink_set_url: (a: number, b: number, c: number) => void;
    readonly wasmdjotlink_set_title: (a: number, b: number, c: number) => void;
    readonly wasmdjotlink_set_text: (a: number, b: number, c: number) => void;
    readonly wasmdjotlink_url: (a: number) => [number, number];
    readonly wasmdjotlink_title: (a: number) => [number, number];
    readonly wasmdjotlink_text: (a: number) => [number, number];
    readonly __wbg_wasmpageboundary_free: (a: number, b: number) => void;
    readonly wasmdocumentrelationship_target: (a: number) => number;
    readonly wasmredactionpattern_pattern: (a: number) => [number, number];
    readonly __wbg_wasmredactionpattern_free: (a: number, b: number) => void;
    readonly wasmredactionpattern_set_pattern: (a: number, b: number, c: number) => void;
    readonly wasmprocessingwarning_message: (a: number) => [number, number];
    readonly __wbg_wasmprocessingwarning_free: (a: number, b: number) => void;
    readonly wasmprocessingwarning_set_message: (a: number, b: number, c: number) => void;
    readonly wasmpatternmatch_start: (a: number) => number;
    readonly __wbg_wasmdbffieldinfo_free: (a: number, b: number) => void;
    readonly wasmtablecell_set_rowSpan: (a: number, b: number) => void;
    readonly wasmtablecell_set_isHeader: (a: number, b: number) => void;
    readonly wasmtablecell_set_content: (a: number, b: number, c: number) => void;
    readonly wasmtablecell_set_colSpan: (a: number, b: number) => void;
    readonly wasmtablecell_rowSpan: (a: number) => number;
    readonly wasmtablecell_isHeader: (a: number) => number;
    readonly wasmtablecell_content: (a: number) => [number, number];
    readonly wasmtablecell_colSpan: (a: number) => number;
    readonly wasmpageconfig_set_markerFormat: (a: number, b: number, c: number) => void;
    readonly wasmtokenreductionoptions_set_preserveImportantWords: (a: number, b: number) => void;
    readonly wasmpageconfig_markerFormat: (a: number) => [number, number];
    readonly wasmtokenreductionoptions_preserveImportantWords: (a: number) => number;
    readonly __wbg_wasmpageconfig_free: (a: number, b: number) => void;
    readonly __wbg_wasmpreprocessingoptions_free: (a: number, b: number) => void;
    readonly wasmpreprocessingoptions_enabled: (a: number) => number;
    readonly wasmocrrotation_set_confidence: (a: number, b: number, c: number) => void;
    readonly wasmocrrotation_set_angleDegrees: (a: number, b: number) => void;
    readonly wasmocrrotation_confidence: (a: number) => [number, number];
    readonly wasmocrrotation_angleDegrees: (a: number) => number;
    readonly __wbg_wasmocrrotation_free: (a: number, b: number) => void;
    readonly wasmpageboundary_set_pageNumber: (a: number, b: number) => void;
    readonly wasmpageboundary_set_byteStart: (a: number, b: number) => void;
    readonly wasmpageboundary_set_byteEnd: (a: number, b: number) => void;
    readonly wasmpageboundary_pageNumber: (a: number) => number;
    readonly wasmpageboundary_byteStart: (a: number) => number;
    readonly wasmpageboundary_byteEnd: (a: number) => number;
    readonly __wbg_wasmdocumentrelationship_free: (a: number, b: number) => void;
    readonly __wbg_wasmsparseembeddingmodeltype_free: (a: number, b: number) => void;
    readonly wasmsparseembeddingmodeltype_set_type: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingmodeltype_set_name: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingmodeltype_set_modelId: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingmodeltype_set_modelFile: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingmodeltype_set_maxLength: (a: number, b: number, c: bigint) => void;
    readonly wasmsparseembeddingmodeltype_set_additionalFiles: (a: number, b: number, c: number) => void;
    readonly wasmsparseembeddingmodeltype_type: (a: number) => [number, number];
    readonly wasmsparseembeddingmodeltype_name: (a: number) => [number, number];
    readonly wasmsparseembeddingmodeltype_modelId: (a: number) => [number, number];
    readonly wasmsparseembeddingmodeltype_modelFile: (a: number) => [number, number];
    readonly wasmsparseembeddingmodeltype_maxLength: (a: number) => [number, bigint];
    readonly wasmsparseembeddingmodeltype_additionalFiles: (a: number) => [number, number];
    readonly compress: (a: number, b: number, c: number, d: number) => [number, number, number];
    readonly decompress: (a: any, b: number, c: number, d: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__hc53a447a5735fa3a: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h2fc03d71408c92bf: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h93700a3b505b0d05: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hff3dd44bad76f477: (a: number, b: number) => void;
    readonly __wbindgen_malloc_command_export: (a: number, b: number) => number;
    readonly __wbindgen_realloc_command_export: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store_command_export: (a: number) => void;
    readonly __externref_table_alloc_command_export: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_destroy_closure_command_export: (a: number, b: number) => void;
    readonly __externref_table_dealloc_command_export: (a: number) => void;
    readonly __externref_drop_slice_command_export: (a: number, b: number) => void;
    readonly __wbindgen_free_command_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
