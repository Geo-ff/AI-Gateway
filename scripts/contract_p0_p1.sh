#!/usr/bin/env bash
# OpenAPI contract tests (schema-based) via Schemathesis
# - Creates an isolated venv in scripts/_contract/.venv (no global pollution)
# - Produces redacted md+log reports under scripts/_contract/

set -euo pipefail
IFS=$'\n\t'

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PYTHON_BIN="${PYTHON_BIN:-python3}"
VENV_DIR="scripts/_contract/.venv"
REQ_FILE="scripts/_contract/requirements.txt"
RUNNER="scripts/_contract/run_contract.py"

mkdir -p scripts/_contract

if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
  "$PYTHON_BIN" -m venv "$VENV_DIR"
fi

"${VENV_DIR}/bin/python" -m pip -q install --upgrade pip >/dev/null
"${VENV_DIR}/bin/pip" -q install -r "$REQ_FILE" >/dev/null

exec "${VENV_DIR}/bin/python" "$RUNNER"

