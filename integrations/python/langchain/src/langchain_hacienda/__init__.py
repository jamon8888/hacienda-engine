"""LangChain document loader and RAG integration for hacienda-engine."""

from langchain_hacienda.loader import HaciendaLoader, HaciendaLoaderError
from langchain_hacienda.rag import HaciendaRetriever, push_documents

__version__ = "0.1.0"

__all__ = [
    "HaciendaLoader",
    "HaciendaLoaderError",
    "HaciendaRetriever",
    "push_documents",
    "__version__",
]
