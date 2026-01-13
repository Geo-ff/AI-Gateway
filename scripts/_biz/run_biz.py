#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import re
import secrets
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any


ROOT_DIR = Path(__file__).resolve().parents[2]

READY_CHECK_URL = "http://localhost:8080/auth/me"

SENSITIVE_JSON_KEYS = {
    "authorization",
    "accesstoken",
    "refreshtoken",
    "password",
    "token",
    "clienttoken",
    "api_key",
    "api_keys",
    "apikey",
    "key",
    "keys",
    "secret",
    "provider_key",
    # `/providers/{provider}/keys/raw` contains plaintext key under `value`
    "value",
}

SENSITIVE_KEY_SUBSTRINGS = ("token", "password", "secret", "key", "authorization")


def utc_now() -> datetime:
    return datetime.now(tz=timezone.utc)


def utc_compact_timestamp(dt: datetime) -> str:
    return dt.strftime("%Y%m%dT%H%M%SZ")


def mask_secret(value: str | None, keep: int = 8) -> str:
    s = value or ""
    n = len(s)
    if n == 0:
        return "(empty)"
    if keep <= 0:
        return f"(len={n})"
    if n <= keep:
        return f"{s[:1]}…(len={n})"
    return f"{s[:keep]}…(len={n})"


def redact_bearer(text: str) -> str:
    text = re.sub(r"(?i)(Authorization:\s*Bearer)\s+([^\s]+)", r"\1 ***REDACTED***", text)
    text = re.sub(r"(?i)\\bBearer\\s+([A-Za-z0-9._+/=\\-]+)\\b", "Bearer ***REDACTED***", text)
    return text


def redact_text(text: str) -> str:
    text = redact_bearer(text)
    text = re.sub(
        r'(?i)"(refreshToken|accessToken|password|token|key|value)"\s*:\s*"[^"]+"',
        lambda m: f"\"{m.group(1)}\": \"***REDACTED***\"",
        text,
    )
    return text


def _is_sensitive_key(key: str) -> bool:
    k = key.lower()
    if k in SENSITIVE_JSON_KEYS:
        return True
    return any(sub in k for sub in SENSITIVE_KEY_SUBSTRINGS)


def redact_json(value: Any) -> Any:
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for k, v in value.items():
            if _is_sensitive_key(str(k)):
                if isinstance(v, str):
                    out[k] = f"***REDACTED*** (len={len(v)})"
                elif v is None:
                    out[k] = None
                else:
                    out[k] = "***REDACTED***"
            else:
                out[k] = redact_json(v)
        return out
    if isinstance(value, list):
        return [redact_json(v) for v in value]
    return value


def norm_env_key(key: str) -> str:
    k = (key or "").strip()
    if k.lower().startswith("export "):
        k = k[7:].strip()
    k = re.sub(r"[^A-Za-z0-9]+", "_", k)
    return k.strip("_").upper()


def parse_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    env: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.rstrip("\r")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        idx_eq = line.find("=")
        idx_col = line.find(":")
        if idx_eq == -1 and idx_col == -1:
            continue
        if idx_eq == -1:
            idx = idx_col
        elif idx_col == -1:
            idx = idx_eq
        else:
            idx = min(idx_eq, idx_col)
        key_raw = line[:idx].strip()
        val = line[idx + 1 :].strip()
        if not key_raw:
            continue
        if (val.startswith('"') and val.endswith('"') and len(val) >= 2) or (
            val.startswith("'") and val.endswith("'") and len(val) >= 2
        ):
            val = val[1:-1]
        env[norm_env_key(key_raw)] = val
    return env


def load_config() -> dict[str, str]:
    env_path = ROOT_DIR / ".env"
    example_path = ROOT_DIR / ".env.example"
    return parse_env_file(env_path) or parse_env_file(example_path)


def pick(cfg: dict[str, str], keys: list[str]) -> str:
    for k in keys:
        v = (cfg.get(norm_env_key(k)) or "").strip()
        if v:
            return v
    return ""


@dataclass(frozen=True)
class CurlResult:
    status_code: int
    body_text: str
    body_json: Any | None


@dataclass(frozen=True)
class CurlRawResult:
    status_code: int
    headers_text: str
    body_text: str


def run_cmd(cmd: list[str], *, timeout_s: int = 30) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout_s)


def curl_http_code(url: str, *, timeout_s: int = 8) -> tuple[int, str]:
    cmd = ["curl", "-sS", "-o", "/dev/null", "-w", "%{http_code}\n", url]
    proc = run_cmd(cmd, timeout_s=timeout_s)
    if proc.returncode != 0:
        return proc.returncode, ""
    return 0, (proc.stdout or "").strip()


def curl_json(
    *,
    base_url: str,
    method: str,
    path: str,
    bearer: str | None = None,
    json_body: dict[str, Any] | None = None,
    timeout_s: int = 30,
) -> CurlResult:
    url = f"{base_url}{path}"
    tmp_file = tempfile.NamedTemporaryFile(prefix="gw_biz_", suffix=".json", delete=False)
    tmp_path = tmp_file.name
    tmp_file.close()
    try:
        cmd: list[str] = [
            "curl",
            "-sS",
            "-o",
            tmp_path,
            "-w",
            "%{http_code}",
            "-X",
            method,
            url,
            "-H",
            "Accept: application/json",
        ]
        if bearer:
            cmd += ["-H", f"Authorization: Bearer {bearer}"]
        if json_body is not None:
            cmd += ["-H", "Content-Type: application/json", "--data", json.dumps(json_body)]

        proc = run_cmd(cmd, timeout_s=timeout_s)
        body_text = Path(tmp_path).read_text(encoding="utf-8", errors="replace")
        if proc.returncode != 0:
            stderr = redact_text(proc.stderr.strip())
            raise RuntimeError(f"curl failed ({method} {path}): {stderr}")
        status_code = int((proc.stdout or "").strip() or "0")
        body_json: Any | None = None
        try:
            if body_text.strip():
                body_json = json.loads(body_text)
        except Exception:
            body_json = None
        return CurlResult(status_code=status_code, body_text=body_text, body_json=body_json)
    finally:
        try:
            Path(tmp_path).unlink(missing_ok=True)
        except Exception:
            pass


def read_file_limited(path: str, *, limit_bytes: int = 65536) -> str:
    try:
        with open(path, "rb") as f:
            data = f.read(limit_bytes + 1)
        if len(data) > limit_bytes:
            data = data[:limit_bytes]
        return data.decode("utf-8", errors="replace")
    except Exception:
        return ""


def curl_raw(
    *,
    base_url: str,
    method: str,
    path: str,
    bearer: str | None = None,
    json_body: dict[str, Any] | None = None,
    extra_headers: dict[str, str] | None = None,
    timeout_s: int = 60,
    max_body_bytes: int = 65536,
) -> CurlRawResult:
    url = f"{base_url}{path}"
    tmp_body = tempfile.NamedTemporaryFile(prefix="gw_biz_body_", suffix=".txt", delete=False)
    tmp_body_path = tmp_body.name
    tmp_body.close()
    tmp_headers = tempfile.NamedTemporaryFile(prefix="gw_biz_hdr_", suffix=".txt", delete=False)
    tmp_headers_path = tmp_headers.name
    tmp_headers.close()
    try:
        cmd: list[str] = [
            "curl",
            "-sS",
            "--no-buffer",
            "-o",
            tmp_body_path,
            "-D",
            tmp_headers_path,
            "-w",
            "%{http_code}",
            "--max-time",
            str(timeout_s),
            "-X",
            method,
            url,
        ]
        if bearer:
            cmd += ["-H", f"Authorization: Bearer {bearer}"]
        if extra_headers:
            for k, v in extra_headers.items():
                cmd += ["-H", f"{k}: {v}"]
        if json_body is not None:
            cmd += ["-H", "Content-Type: application/json", "--data", json.dumps(json_body)]

        proc = run_cmd(cmd, timeout_s=timeout_s + 5)
        if proc.returncode != 0:
            stderr = redact_text(proc.stderr.strip())
            raise RuntimeError(f"curl failed ({method} {path}): {stderr}")
        status_code = int((proc.stdout or "").strip() or "0")
        headers_text = read_file_limited(tmp_headers_path, limit_bytes=8192)
        body_text = read_file_limited(tmp_body_path, limit_bytes=max_body_bytes)
        return CurlRawResult(status_code=status_code, headers_text=headers_text, body_text=body_text)
    finally:
        try:
            Path(tmp_body_path).unlink(missing_ok=True)
        except Exception:
            pass
        try:
            Path(tmp_headers_path).unlink(missing_ok=True)
        except Exception:
            pass


def ensure_error_shape(body_json: Any) -> bool:
    return isinstance(body_json, dict) and "code" in body_json and "message" in body_json


def ensure_models_shape(body_json: Any) -> bool:
    return (
        isinstance(body_json, dict)
        and isinstance(body_json.get("object"), str)
        and isinstance(body_json.get("data"), list)
        and all(isinstance(m, dict) for m in body_json.get("data"))
    )


def ensure_chat_shape(body_json: Any) -> bool:
    return (
        isinstance(body_json, dict)
        and isinstance(body_json.get("id"), str)
        and isinstance(body_json.get("object"), str)
        and isinstance(body_json.get("choices"), list)
    )


def git_sha_short() -> str:
    try:
        proc = run_cmd(["git", "rev-parse", "--short", "HEAD"], timeout_s=5)
        if proc.returncode == 0:
            return (proc.stdout or "").strip()
    except Exception:
        pass
    return "(unknown)"


def response_snippet(body_text: str, *, limit: int = 320) -> str:
    text = (body_text or "").strip()
    if not text:
        return ""
    try:
        parsed = json.loads(text)
        return json.dumps(redact_json(parsed), ensure_ascii=False)[:limit]
    except Exception:
        return redact_text(text[:limit])


def normalize_provider_base_url(*, api_type: str, base_url: str) -> tuple[str, str | None]:
    raw = (base_url or "").strip()
    if not raw:
        return "", None
    trimmed = raw.rstrip("/")
    if api_type == "openai" and trimmed.lower().endswith("/v1"):
        # Server-side OpenAI client appends `/v1/...`, so envs that include `/v1` will become `/v1/v1/...`.
        return trimmed[: -len("/v1")], "openai base_url endswith /v1 -> stripped"
    return trimmed, None


JWT_LIKE_RE = re.compile(r"eyJ[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,}")
OPENAI_KEY_LIKE_RE = re.compile(r"\\bsk-[A-Za-z0-9]{10,}\\b")


def assert_no_secret_leak(text: str, *, where: str) -> None:
    if "Authorization: Bearer " in text:
        raise RuntimeError(f"secret leak detected in {where}: Authorization header")
    if JWT_LIKE_RE.search(text):
        raise RuntimeError(f"secret leak detected in {where}: JWT-like token")
    if OPENAI_KEY_LIKE_RE.search(text):
        raise RuntimeError(f"secret leak detected in {where}: provider key-like token")
    m = re.search(r"(?i)\\bBearer\\s+(?!\\*\\*\\*REDACTED\\*\\*\\*)([A-Za-z0-9._+/=\\-]{20,})", text)
    if m:
        raise RuntimeError(f"secret leak detected in {where}: bearer token")


def append_workflow_record(*, report_path: Path, pass_count: int, fail_count: int, total: int) -> None:
    doc_path = ROOT_DIR / "workflow_follow.md"
    if not doc_path.exists():
        return
    stamp = utc_now().strftime("%Y-%m-%dT%H:%M:%SZ")
    status = "Pass" if fail_count == 0 else "Fail"
    try:
        report_display = report_path.relative_to(ROOT_DIR).as_posix()
    except Exception:
        report_display = report_path.as_posix()

    heading = "#### 接口测试记录（业务语义 biz）"
    record = (
        f"- biz2 业务语义补测（上游调用闭环/约束语义/日志统计） {stamp}：{status}"
        f"（Pass={pass_count} / Fail={fail_count} / Total={total}），报告：`{report_display}`\n"
    )

    text = doc_path.read_text(encoding="utf-8")
    if record in text:
        return

    lines = text.splitlines(keepends=True)
    heading_idx = None
    for i, line in enumerate(lines):
        if line.strip() == heading:
            heading_idx = i
            break

    if heading_idx is None:
        anchor_idx = None
        for i, line in enumerate(lines):
            if line.startswith("#### ") and "接口测试记录（OpenAPI 契约" in line:
                anchor_idx = i
                break
        if anchor_idx is None:
            if not text.endswith("\n"):
                text += "\n"
            doc_path.write_text(text + f"\n{heading}\n\n" + record, encoding="utf-8")
            return
        insert_at = None
        for j in range(anchor_idx + 1, len(lines)):
            if lines[j].startswith(("#### ", "### ", "## ")):
                insert_at = j
                break
        if insert_at is None:
            insert_at = len(lines)
        lines[insert_at:insert_at] = ["\n", f"{heading}\n", "\n"]
        heading_idx = insert_at + 1

    insert_at = None
    for j in range(heading_idx + 1, len(lines)):
        if lines[j].startswith(("#### ", "### ", "## ")):
            insert_at = j
            break
    if insert_at is None:
        insert_at = len(lines)

    lines.insert(insert_at, record)
    doc_path.write_text("".join(lines), encoding="utf-8")


@dataclass(frozen=True)
class CaseResult:
    suite: str
    name: str
    method: str
    path: str
    expected: str
    actual: int
    passed: bool
    request: str
    response: str


def fmt_request(
    *,
    method: str,
    path: str,
    auth_label: str | None = None,
    auth_secret: str | None = None,
    json_body: dict[str, Any] | None = None,
) -> str:
    parts = [f"{method} {path}"]
    if auth_label:
        if auth_secret:
            parts.append(f"auth={auth_label} Bearer <REDACTED {mask_secret(auth_secret, keep=0)}>")
        else:
            parts.append(f"auth={auth_label}")
    if json_body is not None:
        parts.append("json=" + json.dumps(redact_json(json_body), ensure_ascii=False))
    return " | ".join(parts)


def parse_models_ids(body_json: Any) -> list[str]:
    if not ensure_models_shape(body_json):
        return []
    out: list[str] = []
    for m in (body_json or {}).get("data", []):
        if isinstance(m, dict) and isinstance(m.get("id"), str) and m["id"].strip():
            out.append(m["id"].strip())
    return out


def pick_model_id(model_ids: list[str], *, prefer: str | None = None) -> str:
    ids = [m for m in model_ids if m]
    if not ids:
        return ""
    if prefer:
        p = prefer.strip()
        if p:
            for m in ids:
                if m == p:
                    return m
    for hint in ("gpt", "claude", "qwen", "glm"):
        for m in ids:
            if hint in m.lower():
                return m
    return ids[0]


def is_truthy_env(name: str, default: str = "0") -> bool:
    v = (os.getenv(name, default) or default).strip()
    return v in ("1", "true", "TRUE", "yes", "YES", "on", "ON")


def main() -> int:
    run_dt = utc_now()
    run_stamp = utc_compact_timestamp(run_dt)
    run_rand = secrets.token_hex(3)
    run_id = f"biz2_{run_stamp}_{run_rand}"

    out_dir = ROOT_DIR / "scripts" / "_biz"
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / f"{run_id}.md"
    log_path = out_dir / f"{run_id}.log"

    cfg = load_config()
    base_url = (pick(cfg, ["GATEWAY_BASE_URL"]) or "http://localhost:8080").rstrip("/")
    email = pick(cfg, ["EMAIL"]) or ""
    password = pick(cfg, ["PASSWORD"]) or ""
    bootstrap_code = pick(cfg, ["GATEWAY_BOOTSTRAP_CODE"]) or ""

    provider_api_type = (pick(cfg, ["PROVIDER_API_TYPE", "UPSTREAM_API_TYPE", "API_TYPE"]) or "openai").strip().lower()
    provider_base_url = pick(cfg, ["UPSTREAM_BASE_URL", "PROVIDER_BASE_URL", "OPENAI_BASE_URL", "BASEURL", "BASE_URL"]).strip()
    provider_api_key = pick(cfg, ["UPSTREAM_API_KEY", "PROVIDER_API_KEY", "OPENAI_API_KEY", "API_KEY", "APIKEY"]).strip()
    test_model = pick(cfg, ["BIZ_TEST_MODEL", "TEST_MODEL", "OPENAI_MODEL", "MODEL", "CHAT_MODEL"]).strip()

    run_chat = is_truthy_env("BIZ_RUN_CHAT", "0")
    chat_max_tokens = int((os.getenv("BIZ_CHAT_MAX_TOKENS", "16") or "16").strip() or "16")
    chat_max_tokens = max(1, min(chat_max_tokens, 32))

    log_lines: list[str] = []

    def log(line: str) -> None:
        log_lines.append(redact_text(line))

    if not email or not password:
        msg = "FATAL: missing config from .env/.env.example. Need EMAIL, PASSWORD. (GATEWAY_BASE_URL optional)"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2

    if provider_api_type not in ("openai", "anthropic", "zhipu"):
        provider_api_type = "openai"

    provider_base_url, provider_base_url_note = normalize_provider_base_url(
        api_type=provider_api_type, base_url=provider_base_url
    )

    if provider_base_url and provider_base_url.rstrip("/") == base_url.rstrip("/"):
        provider_base_url = ""

    if not provider_base_url or not provider_api_key:
        msg = (
            "FATAL: missing upstream provider config from .env/.env.example. Need provider base_url + api key "
            "(supported keys: BaseURl/BASEURL/UPSTREAM_BASE_URL + API KEY/API_KEY/UPSTREAM_API_KEY)."
        )
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2

    log("== Gateway Zero business semantic tests (biz2: upstream + constraints + observability) ==")
    log(f"time_utc: {run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}")
    log(f"git_sha : {git_sha_short()}")
    log(f"base_url: {base_url}")
    log(f"ready_check(required): curl {READY_CHECK_URL}")

    rc, code = curl_http_code(READY_CHECK_URL, timeout_s=8)
    if rc != 0 or not code or code == "000":
        msg = "FATAL: 无法连接到后端，请先启动数据库与后端：`docker start gateway-postgres` + `cargo run`"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2
    if code not in ("200", "401"):
        msg = f"FATAL: 后端就绪检查返回非预期 http_code={code}（仅 401/200 视为 OK）：{READY_CHECK_URL}"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2
    log(f"ready_check_ok: http_code={code} (401/200 都视为 OK)")

    log(f"superadmin_email: {mask_secret(email)}")
    log(f"provider_api_type: {provider_api_type}")
    log(f"provider_base_url: {mask_secret(provider_base_url, keep=12)}")
    if provider_base_url_note:
        log(f"provider_base_url_normalized: {provider_base_url_note}")
    log(f"provider_api_key: <REDACTED {mask_secret(provider_api_key, keep=0)}>")
    log(f"test_model: {test_model or '(unset)'}")
    log(f"BIZ_RUN_CHAT: {'1' if run_chat else '0'}")
    log(f"BIZ_CHAT_MAX_TOKENS: {chat_max_tokens}")

    bootstrap_fallback_used = False
    access_token: str | None = None

    def login(*, login_email: str, login_password: str) -> CurlResult:
        return curl_json(
            base_url=base_url,
            method="POST",
            path="/auth/login",
            json_body={"email": login_email, "password": login_password},
            timeout_s=30,
        )

    try:
        res = login(login_email=email, login_password=password)
        if res.status_code == 401 and bootstrap_code:
            log("WARN: superadmin login=401, trying one-time /auth/register bootstrap fallback")
            reg = curl_json(
                base_url=base_url,
                method="POST",
                path="/auth/register",
                json_body={"bootstrap_code": bootstrap_code, "email": email, "password": password},
                timeout_s=30,
            )
            log(f"register_status={reg.status_code} register_body={response_snippet(reg.body_text)}")
            if reg.status_code == 201:
                bootstrap_fallback_used = True
                res = login(login_email=email, login_password=password)

        if res.status_code != 200 or not isinstance(res.body_json, dict):
            raise RuntimeError(f"superadmin login failed: status={res.status_code} body={response_snippet(res.body_text)}")
        access_token = str(res.body_json.get("accessToken") or "")
        if not access_token:
            raise RuntimeError("superadmin login response missing accessToken")
        log(f"superadmin_accessToken: {mask_secret(access_token)}")
    except Exception as exc:
        msg = f"FATAL: {exc}"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2

    results: list[CaseResult] = []
    failures: list[CaseResult] = []

    def record(
        *,
        suite: str,
        name: str,
        method: str,
        path: str,
        expected: str,
        actual: int,
        passed: bool,
        request: str,
        response: str,
    ) -> None:
        r = CaseResult(
            suite=suite,
            name=name,
            method=method,
            path=path,
            expected=expected,
            actual=actual,
            passed=passed,
            request=request,
            response=response,
        )
        results.append(r)
        if not passed:
            failures.append(r)
        log(
            f"case suite={suite} name={name} req={request} exp={expected} act={actual} ok={passed} resp={response or '(empty)'}"
        )

    prefix = f"biz2_{run_stamp}_{run_rand}"
    provider_name = f"{prefix}_provider"
    token_name = f"{prefix}_clienttoken"

    created_token_id: str | None = None
    created_client_token: str | None = None
    created_provider = False
    provider_key_added = False
    selected_model_short: str | None = None
    selected_model: str | None = None
    selected_model_auto: bool = False
    created_p2_tokens: list[tuple[str, str]] = []  # (id, token)
    created_model_prices: list[tuple[str, str]] = []  # (provider, model_short)

    cleanup_notes: list[str] = []

    def cleanup_step(desc: str, fn) -> None:
        try:
            ok, note = fn()
            cleanup_notes.append(f"- {desc}：{'OK' if ok else 'SKIP/IGNORED'}（{note}）")
        except Exception as exc:
            cleanup_notes.append(f"- {desc}：FAIL（{redact_text(str(exc))}）")

    def create_token(
        *,
        name: str,
        enabled: bool = True,
        allowed_models: list[str] | None = None,
        expires_at: str | None = None,
        max_amount: float | None = None,
    ) -> tuple[CurlResult, str | None, str | None]:
        body: dict[str, Any] = {"name": name, "enabled": enabled}
        if allowed_models is not None:
            body["allowed_models"] = allowed_models
        if expires_at is not None:
            body["expires_at"] = expires_at
        if max_amount is not None:
            body["max_amount"] = max_amount
        r = curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/tokens",
            bearer=access_token,
            json_body=body,
            timeout_s=30,
        )
        tid = None
        tok = None
        if isinstance(r.body_json, dict):
            tid = str(r.body_json.get("id") or "") or None
            tok = str(r.body_json.get("token") or "") or None
        return r, tid, tok

    def toggle_token(*, token_id: str, enabled: bool) -> CurlResult:
        return curl_json(
            base_url=base_url,
            method="POST",
            path=f"/admin/tokens/{token_id}/toggle",
            bearer=access_token,
            json_body={"enabled": enabled},
            timeout_s=30,
        )

    def upsert_model_price(*, provider: str, model_short: str) -> CurlResult:
        body = {
            "provider": provider,
            "model": model_short,
            "prompt_price_per_million": 1.0,
            "completion_price_per_million": 1.0,
            "currency": "USD",
        }
        return curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/model-prices",
            bearer=access_token,
            json_body=body,
            timeout_s=30,
        )

    def docker_exec_psql(sql: str) -> tuple[bool, str]:
        sql_s = (sql or "").strip()
        if not sql_s:
            return False, "empty sql"
        try:
            proc = run_cmd(
                [
                    "docker",
                    "inspect",
                    "gateway-postgres",
                    "--format",
                    "{{range .Config.Env}}{{println .}}{{end}}",
                ],
                timeout_s=5,
            )
            if proc.returncode != 0:
                return False, "docker container gateway-postgres not found"
        except Exception as exc:
            return False, f"docker inspect failed: {redact_text(str(exc))}"

        user = "postgres"
        dbname = "gateway"
        try:
            for line in (proc.stdout or "").splitlines():
                if line.startswith("POSTGRES_USER="):
                    user = line.split("=", 1)[1].strip() or user
                elif line.startswith("POSTGRES_DB="):
                    dbname = line.split("=", 1)[1].strip() or dbname
        except Exception:
            pass
        try:
            proc = run_cmd(
                [
                    "docker",
                    "exec",
                    "gateway-postgres",
                    "psql",
                    "-U",
                    user,
                    "-d",
                    dbname,
                    "-v",
                    "ON_ERROR_STOP=1",
                    "-c",
                    sql_s,
                ],
                timeout_s=15,
            )
            if proc.returncode != 0:
                return False, redact_text((proc.stderr or proc.stdout or "").strip()[:200])
            return True, "ok"
        except Exception as exc:
            return False, redact_text(str(exc))

    def sql_quote(s: str) -> str:
        return (s or "").replace("'", "''")

    try:
        # --- A. ClientToken business effect ---
        suite = "P1.A.ClientToken"

        req = fmt_request(
            method="POST",
            path="/admin/tokens",
            auth_label="superadmin",
            auth_secret=access_token,
            json_body={"name": token_name, "enabled": True},
        )
        r = curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/tokens",
            bearer=access_token,
            json_body={"name": token_name, "enabled": True},
            timeout_s=30,
        )
        token_len = 0
        if isinstance(r.body_json, dict):
            created_token_id = str(r.body_json.get("id") or "") or None
            created_client_token = str(r.body_json.get("token") or "") or None
            if created_client_token:
                token_len = len(created_client_token)
        passed = r.status_code == 201 and bool(created_token_id) and bool(created_client_token)
        record(
            suite=suite,
            name="创建 ClientToken(enabled=true)",
            method="POST",
            path="/admin/tokens",
            expected="201 + {id} + {token(len)}",
            actual=r.status_code,
            passed=passed,
            request=req,
            response=f"id={created_token_id or '(missing)'} token={mask_secret(created_client_token, keep=0)}",
        )
        if not passed:
            raise RuntimeError("ClientToken create failed; aborting downstream A cases")

        assert created_client_token is not None
        assert created_token_id is not None

        req = fmt_request(
            method="GET",
            path="/v1/models",
            auth_label="client_token",
            auth_secret=created_client_token,
            json_body=None,
        )
        r = curl_json(
            base_url=base_url,
            method="GET",
            path="/v1/models",
            bearer=created_client_token,
            timeout_s=30,
        )
        ok = r.status_code == 200 and ensure_models_shape(r.body_json)
        record(
            suite=suite,
            name="使用 ClientToken 调用 /v1/models",
            method="GET",
            path="/v1/models",
            expected="200 + ModelsResponse",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            auth_label="superadmin",
            auth_secret=access_token,
            json_body={"enabled": False},
        )
        r = curl_json(
            base_url=base_url,
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            bearer=access_token,
            json_body={"enabled": False},
            timeout_s=30,
        )
        record(
            suite=suite,
            name="禁用 ClientToken(enabled=false)",
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            expected="200",
            actual=r.status_code,
            passed=r.status_code == 200,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(method="GET", path="/v1/models", auth_label="client_token", auth_secret=created_client_token)
        r = curl_json(base_url=base_url, method="GET", path="/v1/models", bearer=created_client_token, timeout_s=30)
        expected_reject = r.status_code if r.status_code in (401, 403) else 401
        ok = r.status_code in (401, 403) and ensure_error_shape(r.body_json)
        record(
            suite=suite,
            name="禁用后 /v1/models 应被拒绝",
            method="GET",
            path="/v1/models",
            expected="401/403 + {code,message}",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            auth_label="superadmin",
            auth_secret=access_token,
            json_body={"enabled": True},
        )
        r = curl_json(
            base_url=base_url,
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            bearer=access_token,
            json_body={"enabled": True},
            timeout_s=30,
        )
        record(
            suite=suite,
            name="重新启用 ClientToken(enabled=true)",
            method="POST",
            path=f"/admin/tokens/{created_token_id}/toggle",
            expected="200",
            actual=r.status_code,
            passed=r.status_code == 200,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(method="GET", path="/v1/models", auth_label="client_token", auth_secret=created_client_token)
        r = curl_json(base_url=base_url, method="GET", path="/v1/models", bearer=created_client_token, timeout_s=30)
        ok = r.status_code == 200 and ensure_models_shape(r.body_json)
        record(
            suite=suite,
            name="重新启用后 /v1/models 恢复可用",
            method="GET",
            path="/v1/models",
            expected="200 + ModelsResponse",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(method="DELETE", path=f"/admin/tokens/{created_token_id}", auth_label="superadmin", auth_secret=access_token)
        r = curl_json(base_url=base_url, method="DELETE", path=f"/admin/tokens/{created_token_id}", bearer=access_token, timeout_s=30)
        record(
            suite=suite,
            name="删除 ClientToken",
            method="DELETE",
            path=f"/admin/tokens/{created_token_id}",
            expected="204",
            actual=r.status_code,
            passed=r.status_code == 204,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(method="GET", path="/v1/models", auth_label="client_token", auth_secret=created_client_token)
        r = curl_json(base_url=base_url, method="GET", path="/v1/models", bearer=created_client_token, timeout_s=30)
        ok = r.status_code in (401, 403) and ensure_error_shape(r.body_json) and (r.status_code == expected_reject)
        record(
            suite=suite,
            name="删除后旧 ClientToken 应继续被拒绝",
            method="GET",
            path="/v1/models",
            expected=f"{expected_reject} + {{code,message}}",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        # mark token already deleted, but keep id around for cleanup best-effort

        # --- B. Providers/keys business effect (upstream-dependent, no billing) ---
        suite = "P1.B.ProvidersKeys"

        req = fmt_request(
            method="POST",
            path="/providers",
            auth_label="superadmin",
            auth_secret=access_token,
            json_body={"name": provider_name, "api_type": provider_api_type, "base_url": provider_base_url, "models_endpoint": None},
        )
        r = curl_json(
            base_url=base_url,
            method="POST",
            path="/providers",
            bearer=access_token,
            json_body={"name": provider_name, "api_type": provider_api_type, "base_url": provider_base_url, "models_endpoint": None},
            timeout_s=30,
        )
        created_provider = r.status_code == 200
        record(
            suite=suite,
            name="创建 Provider fixture",
            method="POST",
            path="/providers",
            expected="200",
            actual=r.status_code,
            passed=created_provider,
            request=req,
            response=response_snippet(r.body_text) or "",
        )
        if not created_provider:
            raise RuntimeError("provider fixture create failed; aborting downstream B cases")

        upstream_path = f"/models/{provider_name}?refresh=true"
        req = fmt_request(method="GET", path=upstream_path, auth_label="superadmin", auth_secret=access_token)
        r = curl_json(base_url=base_url, method="GET", path=upstream_path, bearer=access_token, timeout_s=45)
        missing_key_fail_code = r.status_code
        ok = r.status_code != 200
        if ok and r.body_json is not None and not ensure_error_shape(r.body_json):
            ok = False
        record(
            suite=suite,
            name="无 keys 时 refresh 拉取上游应失败",
            method="GET",
            path=upstream_path,
            expected="!=200 (+{code,message})",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=response_snippet(r.body_text) or "",
        )

        req = fmt_request(
            method="POST",
            path=f"/providers/{provider_name}/keys",
            auth_label="superadmin",
            auth_secret=access_token,
            json_body={"key": provider_api_key},
        )
        r = curl_json(
            base_url=base_url,
            method="POST",
            path=f"/providers/{provider_name}/keys",
            bearer=access_token,
            json_body={"key": provider_api_key},
            timeout_s=30,
        )
        provider_key_added = r.status_code == 200
        record(
            suite=suite,
            name="添加 Provider key",
            method="POST",
            path=f"/providers/{provider_name}/keys",
            expected="200",
            actual=r.status_code,
            passed=provider_key_added,
            request=req,
            response=response_snippet(r.body_text) or "",
        )
        if not provider_key_added:
            raise RuntimeError("provider key add failed; aborting downstream B cases")

        req = fmt_request(method="GET", path=upstream_path, auth_label="superadmin", auth_secret=access_token)
        r = curl_json(base_url=base_url, method="GET", path=upstream_path, bearer=access_token, timeout_s=45)
        ok = r.status_code == 200 and ensure_models_shape(r.body_json)
        model_ids = parse_models_ids(r.body_json)
        note = response_snippet(r.body_text) or ""
        if ok:
            if test_model:
                if test_model not in model_ids:
                    note = (note + " | " if note else "") + f"NOTE: env_test_model not found in refresh models: {test_model}"
                selected_model_short = test_model
                selected_model = f"{provider_name}/{selected_model_short}"
                selected_model_auto = False
            else:
                chosen = pick_model_id(model_ids, prefer=None)
                if chosen:
                    selected_model_short = chosen
                    selected_model = f"{provider_name}/{selected_model_short}"
                    selected_model_auto = True
                    note = (note + " | " if note else "") + f"auto_selected_model={selected_model}"
                else:
                    note = (note + " | " if note else "") + "NOTE: refresh models empty; auto selection failed"
        record(
            suite=suite,
            name="添加 key 后 refresh 拉取上游应成功",
            method="GET",
            path=upstream_path,
            expected="200 + ModelsResponse",
            actual=r.status_code,
            passed=ok,
            request=req,
            response=note,
        )

        if not run_chat:
            req = fmt_request(
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                auth_label="superadmin",
                auth_secret=access_token,
                json_body={"key": provider_api_key},
            )
            r = curl_json(
                base_url=base_url,
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                bearer=access_token,
                json_body={"key": provider_api_key},
                timeout_s=30,
            )
            record(
                suite=suite,
                name="删除 Provider key",
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                expected="200",
                actual=r.status_code,
                passed=r.status_code == 200,
                request=req,
                response=response_snippet(r.body_text) or "",
            )
            provider_key_added = False

            req = fmt_request(method="GET", path=upstream_path, auth_label="superadmin", auth_secret=access_token)
            r = curl_json(base_url=base_url, method="GET", path=upstream_path, bearer=access_token, timeout_s=45)
            ok = r.status_code != 200 and (r.body_json is None or ensure_error_shape(r.body_json))
            drift = ""
            if ok and missing_key_fail_code and r.status_code != missing_key_fail_code:
                drift = f"NOTE: failure code changed {missing_key_fail_code} -> {r.status_code}"
            record(
                suite=suite,
                name="删除 key 后 refresh 应再次失败",
                method="GET",
                path=upstream_path,
                expected=f"!=200 (prefer {missing_key_fail_code})",
                actual=r.status_code,
                passed=ok,
                request=req,
                response=((response_snippet(r.body_text) or "") + ((" | " + drift) if drift else "")),
            )
        else:
            p2_suite_a = "P2.A.UpstreamChat"
            p2_suite_b = "P2.B.TokenConstraints"
            p2_suite_c = "P2.C.Observability"

            if not selected_model_short or not selected_model:
                record(
                    suite=p2_suite_a,
                    name="前置：选择模型（refresh 结果为空或未配置）",
                    method="GET",
                    path=upstream_path,
                    expected="selected_model != empty",
                    actual=0,
                    passed=False,
                    request="(internal)",
                    response="missing selected_model",
                )
                raise RuntimeError("P2 aborted: missing selected_model")

            # Create dedicated chat token (unconstrained; model selection probe will use it)
            r_tok, tid_chat, tok_chat = create_token(name=f"{prefix}_p2_chat", enabled=True)
            ok_tok = r_tok.status_code == 201 and bool(tid_chat) and bool(tok_chat)
            record(
                suite=p2_suite_a,
                name="前置：创建 ClientToken（用于 /v1/chat/completions）",
                method="POST",
                path="/admin/tokens",
                expected="201 + {id,token}",
                actual=r_tok.status_code,
                passed=ok_tok,
                request=fmt_request(
                    method="POST",
                    path="/admin/tokens",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={"name": f"{prefix}_p2_chat", "enabled": True},
                ),
                response=f"id={tid_chat or '(missing)'} token={mask_secret(tok_chat, keep=0)}",
            )
            if not ok_tok:
                raise RuntimeError("P2 aborted: failed to create chat ClientToken")
            assert tid_chat is not None and tok_chat is not None
            created_p2_tokens.append((tid_chat, tok_chat))

            def chat_body(*, model: str, stream: bool) -> dict[str, Any]:
                body: dict[str, Any] = {
                    "model": model,
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": chat_max_tokens,
                    "temperature": 0,
                }
                if stream:
                    body["stream"] = True
                return body

            def score_model_id(m: str) -> int:
                s = (m or "").lower()
                if "claude" in s:
                    return 0
                if "qwen" in s:
                    return 1
                if "deepseek" in s:
                    return 2
                if "glm" in s:
                    return 3
                if "gpt" in s:
                    return 4
                return 9

            # Model selection probe: some upstream aggregators may return a 200 + {"error":...} JSON.
            probe_limit = int((os.getenv("BIZ_MODEL_PROBE_LIMIT", "3") or "3").strip() or "3")
            probe_limit = max(1, min(probe_limit, 6))
            candidates: list[str] = []
            if test_model:
                candidates = [test_model]
            else:
                seen: set[str] = set()
                ranked = sorted(model_ids, key=score_model_id)
                for m in ranked:
                    if m and m not in seen:
                        candidates.append(m)
                        seen.add(m)

            probe_selected: str | None = None
            probe_req: str | None = None
            probe_resp: CurlResult | None = None
            for i, cand in enumerate(candidates[:probe_limit]):
                cache_path = f"/models/{provider_name}/cache"
                cache_body = {"mode": "selected", "include": [cand], "replace": True}
                req = fmt_request(
                    method="POST",
                    path=cache_path,
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body=cache_body,
                )
                rc = curl_json(
                    base_url=base_url,
                    method="POST",
                    path=cache_path,
                    bearer=access_token,
                    json_body=cache_body,
                    timeout_s=60,
                )
                cached_ids = parse_models_ids(rc.body_json)
                ok_cache = rc.status_code == 200 and cand in cached_ids
                record(
                    suite=p2_suite_a,
                    name=f"前置：写入模型缓存（probe {i+1}）",
                    method="POST",
                    path=cache_path,
                    expected="200 + ModelsResponse(包含 cand)",
                    actual=rc.status_code,
                    passed=ok_cache,
                    request=req,
                    response=(response_snippet(rc.body_text) or "") + f" | cand={cand} contains_cand={cand in cached_ids}",
                )
                if not ok_cache:
                    continue

                req = fmt_request(
                    method="POST",
                    path="/admin/model-prices",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={
                        "provider": provider_name,
                        "model": cand,
                        "prompt_price_per_million": 1.0,
                        "completion_price_per_million": 1.0,
                        "currency": "USD",
                    },
                )
                rp = upsert_model_price(provider=provider_name, model_short=cand)
                created_model_prices.append((provider_name, cand))
                ok_price = rp.status_code in (200, 201)
                record(
                    suite=p2_suite_a,
                    name=f"前置：设置 model price（probe {i+1}）",
                    method="POST",
                    path="/admin/model-prices",
                    expected="201/200",
                    actual=rp.status_code,
                    passed=ok_price,
                    request=req,
                    response=response_snippet(rp.body_text) or "",
                )
                if not ok_price:
                    continue

                # Probe a minimal non-stream chat request for this candidate
                probe_model = f"{provider_name}/{cand}"
                body = chat_body(model=probe_model, stream=False)
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=tok_chat,
                    json_body=body,
                )
                rr = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=tok_chat,
                    json_body=body,
                    timeout_s=90,
                )
                ok_probe = rr.status_code == 200 and ensure_chat_shape(rr.body_json) and isinstance(rr.body_json, dict) and "error" not in rr.body_json
                record(
                    suite=p2_suite_a,
                    name=f"前置：探测模型可用性（probe {i+1}）",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="200 + chat response（无顶层 error）",
                    actual=rr.status_code,
                    passed=ok_probe,
                    request=req,
                    response=response_snippet(rr.body_text) or "",
                )
                if ok_probe:
                    probe_selected = cand
                    probe_req = req
                    probe_resp = rr
                    break

            if probe_selected:
                selected_model_short = probe_selected
                selected_model = f"{provider_name}/{selected_model_short}"
                selected_model_auto = True
            else:
                record(
                    suite=p2_suite_a,
                    name="前置：自动选择模型失败（探测均未通过）",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="至少 1 个 model probe 成功",
                    actual=0,
                    passed=False,
                    request="(internal)",
                    response=f"probe_limit={probe_limit} candidates={len(candidates)}",
                )

            # A1 non-stream chat
            body_a1 = chat_body(model=selected_model, stream=False) if selected_model else {"model": "(unset)"}
            if probe_req and probe_resp:
                req = probe_req
                r = probe_resp
            else:
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=tok_chat,
                    json_body=body_a1,
                )
                r = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=tok_chat,
                    json_body=body_a1,
                    timeout_s=90,
                )
            ok = r.status_code == 200 and ensure_chat_shape(r.body_json) and isinstance(r.body_json, dict) and "error" not in r.body_json
            record(
                suite=p2_suite_a,
                name="A1 非流式 chat/completions 成功（可能产生费用）",
                method="POST",
                path="/v1/chat/completions",
                expected="200 + {id,object,choices}",
                actual=r.status_code,
                passed=ok,
                request=req,
                response=response_snippet(r.body_text) or "",
            )

            # A2 SSE stream chat (skip for anthropic)
            if provider_api_type == "anthropic":
                record(
                    suite=p2_suite_a,
                    name="A2 SSE 流式（Anthropic 未实现，Skip）",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="skip",
                    actual=0,
                    passed=True,
                    request="(skipped)",
                    response="provider_api_type=anthropic streaming not implemented",
                )
            else:
                body_a2 = chat_body(model=selected_model, stream=True)
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=tok_chat,
                    json_body=body_a2,
                )
                rr = curl_raw(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=tok_chat,
                    json_body=body_a2,
                    extra_headers={"Accept": "text/event-stream"},
                    timeout_s=90,
                    max_body_bytes=65536,
                )
                hdrs = (rr.headers_text or "").lower()
                has_ct = "content-type:" in hdrs and "text/event-stream" in hdrs
                has_data = "data:" in (rr.body_text or "")
                ok = rr.status_code == 200 and (has_ct or has_data)
                record(
                    suite=p2_suite_a,
                    name="A2 SSE 流式 chat/completions 成功（可能产生费用）",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="200 + (event-stream or data:)",
                    actual=rr.status_code,
                    passed=ok,
                    request=req,
                    response=f"content_type_event_stream={has_ct} body_has_data={has_data} body_snip={redact_text((rr.body_text or '')[:240])}",
                )

            # C logs/metrics closure (retry for async writes)
            def has_chat_log(body_json: Any) -> bool:
                if not isinstance(body_json, dict):
                    return False
                data = body_json.get("data")
                if not isinstance(data, list):
                    return False
                for item in data:
                    if not isinstance(item, dict):
                        continue
                    if str(item.get("path") or "") != "/v1/chat/completions":
                        continue
                    if str(item.get("provider") or "") != provider_name:
                        continue
                    return True
                return False

            def get_admin_json_with_retry(path: str, tries: int = 3) -> CurlResult:
                last: CurlResult | None = None
                for i in range(tries):
                    last = curl_json(
                        base_url=base_url,
                        method="GET",
                        path=path,
                        bearer=access_token,
                        timeout_s=30,
                    )
                    if last.status_code == 200:
                        return last
                    if i < tries - 1:
                        time.sleep(i + 1)
                assert last is not None
                return last

            req = fmt_request(
                method="GET",
                path="/admin/logs/chat-completions?limit=20",
                auth_label="superadmin",
                auth_secret=access_token,
            )
            rlog = get_admin_json_with_retry("/admin/logs/chat-completions?limit=20", tries=3)
            ok = rlog.status_code == 200 and has_chat_log(rlog.body_json)
            record(
                suite=p2_suite_c,
                name="C1 chat 请求日志可查询",
                method="GET",
                path="/admin/logs/chat-completions",
                expected="200 + data 中包含本 provider 的 /v1/chat/completions",
                actual=rlog.status_code,
                passed=ok,
                request=req,
                response=response_snippet(rlog.body_text) or "",
            )

            req = fmt_request(
                method="GET",
                path="/admin/metrics/summary?window_minutes=60",
                auth_label="superadmin",
                auth_secret=access_token,
            )
            rmet = curl_json(
                base_url=base_url,
                method="GET",
                path="/admin/metrics/summary?window_minutes=60",
                bearer=access_token,
                timeout_s=30,
            )
            ok = rmet.status_code == 200 and isinstance(rmet.body_json, dict)
            record(
                suite=p2_suite_c,
                name="C2 metrics summary 可查询",
                method="GET",
                path="/admin/metrics/summary",
                expected="200 + MetricsSummary",
                actual=rmet.status_code,
                passed=ok,
                request=req,
                response=response_snippet(rmet.body_text) or "",
            )

            req = fmt_request(
                method="GET",
                path="/admin/logs/requests?limit=20&path=/v1/chat/completions",
                auth_label="superadmin",
                auth_secret=access_token,
            )
            rreq = get_admin_json_with_retry("/admin/logs/requests?limit=20&path=/v1/chat/completions", tries=3)
            ok = rreq.status_code == 200 and isinstance(rreq.body_json, dict) and isinstance(rreq.body_json.get("data"), list)
            record(
                suite=p2_suite_c,
                name="C3 requests 日志（按 path 过滤）可查询",
                method="GET",
                path="/admin/logs/requests",
                expected="200 + RequestLogsResponse",
                actual=rreq.status_code,
                passed=ok,
                request=req,
                response=response_snippet(rreq.body_text) or "",
            )

            # A3 delete provider key then chat should fail
            req = fmt_request(
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                auth_label="superadmin",
                auth_secret=access_token,
                json_body={"key": provider_api_key},
            )
            rk = curl_json(
                base_url=base_url,
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                bearer=access_token,
                json_body={"key": provider_api_key},
                timeout_s=30,
            )
            record(
                suite=p2_suite_a,
                name="A3 删除 provider key",
                method="DELETE",
                path=f"/providers/{provider_name}/keys",
                expected="200",
                actual=rk.status_code,
                passed=rk.status_code == 200,
                request=req,
                response=response_snippet(rk.body_text) or "",
            )
            provider_key_added = False

            req_chat = fmt_request(
                method="POST",
                path="/v1/chat/completions",
                auth_label="client_token",
                auth_secret=tok_chat,
                json_body=body_a1,
            )
            r = curl_json(
                base_url=base_url,
                method="POST",
                path="/v1/chat/completions",
                bearer=tok_chat,
                json_body=body_a1,
                timeout_s=60,
            )
            ok = r.status_code != 200 and ensure_error_shape(r.body_json)
            record(
                suite=p2_suite_a,
                name="A3 删除 key 后 chat 应失败",
                method="POST",
                path="/v1/chat/completions",
                expected="!=200 + {code,message}",
                actual=r.status_code,
                passed=ok,
                request=req_chat,
                response=response_snippet(r.body_text) or "",
            )

            # Re-add key for remaining cases
            req = fmt_request(
                method="POST",
                path=f"/providers/{provider_name}/keys",
                auth_label="superadmin",
                auth_secret=access_token,
                json_body={"key": provider_api_key},
            )
            rk2 = curl_json(
                base_url=base_url,
                method="POST",
                path=f"/providers/{provider_name}/keys",
                bearer=access_token,
                json_body={"key": provider_api_key},
                timeout_s=30,
            )
            provider_key_added = rk2.status_code == 200
            record(
                suite=p2_suite_a,
                name="A4 前置：重新添加 provider key",
                method="POST",
                path=f"/providers/{provider_name}/keys",
                expected="200",
                actual=rk2.status_code,
                passed=provider_key_added,
                request=req,
                response=response_snippet(rk2.body_text) or "",
            )

            rt = toggle_token(token_id=tid_chat, enabled=False)
            record(
                suite=p2_suite_a,
                name="A4 禁用 ClientToken(enabled=false)",
                method="POST",
                path=f"/admin/tokens/{tid_chat}/toggle",
                expected="200",
                actual=rt.status_code,
                passed=rt.status_code == 200,
                request=fmt_request(
                    method="POST",
                    path=f"/admin/tokens/{tid_chat}/toggle",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={"enabled": False},
                ),
                response=response_snippet(rt.body_text) or "",
            )

            req_chat = fmt_request(
                method="POST",
                path="/v1/chat/completions",
                auth_label="client_token",
                auth_secret=tok_chat,
                json_body=body_a1,
            )
            r = curl_json(
                base_url=base_url,
                method="POST",
                path="/v1/chat/completions",
                bearer=tok_chat,
                json_body=body_a1,
                timeout_s=60,
            )
            ok = r.status_code in (400, 401, 403) and ensure_error_shape(r.body_json)
            record(
                suite=p2_suite_a,
                name="A4 禁用 token 后 chat 应失败",
                method="POST",
                path="/v1/chat/completions",
                expected="401/403（实现可能为 400） + {code,message}",
                actual=r.status_code,
                passed=ok,
                request=req_chat,
                response=response_snippet(r.body_text) or "",
            )

            # B1 allowed_models allow => success
            r_allow, tid_allow, tok_allow = create_token(
                name=f"{prefix}_p2_allow_ok", enabled=True, allowed_models=[selected_model]
            )
            ok_tok = r_allow.status_code == 201 and bool(tid_allow) and bool(tok_allow)
            record(
                suite=p2_suite_b,
                name="B1 创建 token（allowed_models=[selected_model]）",
                method="POST",
                path="/admin/tokens",
                expected="201",
                actual=r_allow.status_code,
                passed=ok_tok,
                request=fmt_request(
                    method="POST",
                    path="/admin/tokens",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={"name": f"{prefix}_p2_allow_ok", "enabled": True, "allowed_models": [selected_model]},
                ),
                response=f"id={tid_allow or '(missing)'} token={mask_secret(tok_allow, keep=0)}",
            )
            if ok_tok:
                assert tid_allow is not None and tok_allow is not None
                created_p2_tokens.append((tid_allow, tok_allow))
                body = chat_body(model=selected_model, stream=False)
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=tok_allow,
                    json_body=body,
                )
                r = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=tok_allow,
                    json_body=body,
                    timeout_s=90,
                )
                ok = r.status_code == 200 and ensure_chat_shape(r.body_json)
                record(
                    suite=p2_suite_b,
                    name="B1 allowed_models 允许模型 => 成功",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="200",
                    actual=r.status_code,
                    passed=ok,
                    request=req,
                    response=response_snippet(r.body_text) or "",
                )

                # B2 allowed_models reject other model
                alt_short = ""
                for mid in model_ids:
                    if mid and mid != selected_model_short:
                        alt_short = mid
                        break
                if not alt_short:
                    record(
                        suite=p2_suite_b,
                        name="B2 allowed_models 不允许模型（Skip：仅 1 个 model）",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="skip",
                        actual=0,
                        passed=True,
                        request="(skipped)",
                        response="refresh models only has 1 item",
                    )
                else:
                    alt_model = f"{provider_name}/{alt_short}"
                    body = chat_body(model=alt_model, stream=False)
                    req = fmt_request(
                        method="POST",
                        path="/v1/chat/completions",
                        auth_label="client_token",
                        auth_secret=tok_allow,
                        json_body=body,
                    )
                    r = curl_json(
                        base_url=base_url,
                        method="POST",
                        path="/v1/chat/completions",
                        bearer=tok_allow,
                        json_body=body,
                        timeout_s=60,
                    )
                    ok = r.status_code in (400, 401, 403) and ensure_error_shape(r.body_json)
                    record(
                        suite=p2_suite_b,
                        name="B2 allowed_models 不允许模型 => 被拒绝",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="400/401/403 + {code,message}",
                        actual=r.status_code,
                        passed=ok,
                        request=req,
                        response=response_snippet(r.body_text) or "",
                    )

            # B3 expires_at past => reject (use /v1/models)
            exp_past = (utc_now() - timedelta(hours=1)).replace(microsecond=0).isoformat().replace("+00:00", "Z")
            r_exp, tid_exp, tok_exp = create_token(name=f"{prefix}_p2_expired", enabled=True, expires_at=exp_past)
            ok_tok = r_exp.status_code == 201 and bool(tid_exp) and bool(tok_exp)
            record(
                suite=p2_suite_b,
                name="B3 创建 token（expires_at=过去）",
                method="POST",
                path="/admin/tokens",
                expected="201",
                actual=r_exp.status_code,
                passed=ok_tok,
                request=fmt_request(
                    method="POST",
                    path="/admin/tokens",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={"name": f"{prefix}_p2_expired", "enabled": True, "expires_at": exp_past},
                ),
                response=f"id={tid_exp or '(missing)'} token={mask_secret(tok_exp, keep=0)} expires_at={exp_past}",
            )
            if ok_tok:
                assert tid_exp is not None and tok_exp is not None
                created_p2_tokens.append((tid_exp, tok_exp))
                req = fmt_request(method="GET", path="/v1/models", auth_label="client_token", auth_secret=tok_exp)
                r = curl_json(base_url=base_url, method="GET", path="/v1/models", bearer=tok_exp, timeout_s=30)
                ok = r.status_code in (401, 403) and ensure_error_shape(r.body_json)
                record(
                    suite=p2_suite_b,
                    name="B3 expires_at 过期 token => 被拒绝",
                    method="GET",
                    path="/v1/models",
                    expected="401/403 + {code,message}",
                    actual=r.status_code,
                    passed=ok,
                    request=req,
                    response=response_snippet(r.body_text) or "",
                )

            # B4 max_amount tiny => reject (may take 2 calls)
            r_budget, tid_budget, tok_budget = create_token(name=f"{prefix}_p2_budget", enabled=True, max_amount=0.0)
            ok_tok = r_budget.status_code == 201 and bool(tid_budget) and bool(tok_budget)
            record(
                suite=p2_suite_b,
                name="B4 创建 token（max_amount=0）",
                method="POST",
                path="/admin/tokens",
                expected="201",
                actual=r_budget.status_code,
                passed=ok_tok,
                request=fmt_request(
                    method="POST",
                    path="/admin/tokens",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={"name": f"{prefix}_p2_budget", "enabled": True, "max_amount": 0.0},
                ),
                response=f"id={tid_budget or '(missing)'} token={mask_secret(tok_budget, keep=0)}",
            )
            if ok_tok:
                assert tid_budget is not None and tok_budget is not None
                created_p2_tokens.append((tid_budget, tok_budget))
                body = chat_body(model=selected_model, stream=False)
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=tok_budget,
                    json_body=body,
                )
                r1 = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=tok_budget,
                    json_body=body,
                    timeout_s=90,
                )
                if r1.status_code != 200 and ensure_error_shape(r1.body_json):
                    record(
                        suite=p2_suite_b,
                        name="B4 max_amount=0 => 第一次即被拒绝（可接受）",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="!=200 + {code,message}",
                        actual=r1.status_code,
                        passed=True,
                        request=req,
                        response=response_snippet(r1.body_text) or "",
                    )
                elif r1.status_code == 200 and ensure_chat_shape(r1.body_json) and isinstance(r1.body_json, dict) and "error" not in r1.body_json:
                    record(
                        suite=p2_suite_b,
                        name="B4 第一次请求（可能通过并消耗额度）",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="200 或 被拒绝（实现差异）",
                        actual=r1.status_code,
                        passed=True,
                        request=req,
                        response=response_snippet(r1.body_text) or "",
                    )
                    r2 = curl_json(
                        base_url=base_url,
                        method="POST",
                        path="/v1/chat/completions",
                        bearer=tok_budget,
                        json_body=body,
                        timeout_s=60,
                    )
                    ok = r2.status_code != 200 and ensure_error_shape(r2.body_json)
                    record(
                        suite=p2_suite_b,
                        name="B4 第二次请求应因额度不足被拒绝",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="!=200 + {code,message}",
                        actual=r2.status_code,
                        passed=ok,
                        request=req,
                        response=response_snippet(r2.body_text) or "",
                    )
                else:
                    record(
                        suite=p2_suite_b,
                        name="B4 第一次请求返回异常体（无法验证额度语义）",
                        method="POST",
                        path="/v1/chat/completions",
                        expected="200 + chat response 或 !=200 + {code,message}",
                        actual=r1.status_code,
                        passed=False,
                        request=req,
                        response=response_snippet(r1.body_text) or "",
                    )

            # B5 disabled + allowed_models priority observation (use /v1/models)
            r_dis, tid_dis, tok_dis = create_token(
                name=f"{prefix}_p2_disabled_allow", enabled=False, allowed_models=[selected_model]
            )
            ok_tok = r_dis.status_code == 201 and bool(tid_dis) and bool(tok_dis)
            record(
                suite=p2_suite_b,
                name="B5 创建 token（enabled=false + allowed_models）",
                method="POST",
                path="/admin/tokens",
                expected="201",
                actual=r_dis.status_code,
                passed=ok_tok,
                request=fmt_request(
                    method="POST",
                    path="/admin/tokens",
                    auth_label="superadmin",
                    auth_secret=access_token,
                    json_body={
                        "name": f"{prefix}_p2_disabled_allow",
                        "enabled": False,
                        "allowed_models": [selected_model],
                    },
                ),
                response=f"id={tid_dis or '(missing)'} token={mask_secret(tok_dis, keep=0)}",
            )
            if ok_tok:
                assert tid_dis is not None and tok_dis is not None
                created_p2_tokens.append((tid_dis, tok_dis))
                req = fmt_request(method="GET", path="/v1/models", auth_label="client_token", auth_secret=tok_dis)
                r = curl_json(base_url=base_url, method="GET", path="/v1/models", bearer=tok_dis, timeout_s=30)
                ok = r.status_code in (401, 403) and ensure_error_shape(r.body_json)
                msg = ""
                if isinstance(r.body_json, dict):
                    msg = str(r.body_json.get("message") or "")
                priority = "disabled" if "disabled" in msg.lower() else "other"
                record(
                    suite=p2_suite_b,
                    name="B5 disabled + allowed_models 优先级观察",
                    method="GET",
                    path="/v1/models",
                    expected="401/403 + {code,message}",
                    actual=r.status_code,
                    passed=ok,
                    request=req,
                    response=(response_snippet(r.body_text) or "") + f" | priority_hint={priority}",
                )

    finally:
        if provider_key_added:
            cleanup_step(
                "cleanup: 删除 provider key",
                lambda: (
                    (rr := curl_json(
                        base_url=base_url,
                        method="DELETE",
                        path=f"/providers/{provider_name}/keys",
                        bearer=access_token,
                        json_body={"key": provider_api_key},
                        timeout_s=30,
                    )).status_code in (200, 404),
                    f"status={rr.status_code} (404 ignored)",
                ),
            )
        if created_provider:
            cleanup_step(
                "cleanup: 删除 model cache（provider scope）",
                lambda: (
                    (rr := curl_json(
                        base_url=base_url,
                        method="DELETE",
                        path=f"/models/{provider_name}/cache",
                        bearer=access_token,
                        json_body={"ids": []},
                        timeout_s=30,
                    )).status_code in (200, 404),
                    f"status={rr.status_code} (404 ignored)",
                ),
            )
        if created_provider:
            cleanup_step(
                "cleanup: 删除 provider",
                lambda: (
                    (rr := curl_json(
                        base_url=base_url,
                        method="DELETE",
                        path=f"/providers/{provider_name}",
                        bearer=access_token,
                        timeout_s=30,
                    )).status_code in (200, 404),
                    f"status={rr.status_code} (404 ignored)",
                ),
            )
        if created_token_id:
            cleanup_step(
                "cleanup: 删除 ClientToken",
                lambda: (
                    (rr := curl_json(
                        base_url=base_url,
                        method="DELETE",
                        path=f"/admin/tokens/{created_token_id}",
                        bearer=access_token,
                        timeout_s=30,
                    )).status_code in (204, 404),
                    f"status={rr.status_code} (404 ignored)",
                ),
            )
        for tid, _tok in created_p2_tokens:
            cleanup_step(
                f"cleanup: 删除 P2 ClientToken({tid})",
                lambda tid=tid: (
                    (rr := curl_json(
                        base_url=base_url,
                        method="DELETE",
                        path=f"/admin/tokens/{tid}",
                        bearer=access_token,
                        timeout_s=30,
                    )).status_code in (204, 404),
                    f"status={rr.status_code} (404 ignored)",
                ),
            )
        if created_model_prices:
            cleanup_step(
                "cleanup: DB 删除 model_prices/cached_models（best-effort）",
                lambda: docker_exec_psql(
                    f"DELETE FROM model_prices WHERE provider='{sql_quote(provider_name)}'; DELETE FROM cached_models WHERE provider='{sql_quote(provider_name)}';"
                ),
            )

    pass_count = sum(1 for r in results if r.passed)
    fail_count = sum(1 for r in results if not r.passed)
    total = len(results)
    conclusion = "PASS" if fail_count == 0 else "FAIL"

    report_lines: list[str] = []
    report_lines.append("# biz2 业务语义补测（上游调用闭环 + 约束语义 + 日志/统计闭环）\n\n")
    report_lines.append(f"- time_utc: `{run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}`\n")
    report_lines.append(f"- base_url: `{base_url}`\n")
    report_lines.append(f"- git_sha: `{git_sha_short()}`\n")
    report_lines.append(f"- BIZ_RUN_CHAT: `{'1' if run_chat else '0'}`\n")
    report_lines.append(f"- BIZ_CHAT_MAX_TOKENS: `{chat_max_tokens}`\n")
    if bootstrap_fallback_used:
        report_lines.append(
            "- bootstrap fallback: `YES`（发生副作用：触发 `/auth/register` 创建首个 superadmin 用户）\n"
        )
    else:
        report_lines.append("- bootstrap fallback: `NO`\n")
    report_lines.append(f"- provider_api_type: `{provider_api_type}`\n")
    report_lines.append(f"- provider_base_url: `{provider_base_url}`\n")
    report_lines.append(f"- provider_api_key: `<REDACTED {mask_secret(provider_api_key, keep=0)}>`\n")
    report_lines.append(f"- env_test_model: `{test_model or '(unset)'}`\n")
    report_lines.append(f"- selected_model: `{selected_model or '(unset)'}`\n")
    report_lines.append(
        f"- selected_model_source: `{'auto(refresh)' if selected_model_auto else 'env' if test_model else '(unset)'}`\n"
    )
    report_lines.append("\n")

    if run_chat:
        report_lines.append(
            "> ⚠️ 注意：本次启用了 `BIZ_RUN_CHAT=1`，会调用真实上游并可能产生少量费用（已尽量控制请求次数与 max_tokens）。\n\n"
        )
    else:
        report_lines.append("> 本次未启用 `BIZ_RUN_CHAT=1`，仅运行低成本用例（不包含上游 chat/日志统计闭环）。\n\n")

    report_lines.append("## 用例表\n\n")
    report_lines.append("| 用例 | 请求 | 期望 | 实际 | 结果 | 响应摘要（脱敏） |\n")
    report_lines.append("|---|---|---|---:|---|---|\n")
    for r in results:
        title = f"{r.suite} / {r.name}"
        res = "Pass" if r.passed else "Fail"
        req = redact_text(r.request).replace("|", "\\|")
        resp = redact_text(r.response).replace("|", "\\|")
        report_lines.append(f"| {title} | `{req}` | {r.expected} | {r.actual} | **{res}** | {resp} |\n")

    report_lines.append("\n## 汇总\n\n")
    report_lines.append(f"- Pass={pass_count} / Fail={fail_count} / Total={total}\n")
    report_lines.append(f"- 结论：**{conclusion}**\n")

    report_lines.append("\n## 失败项列表（如有）\n\n")
    if failures:
        for r in failures:
            report_lines.append(f"- {r.suite} / {r.name}：`{r.method} {r.path}` exp={r.expected} act={r.actual}\n")
    else:
        report_lines.append("- 无\n")

    report_lines.append("\n## Cleanup\n\n")
    if cleanup_notes:
        report_lines.extend([line + "\n" for line in cleanup_notes])
    else:
        report_lines.append("- (none)\n")

    report_lines.append("\n## 自我评估\n\n")
    report_lines.append("- 脱敏覆盖：已对 JWT/refreshToken/password/client token/provider key 做脱敏输出（脚本包含泄露自检）\n")
    if run_chat:
        report_lines.append(f"- 费用控制：max_tokens={chat_max_tokens}，尽量减少 chat 调用次数；仍可能产生少量真实上游费用\n")
    else:
        report_lines.append("- 费用控制：未启用上游 chat 调用（BIZ_RUN_CHAT=0）\n")
    report_lines.append("- Cleanup：provider/key/tokens 均做 best-effort 删除；model_prices/cached_models 尝试通过 DB 清理（如可用）\n")
    report_lines.append("- Flakiness：上游稳定性/网络抖动/日志异步写入可能导致波动，已加入少量重试\n")

    report_text = "".join(report_lines)
    report_text = redact_text(report_text)
    log_text = "\n".join(log_lines) + "\n"
    log_text = redact_text(log_text)

    try:
        assert_no_secret_leak(report_text, where=str(report_path))
        assert_no_secret_leak(log_text, where=str(log_path))
    except Exception as exc:
        report_text += f"\n\nFATAL: redaction self-check failed: {exc}\n"
        conclusion = "FAIL"

    report_path.write_text(report_text, encoding="utf-8")
    log_path.write_text(log_text, encoding="utf-8")

    if total > 0:
        append_workflow_record(report_path=report_path, pass_count=pass_count, fail_count=fail_count, total=total)

    sys.stdout.write(f"{conclusion}: report={report_path.relative_to(ROOT_DIR).as_posix()} log={log_path.relative_to(ROOT_DIR).as_posix()}\n")
    return 0 if conclusion == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
