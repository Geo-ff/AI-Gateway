#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:8080}"
CLIENT_TOKEN="${CLIENT_TOKEN:?CLIENT_TOKEN is required (Bearer token for /v1/*)}"

auth_header="Authorization: Bearer ${CLIENT_TOKEN}"

echo "[1/3] Fetch models..."
models_json="$(curl -fsS "${BASE_URL}/v1/models" -H "${auth_header}")"

if command -v jq >/dev/null 2>&1; then
  model_id="$(
    echo "${models_json}" \
      | jq -r '.data[].id' \
      | (grep -Ei "gpt5\\.1|haiku|sonnet" || true) \
      | head -n 1 \
      | sed 's/^\\s*//;s/\\s*$//'
  )"
  if [ -z "${model_id}" ]; then
    model_id="$(echo "${models_json}" | jq -r '.data[0].id')"
  fi
else
  echo "warning: jq not found; fallback to default model name 'gpt5.1'"
  model_id="gpt5.1"
fi

echo "Using model: ${model_id}"

payload="$(cat <<JSON
{
  "model": "${model_id}",
  "messages": [{"role":"user","content":"ping"}],
  "stream": false
}
JSON
)"

echo "[2/3] Send non-stream requests..."
for i in 1 2 3; do
  curl -fsS "${BASE_URL}/v1/chat/completions" \
    -H "${auth_header}" \
    -H "Content-Type: application/json" \
    -d "${payload}" >/dev/null || true
done

echo "[3/3] Send one stream request..."
stream_payload="$(cat <<JSON
{
  "model": "${model_id}",
  "messages": [{"role":"user","content":"ping stream"}],
  "stream": true
}
JSON
)"
curl -fsS "${BASE_URL}/v1/chat/completions" \
  -H "${auth_header}" \
  -H "Content-Type: application/json" \
  -d "${stream_payload}" >/dev/null || true

echo "Done. Verify via: ${BASE_URL}/admin/logs/requests"
