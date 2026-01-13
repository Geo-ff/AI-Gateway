#!/usr/bin/env bash
# RBAC boundary business tests (automated)
# - Creates an isolated venv in scripts/_rbac/.venv (no global pollution)
# - Produces redacted md+log reports under scripts/_rbac/ and appends workflow_follow.md

set -euo pipefail
IFS=$'\n\t'

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PYTHON_BIN="${PYTHON_BIN:-python3}"
VENV_DIR="scripts/_rbac/.venv"
REQ_FILE="scripts/_rbac/requirements.txt"
RUNNER="scripts/_rbac/run_rbac.py"

mkdir -p scripts/_rbac

if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
  "$PYTHON_BIN" -m venv "$VENV_DIR"
fi

"${VENV_DIR}/bin/python" -m pip -q install --upgrade pip >/dev/null
"${VENV_DIR}/bin/pip" -q install -r "$REQ_FILE" >/dev/null

exec "${VENV_DIR}/bin/python" "$RUNNER"

