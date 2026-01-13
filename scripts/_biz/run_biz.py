#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import re
import secrets
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
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


def run_cmd(cmd: list[str], *, timeout_s: int = 30) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout_s)


def curl_http_code(url: str, *, timeout_s: int = 8) -> tuple[int, str]:
    cmd = ["curl", "-sS", "-o", "/dev/null", "-w", "%{http_code}", url]
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


def ensure_error_shape(body_json: Any) -> bool:
    return isinstance(body_json, dict) and "code" in body_json and "message" in body_json


def ensure_models_shape(body_json: Any) -> bool:
    return (
        isinstance(body_json, dict)
        and isinstance(body_json.get("object"), str)
        and isinstance(body_json.get("data"), list)
        and all(isinstance(m, dict) for m in body_json.get("data"))
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
        f"- ClientToken & Providers/keys 业务语义测试（自动化） {stamp}：{status}"
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


def main() -> int:
    run_dt = utc_now()
    run_stamp = utc_compact_timestamp(run_dt)
    run_rand = secrets.token_hex(3)
    run_id = f"biz_{run_stamp}_{run_rand}"

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

    run_chat = (os.getenv("BIZ_RUN_CHAT", "0") or "0").strip() in ("1", "true", "TRUE", "yes", "YES")

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

    log("== Gateway Zero business semantic tests (ClientToken & Providers/keys) ==")
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

    prefix = f"biz_{run_stamp}_{run_rand}"
    provider_name = f"{prefix}_provider"
    token_name = f"{prefix}_clienttoken"

    created_token_id: str | None = None
    created_client_token: str | None = None
    created_provider = False
    provider_key_added = False

    cleanup_notes: list[str] = []

    def cleanup_step(desc: str, fn) -> None:
        try:
            ok, note = fn()
            cleanup_notes.append(f"- {desc}：{'OK' if ok else 'SKIP/IGNORED'}（{note}）")
        except Exception as exc:
            cleanup_notes.append(f"- {desc}：FAIL（{redact_text(str(exc))}）")

    try:
        # --- A. ClientToken business effect ---
        suite = "A.ClientToken"

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
        suite = "B.ProvidersKeys"

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
        note = response_snippet(r.body_text) or ""
        if ok and test_model:
            try:
                models = [str(m.get("id") or "") for m in (r.body_json or {}).get("data", []) if isinstance(m, dict)]
                if test_model not in models:
                    note = (note + " | " if note else "") + f"NOTE: test_model={test_model} not found in returned models"
            except Exception:
                note = (note + " | " if note else "") + "NOTE: model assertion skipped (parse error)"
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

        if run_chat:
            chat_suite = "B.Chat(Optional)"
            if not test_model:
                record(
                    suite=chat_suite,
                    name="BIZ_RUN_CHAT=1 但缺少 model 配置",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="需配置 TEST_MODEL/BIZ_TEST_MODEL",
                    actual=0,
                    passed=False,
                    request="(skipped)",
                    response="missing test_model",
                )
            else:
                body = {
                    "model": test_model,
                    "messages": [{"role": "user", "content": "ping"}],
                    "max_tokens": 1,
                    "stream": False,
                    "temperature": 0,
                }
                req = fmt_request(
                    method="POST",
                    path="/v1/chat/completions",
                    auth_label="client_token",
                    auth_secret=created_client_token,
                    json_body=body,
                )
                r = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/v1/chat/completions",
                    bearer=created_client_token,
                    json_body=body,
                    timeout_s=60,
                )
                ok = r.status_code == 200
                record(
                    suite=chat_suite,
                    name="最小 chat/completions（可能产生费用）",
                    method="POST",
                    path="/v1/chat/completions",
                    expected="200",
                    actual=r.status_code,
                    passed=ok,
                    request=req,
                    response=response_snippet(r.body_text) or "",
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

    pass_count = sum(1 for r in results if r.passed)
    fail_count = sum(1 for r in results if not r.passed)
    total = len(results)
    conclusion = "PASS" if fail_count == 0 else "FAIL"

    report_lines: list[str] = []
    report_lines.append("# ClientToken & Providers/keys 业务语义测试（自动化）\n\n")
    report_lines.append(f"- time_utc: `{run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}`\n")
    report_lines.append(f"- base_url: `{base_url}`\n")
    report_lines.append(f"- git_sha: `{git_sha_short()}`\n")
    report_lines.append(f"- BIZ_RUN_CHAT: `{'1' if run_chat else '0'}`\n")
    if bootstrap_fallback_used:
        report_lines.append(
            "- bootstrap fallback: `YES`（发生副作用：触发 `/auth/register` 创建首个 superadmin 用户）\n"
        )
    else:
        report_lines.append("- bootstrap fallback: `NO`\n")
    report_lines.append(f"- provider_api_type: `{provider_api_type}`\n")
    report_lines.append(f"- provider_base_url: `{mask_secret(provider_base_url, keep=12)}`\n")
    report_lines.append(f"- provider_api_key: `<REDACTED {mask_secret(provider_api_key, keep=0)}>`\n")
    report_lines.append(f"- test_model: `{test_model or '(unset)'}`\n")
    report_lines.append("\n")

    if run_chat:
        report_lines.append("> ⚠️ 注意：本次启用了 `BIZ_RUN_CHAT=1`，`/v1/chat/completions` 可能产生费用。\n\n")

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
