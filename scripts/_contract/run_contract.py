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
from typing import Any, Iterable


ROOT_DIR = Path(__file__).resolve().parents[2]
OPENAPI_PATH = ROOT_DIR / "openapi.yaml"


SENSITIVE_JSON_KEYS = {
    "authorization",
    "accesstoken",
    "refreshtoken",
    "password",
    "token",
    "clienttoken",
    "api_key",
    "apikey",
    "key",
    "secret",
    "provider_key",
    # `/providers/{provider}/keys/raw` contains plaintext key under `value`
    "value",
}


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
    # Redact common JSON fields carrying secrets (keep it heuristic & safe).
    text = re.sub(
        r'(?i)"(refreshToken|accessToken|password|token|key|value)"\s*:\s*"[^"]+"',
        lambda m: f"\"{m.group(1)}\": \"***REDACTED***\"",
        text,
    )
    return text


def redact_json(value: Any) -> Any:
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for k, v in value.items():
            if str(k).lower() in SENSITIVE_JSON_KEYS:
                if isinstance(v, str):
                    out[k] = f"***REDACTED*** (len={len(v)})"
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
    env = parse_env_file(env_path) or parse_env_file(example_path)
    return env


@dataclass(frozen=True)
class CurlResult:
    status_code: int
    body_text: str
    body_json: Any | None


def run_cmd(cmd: list[str], *, timeout_s: int = 30) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout_s)


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
    tmp_file = tempfile.NamedTemporaryFile(prefix="gw_contract_", suffix=".json", delete=False)
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
        status_code = int(proc.stdout.strip() or "0")
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


def git_sha_short() -> str:
    try:
        proc = run_cmd(["git", "rev-parse", "--short", "HEAD"], timeout_s=5)
        if proc.returncode == 0:
            return proc.stdout.strip()
    except Exception:
        pass
    return "(unknown)"


ISO8601_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")


def is_iso8601_rfc3339(value: str) -> bool:
    if not value or "T" not in value:
        return False
    if not ISO8601_RE.match(value):
        return False
    try:
        candidate = value.replace("Z", "+00:00")
        datetime.fromisoformat(candidate)
        return True
    except Exception:
        return False


def collect_datetime_property_names(schema: Any) -> set[str]:
    found: set[str] = set()

    def walk(node: Any, *, prop_name: str | None = None) -> None:
        if isinstance(node, dict):
            if node.get("format") == "date-time" and prop_name:
                found.add(prop_name)
            for k, v in node.items():
                if k == "properties" and isinstance(v, dict):
                    for pname, pnode in v.items():
                        walk(pnode, prop_name=pname)
                else:
                    walk(v, prop_name=prop_name)
        elif isinstance(node, list):
            for item in node:
                walk(item, prop_name=prop_name)

    walk(schema)
    return found


def iter_json_key_values(obj: Any) -> Iterable[tuple[str, Any]]:
    if isinstance(obj, dict):
        for k, v in obj.items():
            yield (str(k), v)
            yield from iter_json_key_values(v)
    elif isinstance(obj, list):
        for item in obj:
            yield from iter_json_key_values(item)


def ensure_error_shape(body_json: Any) -> bool:
    return isinstance(body_json, dict) and "code" in body_json and "message" in body_json


def should_ignore_404_on_cleanup(path: str) -> bool:
    return path.startswith("/admin/") or path.startswith("/providers")


def expected_statuses_from_spec(spec: Any, *, path: str, method: str) -> str:
    if not isinstance(spec, dict):
        return "(unknown)"
    paths = spec.get("paths")
    if not isinstance(paths, dict):
        return "(unknown)"
    op = paths.get(path)
    if not isinstance(op, dict):
        return "(unknown)"
    m = op.get(method.lower())
    if not isinstance(m, dict):
        return "(unknown)"
    responses = m.get("responses")
    if not isinstance(responses, dict):
        return "(unknown)"
    keys = [str(k) for k in responses.keys()]
    keys.sort()
    return "|".join(keys) if keys else "(none)"


def response_snippet(body: str | None, *, limit: int = 500) -> str | None:
    if not body:
        return None
    text = body.strip()
    if not text:
        return None
    try:
        parsed = json.loads(text)
        return json.dumps(redact_json(parsed), ensure_ascii=False)[:limit]
    except Exception:
        return redact_text(text[: min(limit, 200)])


def requests_response_from_curl(
    *,
    base_url: str,
    method: str,
    path: str,
    status_code: int,
    body_text: str,
    headers: dict[str, str] | None = None,
) -> Any:
    import requests
    from requests.structures import CaseInsensitiveDict

    url = f"{base_url}{path}"
    resp = requests.Response()
    resp.status_code = status_code
    resp._content = (body_text or "").encode("utf-8", errors="replace")
    resp.url = url
    resp.encoding = "utf-8"
    resp.headers = CaseInsensitiveDict(headers or {"Content-Type": "application/json"})
    # Attach a minimal request object to keep downstream code happy
    req = requests.Request(method=method, url=url, headers=headers or {})
    resp.request = req.prepare()
    return resp


def get_operation(schema: Any, *, method: str, path: str) -> Any:
    want_m = method.lower()
    for res in schema.get_all_operations():
        op = res.ok()
        if op.method == want_m and op.path == path:
            return op
    raise KeyError(f"operation not found in schema: {method} {path}")


def validate_openapi_response(
    *,
    schema: Any,
    base_url: str,
    method: str,
    path_template: str,
    path_parameters: dict[str, str] | None,
    request_body: Any | None,
    status_code: int,
    body_text: str,
    exclude_ignored_auth: bool = True,
) -> None:
    import schemathesis
    from schemathesis.exceptions import CheckFailed

    op = get_operation(schema, method=method, path=path_template)
    make_case_kwargs: dict[str, Any] = {
        "path_parameters": path_parameters or None,
        "headers": {"Accept": "application/json"},
        "cookies": {},
        "query": None,
    }
    if request_body is not None:
        make_case_kwargs["body"] = request_body
        make_case_kwargs["media_type"] = "application/json"
    case = op.make_case(**make_case_kwargs)
    resp = requests_response_from_curl(
        base_url=base_url,
        method=method,
        path=case.formatted_path,
        status_code=status_code,
        body_text=body_text,
    )
    excluded = ()
    if exclude_ignored_auth and hasattr(schemathesis, "checks"):
        excluded = (schemathesis.checks.ignored_auth,)
    try:
        case.validate_response(resp, excluded_checks=excluded)
    except Exception as exc:
        raise CheckFailed(str(exc))


@dataclass
class Failure:
    method: str
    path: str
    reason: str
    status_code: int | None = None
    expected: str | None = None
    response_snippet: str | None = None


def main() -> int:
    run_dt = utc_now()
    run_stamp = utc_compact_timestamp(run_dt)
    run_rand = secrets.token_hex(3)
    profile = (os.getenv("CONTRACT_PROFILE", "read") or "read").strip().lower()
    is_write_profile = profile == "write"
    run_prefix = "contractw" if is_write_profile else "contract"
    run_id = f"{run_prefix}_{run_stamp}_{run_rand}"

    out_dir = ROOT_DIR / "scripts" / "_contract"
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / f"{run_id}.md"
    log_path = out_dir / f"{run_id}.log"

    seed = int(os.getenv("CONTRACT_SEED", os.getenv("HYPOTHESIS_SEED", "20260112")))
    max_examples = int(os.getenv("CONTRACT_MAX_EXAMPLES", "20"))

    cfg = load_config()
    base_url = (cfg.get("GATEWAY_BASE_URL") or "").rstrip("/")
    email = cfg.get("EMAIL") or ""
    password = cfg.get("PASSWORD") or ""
    bootstrap_code = cfg.get("GATEWAY_BOOTSTRAP_CODE") or ""

    if not base_url or not email or not password:
        msg = "FATAL: missing config from .env/.env.example. Need GATEWAY_BASE_URL, EMAIL, PASSWORD."
        report_path.write_text(msg + "\n", encoding="utf-8")
        log_path.write_text(msg + "\n", encoding="utf-8")
        return 2

    os.environ["HYPOTHESIS_SEED"] = str(seed)
    os.environ.setdefault("SCHEMATHESIS_DISABLE_REPORTING", "1")

    failures: list[Failure] = []
    log_lines: list[str] = []
    bootstrap_fallback_used = False

    def log(line: str) -> None:
        log_lines.append(redact_text(line))

    log("== Gateway Zero OpenAPI contract (schemathesis) ==")
    log(f"time_utc: {run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}")
    log(f"git_sha : {git_sha_short()}")
    log(f"base_url: {base_url}")
    log(f"email   : {mask_secret(email)}")
    log(f"seed    : {seed}")
    log(f"max_ex  : {max_examples}")
    log(f"profile : {profile}")

    access_token: str | None = None

    # --- Auth bootstrap (manual; not fuzzed) ---
    try:
        res = curl_json(
            base_url=base_url,
            method="POST",
            path="/auth/login",
            json_body={"email": email, "password": password},
            timeout_s=30,
        )
        if res.status_code == 200 and isinstance(res.body_json, dict) and isinstance(res.body_json.get("accessToken"), str):
            access_token = res.body_json["accessToken"]
            log("auth: POST /auth/login -> 200 (accessToken acquired)")
        elif res.status_code == 401:
            log("auth: POST /auth/login -> 401 (will consider bootstrap fallback)")
            if not bootstrap_code:
                failures.append(
                    Failure(
                        method="POST",
                        path="/auth/login",
                        reason="login 401 and missing GATEWAY_BOOTSTRAP_CODE for bootstrap fallback",
                        status_code=401,
                        expected="200",
                        response_snippet=json.dumps(redact_json(res.body_json), ensure_ascii=False)[:500]
                        if res.body_json is not None
                        else redact_text(res.body_text[:200]),
                    )
                )
            else:
                reg = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/auth/register",
                    json_body={"bootstrap_code": bootstrap_code, "email": email, "password": password},
                    timeout_s=30,
                )
                if reg.status_code == 201:
                    bootstrap_fallback_used = True
                    log("auth: POST /auth/register -> 201 (bootstrap fallback used)")
                else:
                    log(f"auth: POST /auth/register -> {reg.status_code} (no bootstrap created)")

                res2 = curl_json(
                    base_url=base_url,
                    method="POST",
                    path="/auth/login",
                    json_body={"email": email, "password": password},
                    timeout_s=30,
                )
                if res2.status_code == 200 and isinstance(res2.body_json, dict) and isinstance(res2.body_json.get("accessToken"), str):
                    access_token = res2.body_json["accessToken"]
                    log("auth: POST /auth/login (retry) -> 200 (accessToken acquired)")
                else:
                    failures.append(
                        Failure(
                            method="POST",
                            path="/auth/login",
                            reason="login failed after bootstrap attempt",
                            status_code=res2.status_code,
                            expected="200",
                            response_snippet=json.dumps(redact_json(res2.body_json), ensure_ascii=False)[:500]
                            if res2.body_json is not None
                            else redact_text(res2.body_text[:200]),
                        )
                    )
        else:
            failures.append(
                Failure(
                    method="POST",
                    path="/auth/login",
                    reason="unexpected login response",
                    status_code=res.status_code,
                    expected="200",
                    response_snippet=json.dumps(redact_json(res.body_json), ensure_ascii=False)[:500]
                    if res.body_json is not None
                    else redact_text(res.body_text[:200]),
                )
            )
    except Exception as e:
        failures.append(Failure(method="POST", path="/auth/login", reason=f"auth step failed: {e}"))

    # --- Manual error shape sampling (401/403/404) ---
    error_shape_samples: dict[int, bool] = {401: False, 403: False, 404: False}

    # 401 sample: /auth/me without auth
    try:
        me_unauth = curl_json(base_url=base_url, method="GET", path="/auth/me", timeout_s=30)
        ok = me_unauth.status_code == 401 and ensure_error_shape(me_unauth.body_json)
        error_shape_samples[401] = ok
        log(f"sample: GET /auth/me (no auth) -> {me_unauth.status_code} (error shape ok={ok})")
        if not ok:
            failures.append(
                Failure(
                    method="GET",
                    path="/auth/me",
                    reason="401 error body missing {code,message}",
                    status_code=me_unauth.status_code,
                    expected="401",
                    response_snippet=json.dumps(redact_json(me_unauth.body_json), ensure_ascii=False)[:500]
                    if me_unauth.body_json is not None
                    else redact_text(me_unauth.body_text[:200]),
                )
            )
    except Exception as e:
        failures.append(Failure(method="GET", path="/auth/me", reason=f"401 sampling failed: {e}"))

    # If we don't have an access token, we can't proceed to admin/providers schema runs.
    if not access_token:
        report = "\n".join(
            [
                f"# OpenAPI Contract Test Report ({run_id})",
                "",
                f"- time_utc: {run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}",
                f"- base_url: {base_url}",
                f"- git: {git_sha_short()}",
                f"- seed: {seed}",
                f"- max_examples: {max_examples}",
                f"- schemathesis: (not executed; auth failed)",
                f"- bootstrap_fallback: {'YES' if bootstrap_fallback_used else 'NO'}",
                "",
                "## Summary",
                f"- Pass=0 Fail={len(failures)}",
                "",
                "## Failures",
                *[
                    f"- {f.method} {f.path} :: {f.reason} :: status={f.status_code} expected={f.expected} :: snippet={f.response_snippet}"
                    for f in failures
                ],
            ]
        )
        report_path.write_text(report + "\n", encoding="utf-8")
        log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")
        return 1

    # 404 sample: /admin/users/{id} not found
    try:
        missing_id = f"{run_id}_not_found"
        r404 = curl_json(
            base_url=base_url,
            method="GET",
            path=f"/admin/users/{missing_id}",
            bearer=access_token,
            timeout_s=30,
        )
        ok = r404.status_code == 404 and ensure_error_shape(r404.body_json)
        error_shape_samples[404] = ok
        log(f"sample: GET /admin/users/{{id}} (missing) -> {r404.status_code} (error shape ok={ok})")
        if not ok:
            failures.append(
                Failure(
                    method="GET",
                    path="/admin/users/{id}",
                    reason="404 error body missing {code,message}",
                    status_code=r404.status_code,
                    expected="404",
                    response_snippet=json.dumps(redact_json(r404.body_json), ensure_ascii=False)[:500]
                    if r404.body_json is not None
                    else redact_text(r404.body_text[:200]),
                )
            )
    except Exception as e:
        failures.append(Failure(method="GET", path="/admin/users/{id}", reason=f"404 sampling failed: {e}"))

    # 403 sample: create cashier user (limited write) -> login -> access admin list -> expect 403 -> cleanup
    try:
        prefix = run_id
        cashier_email = f"{prefix}_cashier@example.com"
        cashier_username = f"{prefix}_cashier"
        cashier_password = secrets.token_urlsafe(18)

        created_user_id: str | None = None
        created = curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/users",
            bearer=access_token,
            json_body={
                "username": cashier_username,
                "email": cashier_email,
                "password": cashier_password,
                "role": "cashier",
                "status": "active",
            },
            timeout_s=30,
        )
        if created.status_code == 201 and isinstance(created.body_json, dict) and isinstance(created.body_json.get("id"), str):
            created_user_id = created.body_json["id"]
            log("sample: POST /admin/users -> 201 (rbac sample user created)")
        else:
            log(f"sample: POST /admin/users -> {created.status_code} (rbac sample user not created)")

        cashier_token: str | None = None
        if created_user_id:
            login_cashier = curl_json(
                base_url=base_url,
                method="POST",
                path="/auth/login",
                json_body={"email": cashier_email, "password": cashier_password},
                timeout_s=30,
            )
            if (
                login_cashier.status_code == 200
                and isinstance(login_cashier.body_json, dict)
                and isinstance(login_cashier.body_json.get("accessToken"), str)
            ):
                cashier_token = login_cashier.body_json["accessToken"]

        if cashier_token:
            r = curl_json(base_url=base_url, method="GET", path="/admin/users", bearer=cashier_token, timeout_s=30)
            ok = r.status_code == 403 and ensure_error_shape(r.body_json)
            error_shape_samples[403] = ok
            log(f"sample: GET /admin/users (cashier) -> {r.status_code} (error shape ok={ok})")
            if not ok:
                failures.append(
                    Failure(
                        method="GET",
                        path="/admin/users",
                        reason="403 error body missing {code,message} (cashier token)",
                        status_code=r.status_code,
                        expected="403",
                        response_snippet=json.dumps(redact_json(r.body_json), ensure_ascii=False)[:500]
                        if r.body_json is not None
                        else redact_text(r.body_text[:200]),
                    )
                )
        else:
            failures.append(
                Failure(
                    method="GET",
                    path="/admin/users",
                    reason="unable to acquire cashier token for 403 sampling",
                )
            )
    finally:
        # Best-effort cleanup
        try:
            if "created_user_id" in locals() and locals().get("created_user_id"):
                _ = curl_json(
                    base_url=base_url,
                    method="DELETE",
                    path=f"/admin/users/{locals()['created_user_id']}",
                    bearer=access_token,
                    timeout_s=30,
                )
        except Exception:
            pass

    # --- Write profile fixtures + deterministic write+cleanup ---
    fixtures: dict[str, str] = {}
    created_provider_key: str | None = None
    created_user_password: str | None = None
    write_total = 0
    write_passed = 0
    write_failed = 0

    def write_step(ok: bool) -> None:
        nonlocal write_total, write_passed, write_failed
        write_total += 1
        if ok:
            write_passed += 1
        else:
            write_failed += 1

    if is_write_profile:
        prefix = f"contractw_{run_stamp}"
        fixtures["prefix"] = prefix

        provider_name = f"{prefix}_p"
        user_email = f"{prefix}_u@example.com"
        user_username = f"{prefix}_u"
        created_user_password = secrets.token_urlsafe(18)
        token_name = f"{prefix}_t"
        provider_key_value = f"{prefix}_k_{secrets.token_urlsafe(12)}"

        fixtures["provider_name"] = provider_name
        fixtures["user_email"] = user_email
        fixtures["user_username"] = user_username
        fixtures["token_name"] = token_name

        log(f"fixtures: prefix={mask_secret(prefix)} provider={mask_secret(provider_name)} user_email={mask_secret(user_email)}")

        try:
            import schemathesis
            schema_for_validation = schemathesis.from_path(str(OPENAPI_PATH), base_url=base_url)

            # Provider fixture
            r = curl_json(
                base_url=base_url,
                method="POST",
                path="/providers",
                bearer=access_token,
                json_body={"name": provider_name, "api_type": "openai", "base_url": "https://example.org", "models_endpoint": None},
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="POST",
                path_template="/providers",
                path_parameters=None,
                request_body={"name": provider_name, "api_type": "openai", "base_url": "https://example.org", "models_endpoint": None},
                status_code=r.status_code,
                body_text=r.body_text,
            )
            write_step(True)

            # User fixture
            u = curl_json(
                base_url=base_url,
                method="POST",
                path="/admin/users",
                bearer=access_token,
                json_body={
                    "username": user_username,
                    "email": user_email,
                    "password": created_user_password,
                    "role": "cashier",
                    "status": "active",
                },
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="POST",
                path_template="/admin/users",
                path_parameters=None,
                request_body={
                    "username": user_username,
                    "email": user_email,
                    "password": "***REDACTED***",
                    "role": "cashier",
                    "status": "active",
                },
                status_code=u.status_code,
                body_text=u.body_text,
            )
            write_step(True)
            if isinstance(u.body_json, dict) and isinstance(u.body_json.get("id"), str):
                fixtures["user_id"] = u.body_json["id"]

            # Token fixture
            t = curl_json(
                base_url=base_url,
                method="POST",
                path="/admin/tokens",
                bearer=access_token,
                json_body={"name": token_name, "enabled": True},
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="POST",
                path_template="/admin/tokens",
                path_parameters=None,
                request_body={"name": token_name, "enabled": True},
                status_code=t.status_code,
                body_text=t.body_text,
            )
            write_step(True)
            if isinstance(t.body_json, dict) and isinstance(t.body_json.get("id"), str):
                fixtures["token_id"] = t.body_json["id"]

            # Provider key fixture (do not log raw key)
            created_provider_key = provider_key_value
            pk = curl_json(
                base_url=base_url,
                method="POST",
                path=f"/providers/{provider_name}/keys",
                bearer=access_token,
                json_body={"key": provider_key_value},
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="POST",
                path_template="/providers/{provider}/keys",
                path_parameters={"provider": provider_name},
                request_body={"key": "***REDACTED***"},
                status_code=pk.status_code,
                body_text=pk.body_text,
            )
            write_step(True)
        except Exception as e:
            failures.append(Failure(method="FIXTURE", path="*", reason=f"fixture setup failed: {e}"))
            write_step(False)

        # Minimal write chain + read-back confirmations (best-effort)
        try:
            import schemathesis
            schema_for_validation = schemathesis.from_path(str(OPENAPI_PATH), base_url=base_url)

            if "user_id" in fixtures:
                new_username = f"{prefix}_u_upd"
                r_put = curl_json(
                    base_url=base_url,
                    method="PUT",
                    path=f"/admin/users/{fixtures['user_id']}",
                    bearer=access_token,
                    json_body={"username": new_username, "status": "active"},
                    timeout_s=30,
                )
                validate_openapi_response(
                    schema=schema_for_validation,
                    base_url=base_url,
                    method="PUT",
                    path_template="/admin/users/{id}",
                    path_parameters={"id": fixtures["user_id"]},
                    request_body={"username": new_username, "status": "active"},
                    status_code=r_put.status_code,
                    body_text=r_put.body_text,
                )
                write_step(True)
                r_get = curl_json(
                    base_url=base_url,
                    method="GET",
                    path=f"/admin/users/{fixtures['user_id']}",
                    bearer=access_token,
                    timeout_s=30,
                )
                validate_openapi_response(
                    schema=schema_for_validation,
                    base_url=base_url,
                    method="GET",
                    path_template="/admin/users/{id}",
                    path_parameters={"id": fixtures["user_id"]},
                    request_body=None,
                    status_code=r_get.status_code,
                    body_text=r_get.body_text,
                )
                write_step(True)

            if "token_id" in fixtures:
                new_token_name = f"{prefix}_t_upd"
                r_put = curl_json(
                    base_url=base_url,
                    method="PUT",
                    path=f"/admin/tokens/{fixtures['token_id']}",
                    bearer=access_token,
                    json_body={"name": new_token_name},
                    timeout_s=30,
                )
                validate_openapi_response(
                    schema=schema_for_validation,
                    base_url=base_url,
                    method="PUT",
                    path_template="/admin/tokens/{id}",
                    path_parameters={"id": fixtures["token_id"]},
                    request_body={"name": new_token_name},
                    status_code=r_put.status_code,
                    body_text=r_put.body_text,
                )
                write_step(True)
                r_toggle = curl_json(
                    base_url=base_url,
                    method="POST",
                    path=f"/admin/tokens/{fixtures['token_id']}/toggle",
                    bearer=access_token,
                    json_body={"enabled": False},
                    timeout_s=30,
                )
                validate_openapi_response(
                    schema=schema_for_validation,
                    base_url=base_url,
                    method="POST",
                    path_template="/admin/tokens/{id}/toggle",
                    path_parameters={"id": fixtures["token_id"]},
                    request_body={"enabled": False},
                    status_code=r_toggle.status_code,
                    body_text=r_toggle.body_text,
                )
                write_step(True)
                r_get = curl_json(
                    base_url=base_url,
                    method="GET",
                    path=f"/admin/tokens/{fixtures['token_id']}",
                    bearer=access_token,
                    timeout_s=30,
                )
                validate_openapi_response(
                    schema=schema_for_validation,
                    base_url=base_url,
                    method="GET",
                    path_template="/admin/tokens/{id}",
                    path_parameters={"id": fixtures["token_id"]},
                    request_body=None,
                    status_code=r_get.status_code,
                    body_text=r_get.body_text,
                )
                write_step(True)

            # Providers chain
            r_put = curl_json(
                base_url=base_url,
                method="PUT",
                path=f"/providers/{provider_name}",
                bearer=access_token,
                json_body={"api_type": "openai", "base_url": "https://example.org", "models_endpoint": "https://example.org/models"},
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="PUT",
                path_template="/providers/{provider}",
                path_parameters={"provider": provider_name},
                request_body={"api_type": "openai", "base_url": "https://example.org", "models_endpoint": "https://example.org/models"},
                status_code=r_put.status_code,
                body_text=r_put.body_text,
            )
            write_step(True)
            r_get = curl_json(
                base_url=base_url,
                method="GET",
                path=f"/providers/{provider_name}",
                bearer=access_token,
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="GET",
                path_template="/providers/{provider}",
                path_parameters={"provider": provider_name},
                request_body=None,
                status_code=r_get.status_code,
                body_text=r_get.body_text,
            )
            write_step(True)

            # Provider keys chain (structure only; response redaction handles `value`)
            r_keys = curl_json(
                base_url=base_url,
                method="GET",
                path=f"/providers/{provider_name}/keys",
                bearer=access_token,
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="GET",
                path_template="/providers/{provider}/keys",
                path_parameters={"provider": provider_name},
                request_body=None,
                status_code=r_keys.status_code,
                body_text=r_keys.body_text,
            )
            write_step(True)
            r_raw = curl_json(
                base_url=base_url,
                method="GET",
                path=f"/providers/{provider_name}/keys/raw",
                bearer=access_token,
                timeout_s=30,
            )
            validate_openapi_response(
                schema=schema_for_validation,
                base_url=base_url,
                method="GET",
                path_template="/providers/{provider}/keys/raw",
                path_parameters={"provider": provider_name},
                request_body=None,
                status_code=r_raw.status_code,
                body_text=r_raw.body_text,
            )
            write_step(True)
        except Exception as e:
            failures.append(Failure(method="WRITE", path="*", reason=f"write chain validation failed: {e}"))
            write_step(False)

    # --- Schema-based contract fuzz (profile-dependent; fixture injected) ---
    schema_total = 0
    schema_passed = 0
    schema_failed = 0
    schema_skipped = 0
    schemathesis_version = "(unknown)"
    datetime_checked = 0
    datetime_invalid = 0
    included_ops: list[str] = []

    try:
        import schemathesis
        from schemathesis import hooks
        from hypothesis import settings as hypo_settings
        from schemathesis.exceptions import CheckFailed
        from schemathesis.runner import events
        from schemathesis.runner.serialization import Status

        schemathesis_version = getattr(schemathesis, "__version__", "(unknown)")

        schema = schemathesis.from_path(str(OPENAPI_PATH), base_url=base_url)
        spec = schema.raw_schema if hasattr(schema, "raw_schema") else None
        datetime_keys = collect_datetime_property_names(spec) if isinstance(spec, dict) else set()

        def operation_filter(op: Any) -> bool:
            operation = getattr(op, "operation", op)
            path = getattr(operation, "path", "")
            method = str(getattr(operation, "method", "")).upper()
            if path.startswith("/v1/"):
                return False
            if path in {"/auth/login", "/auth/refresh", "/auth/register"}:
                return False
            # include 仍限定：/auth/me、/admin/users*、/admin/tokens*、/providers*、/providers/*/keys*
            in_scope = (
                path == "/auth/me"
                or path.startswith("/admin/users")
                or path.startswith("/admin/tokens")
                or path.startswith("/providers")
            )
            if not in_scope:
                return False

            if not is_write_profile:
                return method in {"GET", "HEAD"}

            # write profile: allow GET/HEAD + minimal safe writes (no create endpoints)
            if method in {"GET", "HEAD"}:
                return True
            # allowlisted writes
            if method == "PUT" and path in {"/admin/users/{id}", "/admin/tokens/{id}", "/providers/{provider}"}:
                return True
            if method == "POST" and path == "/admin/tokens/{id}/toggle":
                return True
            return False

        # Ensure we don't accidentally accumulate global hooks across multiple runs.
        hooks.unregister_all()

        @hooks.register("before_call")
        def _force_admin_bearer_only(context: Any, case: Any) -> None:
            # Avoid Schemathesis trying other auth mechanisms (cookie / TUI session),
            # which may trigger 400s on malformed Cookie headers in some setups.
            if case.headers is None:
                case.headers = {}
            case.headers.pop("Cookie", None)
            case.headers.pop("cookie", None)
            case.cookies = {}
            case.headers["Accept"] = "application/json"
            case.headers["Authorization"] = f"Bearer {access_token}"

            # Fixture injection for path parameters (write profile requires 2xx success paths)
            if is_write_profile and fixtures:
                if case.path_parameters is None:
                    case.path_parameters = {}
                if "id" in (case.path_parameters or {}):
                    if case.path.startswith("/admin/users/") and "user_id" in fixtures:
                        case.path_parameters["id"] = fixtures["user_id"]
                    if case.path.startswith("/admin/tokens/") and "token_id" in fixtures:
                        case.path_parameters["id"] = fixtures["token_id"]
                if "provider" in (case.path_parameters or {}) and "provider_name" in fixtures:
                    case.path_parameters["provider"] = fixtures["provider_name"]

                # Safe minimal bodies for allowlisted writes
                if case.method.upper() == "PUT" and case.path == "/admin/users/{id}":
                    case.body = {"username": f"{fixtures['prefix']}_u_upd2", "status": "active"}
                    case.media_type = "application/json"
                elif case.method.upper() == "PUT" and case.path == "/admin/tokens/{id}":
                    case.body = {"name": f"{fixtures['prefix']}_t_upd2"}
                    case.media_type = "application/json"
                elif case.method.upper() == "POST" and case.path == "/admin/tokens/{id}/toggle":
                    case.body = {"enabled": True}
                    case.media_type = "application/json"
                elif case.method.upper() == "PUT" and case.path == "/providers/{provider}":
                    case.body = {"api_type": "openai", "base_url": "https://example.org", "models_endpoint": None}
                    case.media_type = "application/json"

        schema = schema.include(func=operation_filter)

        for res in schema.get_all_operations():
            op = res.ok()
            included_ops.append(f"{op.method.upper()} {op.path}")

        def openapi_contract_check(ctx: Any, response: Any, case: Any) -> bool | None:
            try:
                # We always attach Authorization in `before_call`; disable the "ignored auth"
                # check to avoid false failures when Schemathesis tries auth-negative probes.
                excluded = (schemathesis.checks.ignored_auth,) if hasattr(schemathesis, "checks") else ()
                case.validate_response(response, excluded_checks=excluded)
            except Exception as exc:
                raise CheckFailed(str(exc))
            return True

        def datetime_sampling_check(ctx: Any, response: Any, case: Any) -> bool | None:
            nonlocal datetime_checked, datetime_invalid
            if not datetime_keys:
                return None
            try:
                payload = response.json()
            except Exception:
                return None
            if not isinstance(payload, (dict, list)):
                return None

            # Keep it fast & deterministic: validate up to 30 date-time fields per run.
            for key, value in iter_json_key_values(payload):
                if key in datetime_keys and isinstance(value, str):
                    datetime_checked += 1
                    if not is_iso8601_rfc3339(value):
                        datetime_invalid += 1
                        raise CheckFailed(
                            f"date-time field '{key}' is not RFC3339/ISO-8601 (masked={mask_secret(value, keep=12)})"
                        )
                    if datetime_checked >= 30:
                        break
            return True

        runner = schemathesis.runner.from_schema(
            schema,
            checks=(openapi_contract_check, datetime_sampling_check),
            hypothesis_settings=hypo_settings(max_examples=max_examples, deadline=None),
            seed=seed,
            workers_num=1,
            request_timeout=30,
        )

        for event in runner.execute():
            if not isinstance(event, events.AfterExecution):
                continue
            schema_total += 1
            result = event.result
            method = event.method.upper()
            path = event.relative_path

            if result.is_skipped:
                schema_skipped += 1
                continue

            if result.has_failures or result.has_errors or result.is_errored:
                schema_failed += 1
                expected = expected_statuses_from_spec(spec, path=path, method=method)
                actual_status: int | None = None
                snippet: str | None = None

                reasons: list[str] = []
                for check in result.checks:
                    if check.value != Status.success:
                        title = getattr(check, "title", None) or check.name
                        msg = (check.message or "").strip().splitlines()[0] if (check.message or "").strip() else ""
                        reasons.append(f"{title}: {msg}".strip())
                        if actual_status is None and check.response is not None:
                            actual_status = check.response.status_code
                            try:
                                body_bytes = check.response.deserialize_body()
                                body_text = (
                                    body_bytes.decode(check.response.encoding or "utf-8", errors="replace")
                                    if body_bytes is not None
                                    else None
                                )
                            except Exception:
                                body_text = check.response.body
                            snippet = response_snippet(body_text)

                for err in result.errors:
                    reasons.append(f"error: {err.type.value}{(' ' + err.message) if err.message else ''}".strip())

                reason = redact_text(" | ".join(reasons) or "contract validation failed")[:400]
                failures.append(
                    Failure(
                        method=method,
                        path=path,
                        reason=reason,
                        status_code=actual_status,
                        expected=expected,
                        response_snippet=snippet,
                    )
                )
                log(f"FAIL: {method} {path} status={actual_status} expected={expected} reason={reason}")
            else:
                schema_passed += 1

        hooks.unregister(_force_admin_bearer_only)

    except Exception as e:
        failures.append(Failure(method="SCHEMA", path="*", reason=f"schemathesis execution failed: {e}"))

    # --- Cleanup (write profile) ---
    if is_write_profile and access_token:
        try:
            import schemathesis
            schema_for_validation = schemathesis.from_path(str(OPENAPI_PATH), base_url=base_url)

            # Provider key
            if fixtures.get("provider_name") and created_provider_key:
                r = curl_json(
                    base_url=base_url,
                    method="DELETE",
                    path=f"/providers/{fixtures['provider_name']}/keys",
                    bearer=access_token,
                    json_body={"key": created_provider_key},
                    timeout_s=30,
                )
                if r.status_code in (200, 404):
                    validate_openapi_response(
                        schema=schema_for_validation,
                        base_url=base_url,
                        method="DELETE",
                        path_template="/providers/{provider}/keys",
                        path_parameters={"provider": fixtures["provider_name"]},
                        request_body={"key": "***REDACTED***"},
                        status_code=r.status_code,
                        body_text=r.body_text,
                    )
                    write_step(True)
                else:
                    failures.append(
                        Failure(
                            method="CLEANUP",
                            path="/providers/{provider}/keys",
                            reason="cleanup provider key returned unexpected status",
                            status_code=r.status_code,
                            expected="200|404",
                            response_snippet=response_snippet(r.body_text),
                        )
                    )
                    write_step(False)
        except Exception as e:
            failures.append(Failure(method="CLEANUP", path="/providers/{provider}/keys", reason=f"cleanup provider key failed: {e}"))
            write_step(False)

        try:
            # Provider
            if fixtures.get("provider_name"):
                r = curl_json(
                    base_url=base_url,
                    method="DELETE",
                    path=f"/providers/{fixtures['provider_name']}",
                    bearer=access_token,
                    timeout_s=30,
                )
                if r.status_code in (200, 404):
                    validate_openapi_response(
                        schema=schema_for_validation,
                        base_url=base_url,
                        method="DELETE",
                        path_template="/providers/{provider}",
                        path_parameters={"provider": fixtures["provider_name"]},
                        request_body=None,
                        status_code=r.status_code,
                        body_text=r.body_text,
                    )
                    write_step(True)
                else:
                    failures.append(
                        Failure(
                            method="CLEANUP",
                            path="/providers/{provider}",
                            reason="cleanup provider returned unexpected status",
                            status_code=r.status_code,
                            expected="200|404",
                            response_snippet=response_snippet(r.body_text),
                        )
                    )
                    write_step(False)
        except Exception as e:
            failures.append(Failure(method="CLEANUP", path="/providers/{provider}", reason=f"cleanup provider failed: {e}"))
            write_step(False)

        try:
            # Token
            if fixtures.get("token_id"):
                r = curl_json(
                    base_url=base_url,
                    method="DELETE",
                    path=f"/admin/tokens/{fixtures['token_id']}",
                    bearer=access_token,
                    timeout_s=30,
                )
                if r.status_code in (204, 404):
                    validate_openapi_response(
                        schema=schema_for_validation,
                        base_url=base_url,
                        method="DELETE",
                        path_template="/admin/tokens/{id}",
                        path_parameters={"id": fixtures["token_id"]},
                        request_body=None,
                        status_code=r.status_code,
                        body_text=r.body_text,
                    )
                    write_step(True)
                else:
                    failures.append(
                        Failure(
                            method="CLEANUP",
                            path="/admin/tokens/{id}",
                            reason="cleanup token returned unexpected status",
                            status_code=r.status_code,
                            expected="204|404",
                            response_snippet=response_snippet(r.body_text),
                        )
                    )
                    write_step(False)
        except Exception as e:
            failures.append(Failure(method="CLEANUP", path="/admin/tokens/{id}", reason=f"cleanup token failed: {e}"))
            write_step(False)

        try:
            # User
            if fixtures.get("user_id"):
                r = curl_json(
                    base_url=base_url,
                    method="DELETE",
                    path=f"/admin/users/{fixtures['user_id']}",
                    bearer=access_token,
                    timeout_s=30,
                )
                if r.status_code in (204, 404):
                    validate_openapi_response(
                        schema=schema_for_validation,
                        base_url=base_url,
                        method="DELETE",
                        path_template="/admin/users/{id}",
                        path_parameters={"id": fixtures["user_id"]},
                        request_body=None,
                        status_code=r.status_code,
                        body_text=r.body_text,
                    )
                    write_step(True)
                else:
                    failures.append(
                        Failure(
                            method="CLEANUP",
                            path="/admin/users/{id}",
                            reason="cleanup user returned unexpected status",
                            status_code=r.status_code,
                            expected="204|404",
                            response_snippet=response_snippet(r.body_text),
                        )
                    )
                    write_step(False)
        except Exception as e:
            failures.append(Failure(method="CLEANUP", path="/admin/users/{id}", reason=f"cleanup user failed: {e}"))
            write_step(False)

    # --- Report ---
    fail_count = len(failures)
    overall_pass = fail_count == 0

    report_lines: list[str] = []
    report_lines.append(f"# OpenAPI Contract Test Report ({run_id})")
    report_lines.append("")
    report_lines.append(f"- 运行时间(UTC)：{run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}")
    report_lines.append(f"- BASE_URL：{base_url}")
    report_lines.append(f"- git：{git_sha_short()}")
    report_lines.append(f"- schemathesis：{schemathesis_version}")
    report_lines.append(f"- seed：{seed}")
    report_lines.append(f"- max_examples：{max_examples}")
    report_lines.append(f"- profile：{'write+cleanup (fixture injected)' if is_write_profile else 'read (GET/HEAD only)'}")
    report_lines.append(
        "- 过滤策略：include=/auth/me + /admin/* + /providers*；methods=GET/HEAD；skip=/auth/login,/auth/refresh,/auth/register,/v1/*"
    )
    report_lines.append(f"- bootstrap fallback：{'YES' if bootstrap_fallback_used else 'NO'}")
    if is_write_profile and fixtures:
        report_lines.append(
            f"- fixtures：prefix={mask_secret(fixtures.get('prefix'))} provider={mask_secret(fixtures.get('provider_name'))} user_id={mask_secret(fixtures.get('user_id'))} token_id={mask_secret(fixtures.get('token_id'))}"
        )
    report_lines.append("")
    report_lines.append("## Summary")
    report_lines.append(
        f"- schema: Pass={schema_passed} / Fail={schema_failed} / Skip={schema_skipped} / Total={schema_total}"
    )
    report_lines.append(f"- date-time sampled: checked={datetime_checked} invalid={datetime_invalid}")
    if is_write_profile:
        report_lines.append(f"- write_chain: Pass={write_passed} / Fail={write_failed} / Total={write_total}")
    report_lines.append(f"- overall: {'Pass' if overall_pass else 'Fail'} (Fail={fail_count})")
    report_lines.append("")
    report_lines.append("## Error Body Samples ({code,message})")
    report_lines.append(f"- 401：{'OK' if error_shape_samples[401] else 'MISSING'}")
    report_lines.append(f"- 403：{'OK' if error_shape_samples[403] else 'MISSING'}")
    report_lines.append(f"- 404：{'OK' if error_shape_samples[404] else 'MISSING'}")
    report_lines.append("")

    report_lines.append("## Coverage")
    report_lines.append(f"- operations_included: {len(included_ops)}")
    if included_ops:
        report_lines.append("- operations (methods+paths):")
        for op in sorted(included_ops):
            report_lines.append(f"  - {op}")
    report_lines.append("")

    if failures:
        report_lines.append("## Failures")
        for f in failures:
            line = f"- {f.method} {f.path} :: {f.reason}"
            if f.status_code is not None or f.expected is not None:
                line += f" :: status={f.status_code} expected={f.expected}"
            if f.response_snippet:
                line += f" :: snippet={f.response_snippet}"
            report_lines.append(redact_text(line))
    else:
        report_lines.append("## Failures")
        report_lines.append("- (none)")

    report_lines.append("")
    report_lines.append("## Artifacts")
    report_lines.append(f"- report: {report_path.as_posix()}")
    report_lines.append(f"- log: {log_path.as_posix()}")

    report_path.write_text("\n".join(report_lines) + "\n", encoding="utf-8")
    log_path.write_text("\n".join(log_lines) + "\n", encoding="utf-8")

    return 0 if overall_pass else 1


if __name__ == "__main__":
    raise SystemExit(main())
