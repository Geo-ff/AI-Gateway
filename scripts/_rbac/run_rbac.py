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


def parse_env_file(path: Path) -> dict[str, str]:
    if not path.exists():
        return {}
    env: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.rstrip("\r")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = re.match(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*([=:])\s*(.*)\s*$", line)
        if not m:
            continue
        key, _sep, val = m.group(1), m.group(2), m.group(3)
        if val.startswith('"') and val.endswith('"') and len(val) >= 2:
            val = val[1:-1]
        env[key] = val
    return env


def load_config() -> dict[str, str]:
    env_path = ROOT_DIR / ".env"
    example_path = ROOT_DIR / ".env.example"
    return parse_env_file(env_path) or parse_env_file(example_path)


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
    tmp_file = tempfile.NamedTemporaryFile(prefix="gw_rbac_", suffix=".json", delete=False)
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


def git_sha_short() -> str:
    try:
        proc = run_cmd(["git", "rev-parse", "--short", "HEAD"], timeout_s=5)
        if proc.returncode == 0:
            return proc.stdout.strip()
    except Exception:
        pass
    return "(unknown)"


def response_snippet(body_text: str, *, limit: int = 300) -> str:
    text = (body_text or "").strip()
    if not text:
        return ""
    try:
        parsed = json.loads(text)
        return json.dumps(redact_json(parsed), ensure_ascii=False)[:limit]
    except Exception:
        return redact_text(text[:limit])


JWT_LIKE_RE = re.compile(r"eyJ[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,}\\.[A-Za-z0-9_-]{10,}")


def assert_no_secret_leak(text: str, *, where: str) -> None:
    if "Authorization: Bearer " in text:
        raise RuntimeError(f"secret leak detected in {where}: Authorization header")
    if JWT_LIKE_RE.search(text):
        raise RuntimeError(f"secret leak detected in {where}: JWT-like token")
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
    record = (
        f"- RBAC 边界业务测试（自动化） {stamp}：{status}（Pass={pass_count} / Fail={fail_count} / Total={total}），报告：`{report_display}`\n"
    )

    text = doc_path.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    insert_at = None
    heading_idx = None
    for i, line in enumerate(lines):
        if line.startswith("#### ") and "接口测试记录" in line:
            heading_idx = i
            break
    if heading_idx is None:
        doc_path.write_text(text + "\n" + record, encoding="utf-8")
        return

    for j in range(heading_idx + 1, len(lines)):
        if lines[j].startswith(("#### ", "### ", "## ")):
            insert_at = j
            break
    if insert_at is None:
        insert_at = len(lines)

    if record in text:
        return

    lines.insert(insert_at, record)
    doc_path.write_text("".join(lines), encoding="utf-8")


@dataclass(frozen=True)
class CaseResult:
    role: str
    method: str
    path: str
    expected: int
    actual: int
    passed: bool
    note: str


def main() -> int:
    run_dt = utc_now()
    run_stamp = utc_compact_timestamp(run_dt)
    run_rand = secrets.token_hex(3)
    run_id = f"rbac_{run_stamp}_{run_rand}"

    out_dir = ROOT_DIR / "scripts" / "_rbac"
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / f"{run_id}.md"
    log_path = out_dir / f"{run_id}.log"

    cfg = load_config()
    base_url = (cfg.get("GATEWAY_BASE_URL") or "http://localhost:8080").rstrip("/")
    email = cfg.get("EMAIL") or ""
    password = cfg.get("PASSWORD") or ""
    bootstrap_code = cfg.get("GATEWAY_BOOTSTRAP_CODE") or ""

    log_lines: list[str] = []

    def log(line: str) -> None:
        log_lines.append(redact_text(line))

    if not email or not password:
        msg = "FATAL: missing config from .env/.env.example. Need EMAIL, PASSWORD. (GATEWAY_BASE_URL optional)"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log_path.write_text(msg + "\n", encoding="utf-8")
        return 2

    log("== Gateway Zero RBAC boundary business tests ==")
    log(f"time_utc: {run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}")
    log(f"git_sha : {git_sha_short()}")
    log(f"base_url: {base_url}")
    log(f"email   : {mask_secret(email)}")

    ready_url = f"{base_url}/auth/me"
    log(f"ready_check: curl {ready_url}")
    rc, code = curl_http_code(ready_url, timeout_s=8)
    if rc != 0 or not code or code == "000":
        msg = "FATAL: 无法连接到后端，请先启动数据库与后端：`docker start gateway-postgres` + `cargo run`"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 2
    log(f"ready_check_ok: http_code={code} (401/200 都视为 OK)")

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

    created_users: dict[str, dict[str, str]] = {}
    role_tokens: dict[str, str] = {"superadmin": access_token}

    def create_user(role: str) -> tuple[str, str, str]:
        user_email = f"{run_id}_{role}@example.com"
        user_password = secrets.token_urlsafe(16)
        user_username = f"{run_id}_{role}"
        body = {
            "email": user_email,
            "password": user_password,
            "username": user_username,
            "status": "active",
            "role": role,
        }
        resu = curl_json(base_url=base_url, method="POST", path="/admin/users", bearer=access_token, json_body=body)
        log(f"create_user role={role} status={resu.status_code} body={response_snippet(resu.body_text)}")
        if resu.status_code != 201 or not isinstance(resu.body_json, dict):
            raise RuntimeError(f"create_user failed role={role}: status={resu.status_code} body={response_snippet(resu.body_text)}")
        uid = str(resu.body_json.get("id") or "")
        if not uid:
            raise RuntimeError(f"create_user missing id role={role}")
        created_users[role] = {"id": uid, "email": user_email}
        return uid, user_email, user_password

    def delete_user(user_id: str) -> None:
        try:
            resd = curl_json(base_url=base_url, method="DELETE", path=f"/admin/users/{user_id}", bearer=access_token)
            if resd.status_code in (204, 404):
                log(f"cleanup delete user_id={user_id} status={resd.status_code}")
            else:
                log(f"WARN: cleanup delete user_id={user_id} status={resd.status_code} body={response_snippet(resd.body_text)}")
        except Exception as exc:
            log(f"WARN: cleanup delete user_id={user_id} failed: {exc}")

    try:
        for role in ("admin", "manager", "cashier"):
            _uid, u_email, u_password = create_user(role)
            log(f"created_user role={role} email={u_email} password_len={len(u_password)}")
            lr = login(login_email=u_email, login_password=u_password)
            if lr.status_code != 200 or not isinstance(lr.body_json, dict):
                raise RuntimeError(f"login failed role={role}: status={lr.status_code} body={response_snippet(lr.body_text)}")
            token = str(lr.body_json.get("accessToken") or "")
            if not token:
                raise RuntimeError(f"login response missing accessToken role={role}")
            role_tokens[role] = token
            log(f"{role}_accessToken: {mask_secret(token)}")

        cases: list[tuple[str, str, str, int]] = [
            ("GET", "/auth/me", "auth_me", 200),
            ("GET", "/admin/users", "admin_users", 200),
            ("GET", "/admin/tokens", "admin_tokens", 200),
            ("GET", "/providers", "providers", 200),
            ("GET", "/admin/logs/requests?limit=1", "admin_logs_requests", 200),
            ("GET", "/admin/metrics/summary?window_minutes=60", "admin_metrics_summary", 200),
            ("GET", "/admin/model-prices", "admin_model_prices", 200),
        ]

        results: list[CaseResult] = []
        mismatches: list[CaseResult] = []

        for role in ("superadmin", "admin", "manager", "cashier"):
            bearer = role_tokens.get(role)
            if not bearer:
                raise RuntimeError(f"missing token for role={role}")
            for method, path, _name, exp_superadmin in cases:
                expected = exp_superadmin if role == "superadmin" else (200 if path == "/auth/me" else 403)
                rr = curl_json(base_url=base_url, method=method, path=path, bearer=bearer)
                ok = rr.status_code == expected
                note_parts: list[str] = []
                if expected == 403:
                    shape_ok = ensure_error_shape(rr.body_json)
                    if not shape_ok:
                        note_parts.append("403 body missing {code,message}")
                        ok = False
                snip = response_snippet(rr.body_text)
                if snip:
                    note_parts.append(snip)
                note = " | ".join(note_parts)
                r = CaseResult(
                    role=role,
                    method=method,
                    path=path,
                    expected=expected,
                    actual=rr.status_code,
                    passed=ok,
                    note=note,
                )
                results.append(r)
                if not ok:
                    mismatches.append(r)
                log(f"case role={role} req={method} {path} exp={expected} act={rr.status_code} ok={ok} note={note}")

    finally:
        for info in list(created_users.values()):
            uid = info.get("id") or ""
            if uid:
                delete_user(uid)

    pass_count = sum(1 for r in results if r.passed)
    fail_count = sum(1 for r in results if not r.passed)
    total = len(results)
    conclusion = "PASS" if fail_count == 0 else "FAIL"

    report_lines: list[str] = []
    report_lines.append("# Gateway Zero RBAC 边界业务测试（自动化）\n")
    report_lines.append(f"- time_utc: `{run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}`\n")
    report_lines.append(f"- git_sha: `{git_sha_short()}`\n")
    report_lines.append(f"- base_url: `{base_url}`\n")
    report_lines.append("- 约定（RBAC v1）：仅 `superadmin` 可访问 `/admin/*` 与 `/providers/*`；登录用户可访问 `GET /auth/me`\n")
    if bootstrap_fallback_used:
        report_lines.append(
            "- bootstrap fallback: `YES`（发生副作用：触发 `/auth/register` 创建首个 superadmin 用户）\n"
        )
    else:
        report_lines.append("- bootstrap fallback: `NO`\n")
    report_lines.append("\n")

    report_lines.append("## 矩阵\n\n")
    report_lines.append("| 角色 | 方法 | 端点 | 期望 | 实际 | 结果 | 响应摘要（脱敏） |\n")
    report_lines.append("|---|---|---|---:|---:|---|---|\n")
    for r in results:
        res = "Pass" if r.passed else "Fail"
        report_lines.append(
            f"| `{r.role}` | `{r.method}` | `{r.path}` | `{r.expected}` | `{r.actual}` | **{res}** | {r.note or ''} |\n"
        )

    report_lines.append("\n## 汇总\n\n")
    report_lines.append(f"- Pass={pass_count} / Fail={fail_count} / Total={total}\n")
    report_lines.append(f"- 结论：**{conclusion}**\n")
    if mismatches:
        report_lines.append("- 不一致清单（实现 vs 约定）：\n")
        for r in mismatches:
            report_lines.append(f"  - `{r.role}` `{r.method} {r.path}`：期望 {r.expected}，实际 {r.actual}\n")
    else:
        report_lines.append("- 不一致清单：无\n")

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

    append_workflow_record(report_path=report_path, pass_count=pass_count, fail_count=fail_count, total=total)

    print(f"RBAC report: {report_path}")
    print(f"RBAC log   : {log_path}")
    print(f"Summary    : Pass={pass_count} Fail={fail_count} Total={total} => {conclusion}")
    return 0 if fail_count == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
