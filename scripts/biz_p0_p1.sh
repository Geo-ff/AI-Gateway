#!/usr/bin/env bash
# Business semantic tests (ClientToken & Providers/keys)
# - Creates an isolated venv in scripts/_biz/.venv (no global pollution)
# - Produces redacted md+log reports under scripts/_biz/

set -euo pipefail
IFS=$'\n\t'

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PYTHON_BIN="${PYTHON_BIN:-python3}"
VENV_DIR="scripts/_biz/.venv"
RUNNER="scripts/_biz/run_biz.py"

mkdir -p scripts/_biz

if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
  "$PYTHON_BIN" -m venv "$VENV_DIR"
fi

"${VENV_DIR}/bin/python" -m pip -q install --upgrade pip >/dev/null

exec "${VENV_DIR}/bin/python" "$RUNNER"

