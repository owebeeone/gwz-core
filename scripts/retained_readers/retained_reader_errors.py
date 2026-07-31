"""Shared retained-reader gate errors."""


class MatrixError(RuntimeError):
    """A retained-reader fixture cannot be executed or evaluated safely."""


class HarnessError(RuntimeError):
    """A required compatibility operation could not be completed."""
