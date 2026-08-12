"""hacienda-engine document loader for LangChain."""

from __future__ import annotations

import base64
import mimetypes
import os
from pathlib import Path
from typing import TYPE_CHECKING, Any
from uuid import UUID

from hacienda_sdk import HaciendaApiError, HaciendaClient

# `ProcessDocumentsRequest`/`DocumentInput` are not re-exported from `hacienda_sdk`'s
# public `__init__.py` (only `HaciendaClient`, `AsyncHaciendaClient`, `HaciendaApiError`
# are) — this mirrors exactly how `hacienda_sdk.client`, the SDK's own hand-written
# module, constructs request bodies, since there is currently no other way to build one.
# See sdks/python/src/hacienda_sdk/client.py's imports for the same pattern.
from hacienda_sdk._generated.models.document_input import DocumentInput
from hacienda_sdk._generated.models.process_documents_request import ProcessDocumentsRequest
from langchain_core.document_loaders import BaseLoader
from langchain_core.documents import Document

if TYPE_CHECKING:
    from collections.abc import Iterator

    from hacienda_sdk._generated.models.document_result import DocumentResult


class HaciendaLoaderError(RuntimeError):
    """Raised for loader-level failures (bad input, unresolvable MIME type)."""


def _guess_mime_type(path: Path) -> str | None:
    mime_type, _ = mimetypes.guess_type(path.name)
    return mime_type


class HaciendaLoader(BaseLoader):
    """Load documents through hacienda-engine's redacting, audited extraction pipeline.

    Unlike a loader that extracts raw text, every ``Document`` this yields has already
    had PII removed per the server's configured redaction policy (`POST /v1/documents`
    — see `hacienda-api/src/handlers/documents.rs`). Detected entities (category, span,
    confidence — never raw text, since this endpoint never sets `include_text`) and the
    batch's audit-chain tip land in ``Document.metadata`` alongside the usual fields, so
    a chunk that later ends up in a vector store can always be traced back to the
    audited extraction that produced it.

    Multiple files are sent as one batched request — concurrency happens server-side,
    the same reasoning `XbergLoader` (xberg's own LangChain loader) gives for batching
    ``extract_batch`` calls Rust-side rather than looping client-side.

    Examples:
        Load a single file:
            >>> loader = HaciendaLoader(file_path="contract.pdf", api_key="...")
            >>> docs = loader.load()

        Load multiple files (one batched request):
            >>> loader = HaciendaLoader(file_path=["a.pdf", "b.docx"], api_key="...")
            >>> docs = loader.load()

        Load from bytes:
            >>> loader = HaciendaLoader(data=raw_bytes, mime_type="application/pdf", api_key="...")
            >>> docs = loader.load()

        Load a directory:
            >>> loader = HaciendaLoader(file_path="./docs/", glob="**/*.pdf", api_key="...")
            >>> docs = loader.load()

        Reuse an existing client (e.g. a non-default base_url, or connection pooling
        across loaders):
            >>> from hacienda_sdk import HaciendaClient
            >>> client = HaciendaClient(base_url="https://hacienda.example.com", api_key="...")
            >>> loader = HaciendaLoader(file_path="contract.pdf", client=client)
    """

    def __init__(
        self,
        *,
        file_path: str | Path | list[str | Path] | None = None,
        data: bytes | None = None,
        mime_type: str | None = None,
        filename: str | None = None,
        glob: str | None = None,
        document_id: str | UUID | None = None,
        client: HaciendaClient | None = None,
        base_url: str | None = None,
        api_key: str | None = None,
    ) -> None:
        """Initialize the HaciendaLoader.

        Args:
            file_path: File path, list of file paths, or directory path to load.
            data: Raw bytes to extract text from. Mutually exclusive with file_path.
            mime_type: MIME type override. Required when using ``data``; inferred from
                the file extension for ``file_path`` when omitted (via
                :mod:`mimetypes`) and raises :class:`HaciendaLoaderError` if it can't be.
            filename: Display filename to send alongside ``data``. Ignored for
                ``file_path`` (the path's own name is used).
            glob: Glob pattern for directory mode. Defaults to ``"**/*"``.
            document_id: Enables server-side versioning for a single-file load (see
                `DocumentInput.document_id`). Only valid with a single ``file_path`` or
                ``data`` input — never with a list or directory, since every document in
                a batch would otherwise collide on one id.
            client: A pre-built :class:`hacienda_sdk.HaciendaClient`. Takes precedence
                over ``base_url``/``api_key`` when given; use this to share one client
                (and its connection pool) across multiple loaders.
            base_url: hacienda-api base URL. Falls back to the ``HACIENDA_BASE_URL``
                env var, then ``http://127.0.0.1:8080``. Ignored if ``client`` is given.
            api_key: hacienda-api bearer token. Falls back to the ``HACIENDA_API_KEY``
                env var. Ignored if ``client`` is given.

        Raises:
            HaciendaLoaderError: If neither/both of file_path and data are given, if
                data is given without mime_type, or if document_id is given for a
                multi-document load.
        """
        if file_path is None and data is None:
            raise HaciendaLoaderError("Either 'file_path' or 'data' must be provided.")
        if file_path is not None and data is not None:
            raise HaciendaLoaderError(
                "Cannot specify both 'file_path' and 'data'. Use one or the other."
            )
        if data is not None and mime_type is None:
            raise HaciendaLoaderError("'mime_type' is required when using 'data'.")

        if isinstance(file_path, (str, Path)):
            self._file_path: Path | list[Path] | None = Path(file_path)
        elif file_path is not None:
            self._file_path = [Path(p) for p in file_path]
        else:
            self._file_path = None

        self._data = data
        self._mime_type = mime_type
        self._filename = filename
        self._glob = glob
        self._document_id = UUID(str(document_id)) if document_id is not None else None

        if self._document_id is not None and self._is_batch():
            raise HaciendaLoaderError(
                "'document_id' is only valid for a single-file or bytes load, not a "
                "list of paths or a directory — every document in a batch would "
                "otherwise collide on the same id."
            )

        if client is not None:
            self._client = client
            self._owns_client = False
        else:
            resolved_base_url = base_url or os.environ.get(
                "HACIENDA_BASE_URL", "http://127.0.0.1:8080"
            )
            resolved_api_key = api_key or os.environ.get("HACIENDA_API_KEY")
            if not resolved_api_key:
                raise HaciendaLoaderError(
                    "No API key: pass api_key=..., set HACIENDA_API_KEY, or pass an "
                    "existing client=HaciendaClient(...)."
                )
            self._client = HaciendaClient(base_url=resolved_base_url, api_key=resolved_api_key)
            self._owns_client = True

    def _is_batch(self) -> bool:
        if self._data is not None:
            return False
        file_path = self._file_path
        if isinstance(file_path, list):
            return True
        return isinstance(file_path, Path) and file_path.is_dir()

    def _resolve_paths(self) -> Iterator[Path]:
        file_path = self._file_path
        if isinstance(file_path, list):
            yield from file_path
        elif isinstance(file_path, Path):
            if file_path.is_dir():
                pattern = self._glob or "**/*"
                yield from sorted(p for p in file_path.glob(pattern) if p.is_file())
            else:
                yield file_path

    def _build_request(self) -> tuple[ProcessDocumentsRequest, list[str]]:
        """Build the batched request body and the human-readable source per input."""
        if self._data is not None:
            source = f"bytes://{self._mime_type}"
            doc_input = DocumentInput(
                content_base64=base64.b64encode(self._data).decode("ascii"),
                mime_type=self._mime_type,  # type: ignore[arg-type]  # checked non-None in __init__
                filename=self._filename,
                document_id=self._document_id,
            )
            return ProcessDocumentsRequest(documents=[doc_input]), [source]

        paths = list(self._resolve_paths())
        documents: list[DocumentInput] = []
        sources: list[str] = []
        for path in paths:
            mime_type = self._mime_type or _guess_mime_type(path)
            if mime_type is None:
                raise HaciendaLoaderError(
                    f"Could not infer a MIME type for '{path}'. Pass mime_type=... explicitly."
                )
            content = path.read_bytes()
            documents.append(
                DocumentInput(
                    content_base64=base64.b64encode(content).decode("ascii"),
                    mime_type=mime_type,
                    filename=path.name,
                    document_id=self._document_id,
                )
            )
            sources.append(str(path))

        return ProcessDocumentsRequest(documents=documents), sources

    def _result_to_document(
        self,
        result: DocumentResult,
        source: str,
        *,
        processing_time_ms: int,
        audit_chain_tip: str | None,
    ) -> Document:
        categories = sorted({entity.category for entity in result.entities})
        metadata: dict[str, Any] = {
            "source": source,
            "pii_entities_found": len(result.entities),
            "pii_categories": categories,
            "pii_entities": [
                {
                    "category": entity.category,
                    "start": entity.start,
                    "end": entity.end,
                    "confidence": entity.confidence,
                    "source": entity.source,
                }
                for entity in result.entities
            ],
            "processing_time_ms": processing_time_ms,
        }
        if audit_chain_tip is not None:
            metadata["audit_chain_tip"] = audit_chain_tip
        # `document_id`/`version_sequence` are `None | Unset | T` — excluding both in one
        # check by testing for the concrete type, rather than `is not None` (which is
        # True for `Unset` too; see `hacienda_sdk._generated.types.Unset`).
        if isinstance(result.document_id, UUID):
            metadata["document_id"] = str(result.document_id)
        if isinstance(result.version_sequence, int):
            metadata["version_sequence"] = result.version_sequence

        return Document(page_content=result.content, metadata=metadata)

    def lazy_load(self) -> Iterator[Document]:
        """Load documents, yielding one already-redacted Document per input.

        Async loading (``aload``/``alazy_load``) is inherited from
        :class:`~langchain_core.document_loaders.BaseLoader`'s default implementation,
        which runs this method in a background thread — there is no separate async
        hacienda-engine call to make here since the request is a single batched
        round-trip either way.

        Yields:
            Document objects with redacted content and PII/audit metadata.

        Raises:
            HaciendaLoaderError: Wraps any :class:`hacienda_sdk.HaciendaApiError`
                raised by the request.
        """
        request, sources = self._build_request()
        if not request.documents:
            return

        try:
            response = self._client.documents.process_documents(request)
        except HaciendaApiError as exc:
            raise HaciendaLoaderError(f"hacienda-engine request failed: {exc}") from exc

        audit_chain_tip = (
            response.audit_chain_tip if isinstance(response.audit_chain_tip, str) else None
        )
        for index, result in enumerate(response.documents):
            source = sources[index] if index < len(sources) else (sources[-1] if sources else "")
            yield self._result_to_document(
                result,
                source,
                processing_time_ms=response.processing_time_ms,
                audit_chain_tip=audit_chain_tip,
            )

    def __del__(self) -> None:
        if getattr(self, "_owns_client", False):
            self._client.close()
