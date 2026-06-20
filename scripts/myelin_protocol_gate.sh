#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

printf 'myelin_protocol_gate.sh is kept as a compatibility wrapper.\n'
printf 'Delegating to scripts/myelin_production_gate.sh, the single release gate.\n\n'

exec "${SCRIPT_DIR}/myelin_production_gate.sh" "$@"
