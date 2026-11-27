"""Pinned versions for the shared VPS agent harness.

The harness is written against basemind 0.34.0 and Orca 1.4.263.  basemind is
expected at exactly 0.34.0 (the version whose MCP tool set this harness was
validated against); Orca must be at least 1.4.263.
"""

__version__ = "0.2.0"

BASEMIND_EXACT = (0, 34, 0)
ORCA_MIN = (1, 4, 263)

BASEMIND_EXACT_STR = "0.34.0"
ORCA_MIN_STR = "1.4.263"
