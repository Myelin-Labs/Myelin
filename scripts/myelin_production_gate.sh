#!/usr/bin/env bash
# Single validation entry point; orchestration and evidence live in validation.py.
set -euo pipefail
exec python3 "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/validation.py" "$@"
