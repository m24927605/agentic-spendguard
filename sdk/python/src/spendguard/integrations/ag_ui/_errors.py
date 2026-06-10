"""AgUiEventValidationError — the single error class raised by the
``spendguard.integrations.ag_ui`` validators, builders, and canonical
serializer.

``field`` carries the payload-key-style (snake_case) name of the
offending field (design.md §8.2; review-standards §5.6). Serializer-level
violations that have no payload key use the sentinel names ``"(value)"``
/ ``"(key)"`` — mirroring ``errors.ts`` in ``@spendguard/ag-ui``.
"""

from __future__ import annotations

__all__ = ["AgUiEventValidationError"]


class AgUiEventValidationError(ValueError):
    """Raised when a builder input or canonical payload violates the
    LOCKED design.md §5/§7 rules.

    Mirrors the TS ``AgUiEventValidationError`` 1:1: ``field`` names the
    offending payload key (snake_case), never an input attribute name.
    """

    field: str

    def __init__(self, field: str, message: str | None = None) -> None:
        super().__init__(message or f'invalid value for field "{field}"')
        self.field = field
