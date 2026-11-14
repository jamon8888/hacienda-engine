import type { PiiEntity } from "./pii-engine";
import type { SensitiveMarkAttrs, SensitiveMode } from "./sensitive-mark";
export type { SensitiveMarkAttrs, SensitiveMarkSource, SensitiveMode } from "./sensitive-mark";

export interface TokenizedEditorTextNode {
  type: "text";
  text: string;
  marks?: Array<{ type: "haciendaSensitive"; attrs: SensitiveMarkAttrs }>;
}

export interface TokenizedEditorBlock {
  type: "paragraph" | "heading";
  attrs?: Record<string, unknown>;
  content?: TokenizedEditorTextNode[];
}

export interface TokenizedEditorDocument {
  type: "doc";
  content: TokenizedEditorBlock[];
}

export interface TokenizedMetadata {
  title: string;
  filename: string;
  authors: string[];
  subject: string;
  keywords: string[];
}

export interface TokenizedEditorEnvelope {
  schema_version: "hacienda-editor-v1";
  document_id: string;
  version_id: string;
  source: {
    mime_type: string;
    original_ref: string;
    viewer_ref: string;
    capability_ref: string;
  };
  metadata_token_only: TokenizedMetadata;
  editor: TokenizedEditorDocument;
  search_projection: {
    text_token_only: string;
    chunk_ids: string[];
    embedding_state: "pending" | "ready";
  };
  provenance: {
    redaction_policy_version: string;
    editor_schema_version: "hacienda-editor-v1";
    created_by: string;
    audit_event_id: string;
  };
}

export interface TokenIssueContext {
  value: string;
  tokenClass: string;
  mode: SensitiveMode;
}

export type TokenIssuer = (context: TokenIssueContext) => string;

function defaultOpaqueId(): string {
  const cryptoObject = (globalThis as { crypto?: Crypto }).crypto;
  if (cryptoObject?.randomUUID)
    return cryptoObject.randomUUID().replaceAll("-", "");
  return `local-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}

export function tokenText(tokenClass: string, tokenId: string): string {
  return `[${tokenClass.toUpperCase()}:${tokenId}]`;
}

export function redactMetadata(
  metadata: {
    title: string;
    filename: string;
    authors?: string[];
    subject?: string;
    keywords?: string[];
  },
  issueToken: TokenIssuer = ({ tokenClass }) => defaultOpaqueId(),
): TokenizedMetadata {
  const issue = (value: string, tokenClass: string) =>
    tokenText(
      tokenClass,
      issueToken({ value, tokenClass, mode: "pseudonymize" }),
    );
  return {
    title: issue(metadata.title, "title"),
    filename: issue(metadata.filename, "filename"),
    authors: (metadata.authors ?? []).map((value) => issue(value, "person")),
    subject: issue(metadata.subject ?? "", "matter"),
    keywords: (metadata.keywords ?? []).map((value) => issue(value, "keyword")),
  };
}

function sensitiveMark(category: string, tokenId: string, mode: SensitiveMode) {
  return {
    type: "haciendaSensitive" as const,
    attrs: {
      tokenClass: category.toLowerCase(),
      tokenId,
      mode,
      source: "manual" as const,
      disclosure: "viewer-only" as const,
    },
  };
}

function cloneDocument(
  document: TokenizedEditorDocument,
): TokenizedEditorDocument {
  return structuredClone(document);
}

function textNodes(document: TokenizedEditorDocument) {
  const result: Array<{
    node: TokenizedEditorTextNode;
    block: TokenizedEditorBlock;
    index: number;
    start: number;
  }> = [];
  let offset = 0;
  for (const block of document.content) {
    for (const [index, node] of (block.content ?? []).entries()) {
      result.push({ node, block, index, start: offset });
      offset += node.text.length;
    }
  }
  return result;
}

/** Returns only the text that the tokenized editor is allowed to persist/search. */
export function textFromTokenizedDocument(
  document: TokenizedEditorDocument,
): string {
  return document.content
    .map((block) => (block.content ?? []).map((node) => node.text).join(""))
    .join("\n");
}

export function applyManualRedaction({
  document,
  from,
  to,
  category,
  mode,
  issueToken,
}: {
  document: TokenizedEditorDocument;
  /** Offsets into textFromTokenizedDocument, not ProseMirror node positions. */
  from: number;
  to: number;
  category: string;
  mode: SensitiveMode;
  issueToken?: TokenIssuer;
}): TokenizedEditorDocument {
  if (from < 0 || to <= from)
    throw new Error("A non-empty selection is required");
  const next = cloneDocument(document);
  const nodes = textNodes(next);
  const first = nodes.find(
    ({ start, node }) => from >= start && from < start + node.text.length,
  );
  const last = nodes.find(
    ({ start, node }) => to > start && to <= start + node.text.length,
  );
  if (!first || !last)
    throw new Error("Selection is outside the tokenized document");

  const selectedText = textFromTokenizedDocument(document).slice(from, to);
  const tokenId =
    mode === "pseudonymize"
      ? (issueToken ?? (({ tokenClass }) => defaultOpaqueId()))({
          value: selectedText,
          tokenClass: category,
          mode,
        })
      : "manual";
  const replacement =
    mode === "delete"
      ? ""
      : mode === "manual-mask"
        ? `[${category.toUpperCase()}]`
        : tokenText(category, tokenId);
  const replacementNode: TokenizedEditorTextNode = {
    type: "text",
    text: replacement,
    ...(mode === "delete"
      ? {}
      : { marks: [sensitiveMark(category, tokenId, mode)] }),
  };

  if (first.block === last.block && first.index === last.index) {
    const localFrom = from - first.start;
    const localTo = to - first.start;
    const original = first.node;
    const nodesForBlock = first.block.content ?? [];
    const replacementNodes: TokenizedEditorTextNode[] = [];
    if (localFrom > 0)
      replacementNodes.push({
        type: "text",
        text: original.text.slice(0, localFrom),
      });
    if (replacement) replacementNodes.push(replacementNode);
    if (localTo < original.text.length)
      replacementNodes.push({
        type: "text",
        text: original.text.slice(localTo),
      });
    nodesForBlock.splice(first.index, 1, ...replacementNodes);
    return next;
  }

  // A cross-node selection is uncommon in the toolbar but must remain safe: preserve
  // only the unselected prefix/suffix and put one sensitive token at the first node.
  const firstLocal = from - first.start;
  const lastLocal = to - last.start;
  first.node.text = first.node.text.slice(0, firstLocal) + replacement;
  first.node.marks = replacementNode.marks;
  last.node.text = last.node.text.slice(lastLocal);
  for (const entry of nodes) {
    if (entry.start > first.start && entry.start < last.start)
      entry.node.text = "";
  }
  for (const block of next.content)
    block.content = (block.content ?? []).filter(
      (node) => node.text.length > 0,
    );
  return next;
}

function tokenizedParagraphs(
  markdown: string,
  findings: PiiEntity[],
): TokenizedEditorDocument {
  const valid = findings
    .filter(
      (finding) =>
        finding.start >= 0 &&
        finding.end > finding.start &&
        finding.end <= markdown.length,
    )
    .sort((a, b) => a.start - b.start);
  const lines = markdown.split("\n");
  const content: TokenizedEditorBlock[] = [];
  let documentOffset = 0;
  for (const line of lines) {
    const lineEnd = documentOffset + line.length;
    const lineFindings = valid.filter(
      (finding) => finding.start < lineEnd && finding.end > documentOffset,
    );
    const lineContent: TokenizedEditorTextNode[] = [];
    let cursor = 0;
    for (const finding of lineFindings) {
      const start = Math.max(finding.start - documentOffset, 0);
      const end = Math.min(finding.end - documentOffset, line.length);
      if (start < cursor) continue;
      if (start > cursor)
        lineContent.push({ type: "text", text: line.slice(cursor, start) });
      const replacement =
        finding.redact_template || `[${finding.category.toUpperCase()}]`;
      lineContent.push({
        type: "text",
        text: replacement,
        marks: [
          {
            type: "haciendaSensitive",
            attrs: {
              tokenClass: finding.category.toLowerCase(),
              tokenId: replacement.slice(1, -1),
              mode: "pseudonymize",
              source: finding.source === "manual" ? "manual" : "gliner",
              disclosure: "viewer-only",
            },
          },
        ],
      });
      cursor = end;
    }
    if (cursor < line.length)
      lineContent.push({ type: "text", text: line.slice(cursor) });
    content.push({ type: "paragraph", content: lineContent });
    documentOffset = lineEnd + 1;
  }
  return { type: "doc", content };
}

export function buildTokenizedEditorEnvelope({
  documentId,
  versionId,
  mimeType,
  originalRef,
  viewerRef,
  metadata,
  markdown,
  findings,
  createdBy,
  auditEventId,
  issueToken,
}: {
  documentId: string;
  versionId: string;
  mimeType: string;
  originalRef: string;
  viewerRef: string;
  capabilityRef: string;
  metadata: Parameters<typeof redactMetadata>[0];
  markdown: string;
  findings: PiiEntity[];
  createdBy: string;
  auditEventId: string;
  issueToken?: TokenIssuer;
}): TokenizedEditorEnvelope {
  const editor = tokenizedParagraphs(markdown, findings);
  return {
    schema_version: "hacienda-editor-v1",
    document_id: documentId,
    version_id: versionId,
    source: {
      mime_type: mimeType,
      original_ref: originalRef,
      viewer_ref: viewerRef,
      capability_ref: capabilityRef,
    },
    metadata_token_only: redactMetadata(metadata, issueToken),
    editor,
    search_projection: {
      text_token_only: textFromTokenizedDocument(editor),
      chunk_ids: [],
      embedding_state: "pending",
    },
    provenance: {
      redaction_policy_version: "pii-policy-v1",
      editor_schema_version: "hacienda-editor-v1",
      created_by: createdBy,
      audit_event_id: auditEventId,
    },
  };
}
