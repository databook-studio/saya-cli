"""Policy loading and validation for the file-limits checker."""

import json
from dataclasses import dataclass
from pathlib import Path

REQUIRED_KEYS = ("limit", "target", "review", "extensions", "exclude_dir_names")


class PolicyError(Exception):
    """Raised when the policy file is missing, unreadable or invalid."""


@dataclass(frozen=True)
class Policy:
    limit: int
    target: int
    review: int
    extensions: tuple[str, ...]
    exclude_dir_names: tuple[str, ...]


def _positive_int(data, key, origin):
    value = data[key]
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise PolicyError(
            f"{origin}: policy key {key!r} must be a positive integer, got {value!r}"
        )
    return value


def _string_list(data, key, origin):
    value = data[key]
    if not isinstance(value, list) or not all(
        isinstance(item, str) and item for item in value
    ):
        raise PolicyError(
            f"{origin}: policy key {key!r} must be a list of non-empty strings,"
            f" got {value!r}"
        )
    return tuple(value)


def _extensions(data, origin):
    values = _string_list(data, "extensions", origin)
    for value in values:
        if len(value) < 2 or not value.startswith("."):
            raise PolicyError(
                f"{origin}: policy key 'extensions' entries must start with '.',"
                f" got {value!r}"
            )
    return values


def load_policy(path) -> Policy:
    """Load and validate a policy JSON file; raises PolicyError on any problem."""
    origin = Path(path)
    try:
        raw = origin.read_bytes()
    except OSError as exc:
        raise PolicyError(f"cannot read policy file {origin}: {exc}") from None
    try:
        data = json.loads(raw)
    except (UnicodeDecodeError, ValueError) as exc:
        raise PolicyError(f"policy file {origin} is not valid JSON: {exc}") from None
    if not isinstance(data, dict):
        raise PolicyError(f"policy file {origin} must contain a JSON object")
    missing = [key for key in REQUIRED_KEYS if key not in data]
    unknown = [key for key in sorted(data) if key not in REQUIRED_KEYS]
    if missing:
        raise PolicyError(f"policy file {origin} is missing keys: {', '.join(missing)}")
    if unknown:
        raise PolicyError(f"policy file {origin} has unknown keys: {', '.join(unknown)}")
    return Policy(
        limit=_positive_int(data, "limit", origin),
        target=_positive_int(data, "target", origin),
        review=_positive_int(data, "review", origin),
        extensions=_extensions(data, origin),
        exclude_dir_names=_string_list(data, "exclude_dir_names", origin),
    )