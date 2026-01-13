#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import secrets
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import quote

# Keep redaction & curl behavior consistent with biz2.
import run_biz


ROOT_DIR = Path(__file__).resolve().parents[2]
READY_CHECK_URL = "http://localhost:8080/auth/me"
CUSTOM_CONFIG = ROOT_DIR / "custom-config.toml"


def utc_now() -> datetime:
    return datetime.now(tz=timezone.utc)


def utc_compact_timestamp(dt: datetime) -> str:
    return dt.strftime("%Y%m%dT%H%M%SZ")


def git_sha_short() -> str:
    try:
        p = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=str(ROOT_DIR),
            capture_output=True,
            text=True,
            timeout=5,
        )
        if p.returncode == 0:
            return (p.stdout or "").strip() or "(unknown)"
    except Exception:
        pass
    return "(unknown)"


def load_config_merged() -> dict[str, str]:
    base = run_biz.parse_env_file(ROOT_DIR / ".env.example")
    base.update(run_biz.parse_env_file(ROOT_DIR / ".env"))
    return base


def parse_custom_config_strategy(path: Path) -> str:
    if not path.exists():
        return ""
    text = path.read_text(encoding="utf-8", errors="replace")
    in_lb = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_lb = line == "[load_balancing]"
            continue
        if not in_lb:
            continue
        m = re.match(r'^strategy\s*=\s*"([^"]+)"\s*$', line)
        if m:
            return m.group(1).strip()
    return ""


def rewrite_custom_config_strategy(path: Path, *, strategy: str) -> None:
    if not path.exists():
        raise RuntimeError(f"missing {path}")
    src = path.read_text(encoding="utf-8", errors="replace").splitlines(keepends=True)
    out: list[str] = []
    in_lb = False
    replaced = False
    for raw in src:
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            if in_lb and not replaced:
                out.append(f'strategy = "{strategy}"\n')
                replaced = True
            in_lb = line == "[load_balancing]"
            out.append(raw)
            continue
        if in_lb and re.match(r'^\s*strategy\s*=\s*".*"\s*(#.*)?$', raw):
            out.append(f'strategy = "{strategy}"\n')
            replaced = True
            continue
        out.append(raw)
    if in_lb and not replaced:
        out.append(f'strategy = "{strategy}"\n')
    path.write_text("".join(out), encoding="utf-8")


def _which(cmd: str) -> bool:
    return shutil.which(cmd) is not None


def find_listening_pids(port: int) -> list[int]:
    pids: list[int] = []
    if _which("lsof"):
        p = subprocess.run(
            ["lsof", "-t", f"-iTCP:{port}", "-sTCP:LISTEN"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        if p.returncode == 0:
            for s in (p.stdout or "").split():
                if s.isdigit():
                    pids.append(int(s))
        return sorted(set(pids))
    if _which("fuser"):
        p = subprocess.run(
            ["fuser", "-n", "tcp", str(port)],
            capture_output=True,
            text=True,
            timeout=5,
        )
        for s in re.findall(r"\\b(\\d+)\\b", (p.stdout or "") + " " + (p.stderr or "")):
            pids.append(int(s))
        return sorted(set(pids))
    if _which("ss"):
        p = subprocess.run(
            ["ss", "-ltnp", f"sport = :{port}"],
            capture_output=True,
            text=True,
            timeout=5,
        )
        for m in re.finditer(r"pid=(\\d+)", p.stdout or ""):
            pids.append(int(m.group(1)))
        return sorted(set(pids))
    return []


def stop_by_port(port: int, *, timeout_s: int = 15) -> None:
    pids = find_listening_pids(port)
    if not pids:
        return
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        still = [pid for pid in pids if Path(f"/proc/{pid}").exists()]
        if not still:
            return
        time.sleep(0.2)
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def wait_ready(base_url: str, *, timeout_s: int = 120) -> bool:
    deadline = time.time() + timeout_s
    url = f"{base_url}/auth/me"
    while time.time() < deadline:
        rc, code = run_biz.curl_http_code(url, timeout_s=5)
        if rc == 0 and code in ("200", "401"):
            return True
        time.sleep(0.3)
    return False


def start_backend(server_log_path: Path) -> subprocess.Popen[bytes]:
    # Prefer running the built binary to avoid repeated `cargo run` rebuild overhead.
    # Avoid capturing output to reduce risk of leaking config secrets.
    bin_path = ROOT_DIR / "target" / "debug" / "gateway"
    subprocess.run(["cargo", "build"], cwd=str(ROOT_DIR), check=True)
    server_log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(server_log_path, "ab") as fp:
        return subprocess.Popen(
            [str(bin_path)],
            cwd=str(ROOT_DIR),
            stdout=fp,
            stderr=fp,
            env={**os.environ},
        )


def _tail_bytes(path: Path, n: int = 4096) -> str:
    try:
        data = path.read_bytes()
        if len(data) > n:
            data = data[-n:]
        return data.decode("utf-8", errors="replace")
    except Exception:
        return ""


def wait_ready_with_proc(
    proc: subprocess.Popen[bytes],
    base_url: str,
    server_log_path: Path,
    *,
    timeout_s: int = 120,
) -> tuple[bool, str]:
    deadline = time.time() + timeout_s
    url = f"{base_url}/auth/me"
    while time.time() < deadline:
        if proc.poll() is not None:
            return False, _tail_bytes(server_log_path, 4000)
        rc, code = run_biz.curl_http_code(url, timeout_s=5)
        if rc == 0 and code in ("200", "401"):
            return True, ""
        time.sleep(0.3)
    return False, _tail_bytes(server_log_path, 4000)


@dataclass(frozen=True)
class LbRow:
    i: int
    http_status: int
    rt_ms: int
    provider: str
    api_key_hint: str


def mask_key_local(key: str) -> str:
    s = (key or "").strip()
    if not s:
        return "(none)"
    if len(s) <= 8:
        return "****"
    return f"{s[:4]}****{s[-4:]}"


def append_workflow_record_biz3(*, report_path: Path, ok: bool) -> None:
    doc_path = ROOT_DIR / "workflow_follow.md"
    if not doc_path.exists():
        return
    stamp = utc_now().strftime("%Y-%m-%dT%H:%M:%SZ")
    status = "Pass" if ok else "Fail"
    try:
        report_display = report_path.relative_to(ROOT_DIR).as_posix()
    except Exception:
        report_display = report_path.as_posix()

    heading = "#### 接口测试记录（业务语义 biz）"
    record = f"- biz3 负载均衡（多 Provider/多 Key） {stamp}：{status}，报告：`{report_display}`\n"
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
        if not text.endswith("\n"):
            text += "\n"
        doc_path.write_text(text + f"\n{heading}\n\n" + record, encoding="utf-8")
        return
    insert_at = None
    for j in range(heading_idx + 1, len(lines)):
        if lines[j].startswith(("#### ", "### ", "## ")):
            insert_at = j
            break
    if insert_at is None:
        insert_at = len(lines)
    lines.insert(insert_at, record)
    doc_path.write_text("".join(lines), encoding="utf-8")


def main() -> int:
    run_dt = utc_now()
    run_stamp = utc_compact_timestamp(run_dt)
    run_rand = secrets.token_hex(3)
    run_id = f"biz3_lb_{run_stamp}_{run_rand}"

    out_dir = ROOT_DIR / "scripts" / "_biz"
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / f"{run_id}.md"
    log_path = out_dir / f"{run_id}.log"
    # Server stderr/stdout can include config parsing errors (may contain sensitive DSNs),
    # so keep it out of the workspace reports by default.
    server_log_path = Path("/tmp") / f"{run_id}.server.log"

    cfg = load_config_merged()
    base_url = (run_biz.pick(cfg, ["GATEWAY_BASE_URL"]) or "http://localhost:8080").rstrip("/")
    email = run_biz.pick(cfg, ["EMAIL"]) or ""
    password = run_biz.pick(cfg, ["PASSWORD"]) or ""
    bootstrap_code = run_biz.pick(cfg, ["GATEWAY_BOOTSTRAP_CODE"]) or ""

    def log(msg: str) -> None:
        # Keep both stdout and log file consistent and redacted.
        line = run_biz.redact_text(msg.rstrip("\n"))
        with open(log_path, "a", encoding="utf-8") as fp:
            fp.write(line + "\n")
        sys.stdout.write(line + "\n")
        sys.stdout.flush()

    # Truncate any old file (re-run safety).
    log_path.write_text("", encoding="utf-8")

    log("== Gateway Zero biz3: load balancing verification (providers/keys) ==")
    log(f"time_utc: {run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}")
    log(f"git_sha : {git_sha_short()}")
    log(f"base_url: {base_url}")
    log(f"ready_check(required): curl {READY_CHECK_URL}")

    # Required readiness check (401/200 OK).
    rc, code = run_biz.curl_http_code(READY_CHECK_URL, timeout_s=8)
    if rc != 0 or not code or code == "000":
        msg = "FATAL: 无法连接到后端，请先启动数据库与后端：`docker start gateway-postgres` + `cargo run`"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        append_workflow_record_biz3(report_path=report_path, ok=False)
        return 2
    if code not in ("200", "401"):
        msg = f"FATAL: 后端就绪检查返回非预期 http_code={code}（仅 401/200 视为 OK）：{READY_CHECK_URL}"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        append_workflow_record_biz3(report_path=report_path, ok=False)
        return 2

    # Login as superadmin
    access_token: str | None = None

    def do_login() -> run_biz.CurlResult:
        return run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path="/auth/login",
            json_body={"email": email, "password": password},
            timeout_s=30,
        )

    try:
        res = do_login()
        if res.status_code == 401 and bootstrap_code:
            log("WARN: superadmin login=401, trying one-time /auth/register bootstrap fallback")
            _ = run_biz.curl_json(
                base_url=base_url,
                method="POST",
                path="/auth/register",
                json_body={"bootstrap_code": bootstrap_code, "email": email, "password": password},
                timeout_s=30,
            )
            res = do_login()
        if res.status_code != 200 or not isinstance(res.body_json, dict):
            raise RuntimeError(
                f"login failed: status={res.status_code} body={run_biz.response_snippet(res.body_text)}"
            )
        access_token = str(res.body_json.get("accessToken") or "").strip() or None
        if not access_token:
            raise RuntimeError("login missing accessToken")
        log(f"superadmin_accessToken: {run_biz.mask_secret(access_token)}")
    except Exception as exc:
        msg = f"FATAL: {exc}"
        report_path.write_text(run_biz.redact_text(msg) + "\n", encoding="utf-8")
        log(msg)
        append_workflow_record_biz3(report_path=report_path, ok=False)
        return 2

    assert access_token is not None

    # API helpers
    def list_providers() -> list[dict[str, Any]]:
        r = run_biz.curl_json(base_url=base_url, method="GET", path="/providers", bearer=access_token, timeout_s=30)
        if r.status_code != 200:
            return []
        if isinstance(r.body_json, list):
            return [p for p in r.body_json if isinstance(p, dict)]
        if isinstance(r.body_json, dict) and isinstance(r.body_json.get("data"), list):
            return [p for p in r.body_json["data"] if isinstance(p, dict)]
        return []

    def get_provider(name: str) -> dict[str, Any] | None:
        r = run_biz.curl_json(
            base_url=base_url,
            method="GET",
            path=f"/providers/{name}",
            bearer=access_token,
            timeout_s=30,
        )
        if r.status_code == 200 and isinstance(r.body_json, dict):
            return r.body_json
        return None

    def provider_keys_raw(provider: str) -> list[str]:
        r = run_biz.curl_json(
            base_url=base_url,
            method="GET",
            path=f"/providers/{provider}/keys/raw",
            bearer=access_token,
            timeout_s=30,
        )
        if r.status_code != 200 or r.body_json is None:
            return []
        items: list[Any] = []
        if isinstance(r.body_json, dict) and isinstance(r.body_json.get("keys"), list):
            items = r.body_json["keys"]
        elif isinstance(r.body_json, list):
            items = r.body_json
        out: list[str] = []
        for item in items:
            if isinstance(item, dict) and isinstance(item.get("value"), str):
                v = item["value"].strip()
                if v:
                    out.append(v)
        return out

    def delete_keys_batch(provider: str, keys: list[str]) -> bool:
        if not keys:
            return True
        r = run_biz.curl_json(
            base_url=base_url,
            method="DELETE",
            path=f"/providers/{provider}/keys/batch",
            bearer=access_token,
            json_body={"keys": keys},
            timeout_s=30,
        )
        return r.status_code == 200

    def add_keys_batch(provider: str, keys: list[str]) -> bool:
        if not keys:
            return True
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path=f"/providers/{provider}/keys/batch",
            bearer=access_token,
            json_body={"keys": keys},
            timeout_s=30,
        )
        return r.status_code == 200

    def create_provider(*, name: str, api_type: str, upstream_base_url: str) -> bool:
        body = {"name": name, "api_type": api_type, "base_url": upstream_base_url, "models_endpoint": None}
        r = run_biz.curl_json(base_url=base_url, method="POST", path="/providers", bearer=access_token, json_body=body, timeout_s=30)
        return r.status_code == 200

    def delete_provider(name: str) -> bool:
        r = run_biz.curl_json(base_url=base_url, method="DELETE", path=f"/providers/{name}", bearer=access_token, timeout_s=30)
        return r.status_code in (200, 204)

    def add_key(provider: str, key: str) -> bool:
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path=f"/providers/{provider}/keys",
            bearer=access_token,
            json_body={"key": key},
            timeout_s=30,
        )
        return r.status_code == 200

    def update_cache_selected(provider: str, model_id: str) -> bool:
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path=f"/models/{provider}/cache",
            bearer=access_token,
            json_body={"mode": "selected", "include": [model_id], "replace": False},
            timeout_s=60,
        )
        return r.status_code == 200

    def upsert_price(provider: str, model_id: str) -> bool:
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/model-prices",
            bearer=access_token,
            json_body={
                "provider": provider,
                "model": model_id,
                "prompt_price_per_million": 0.0,
                "completion_price_per_million": 0.0,
                "currency": "USD",
            },
            timeout_s=30,
        )
        return r.status_code in (200, 201)

    def refresh_models(provider: str) -> list[str]:
        r = run_biz.curl_json(
            base_url=base_url,
            method="GET",
            path=f"/models/{provider}?refresh=true",
            bearer=access_token,
            timeout_s=45,
        )
        return run_biz.parse_models_ids(r.body_json)

    def create_client_token(name: str) -> str:
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path="/admin/tokens",
            bearer=access_token,
            json_body={"name": name, "enabled": True},
            timeout_s=30,
        )
        if r.status_code == 201 and isinstance(r.body_json, dict):
            tok = str(r.body_json.get("token") or "").strip()
            return tok
        return ""

    def chat_once(*, client_token: str, model_id: str) -> int:
        body = {"model": model_id, "messages": [{"role": "user", "content": "ping"}], "max_tokens": 1, "temperature": 0}
        r = run_biz.curl_json(
            base_url=base_url,
            method="POST",
            path="/v1/chat/completions",
            bearer=client_token,
            json_body=body,
            timeout_s=60,
        )
        return r.status_code

    def latest_chat_log(*, client_token: str, model_id: str) -> dict[str, Any] | None:
        q = (
            "/admin/logs/requests?limit=1"
            "&request_type=chat_once"
            "&method=POST"
            "&path=/v1/chat/completions"
            f"&client_token={quote(client_token)}"
            f"&model={quote(model_id)}"
        )
        r = run_biz.curl_json(base_url=base_url, method="GET", path=q, bearer=access_token, timeout_s=30)
        if r.status_code != 200 or not isinstance(r.body_json, dict):
            return None
        data = r.body_json.get("data")
        if isinstance(data, list) and data and isinstance(data[0], dict):
            return data[0]
        return None

    # --- fixture + isolation ---
    prefix = f"biz3_{run_stamp}"
    provider_a = f"{prefix}_provider"
    provider_b = f"{prefix}_provider_b"
    client_token_name = f"{prefix}_clienttoken"

    disabled_keys: dict[str, list[str]] = {}
    cleanup_notes: list[str] = []

    def cleanup_step(desc: str, fn) -> None:
        try:
            ok = bool(fn())
            cleanup_notes.append(f"- {desc}：{'OK' if ok else 'SKIP/IGNORED'}")
        except Exception as exc:
            cleanup_notes.append(f"- {desc}：FAIL（{run_biz.redact_text(str(exc))}）")

    # Select a source provider/key from DB (fallback when .env doesn't have upstream keys).
    providers = list_providers()
    source_provider = ""
    for p in providers:
        name = str(p.get("name") or "")
        keys = p.get("api_keys") or []
        if name and isinstance(keys, list) and keys:
            source_provider = name
            break

    source_keys = provider_keys_raw(source_provider) if source_provider else []
    source_meta = get_provider(source_provider) if source_provider else None
    key1 = (run_biz.pick(cfg, ["UPSTREAM_API_KEY_1", "UPSTREAM_API_KEY1", "PROVIDER_API_KEY_1", "PROVIDER_API_KEY1"]) or "").strip()
    key2 = (run_biz.pick(cfg, ["UPSTREAM_API_KEY_2", "UPSTREAM_API_KEY2", "PROVIDER_API_KEY_2", "PROVIDER_API_KEY2"]) or "").strip()
    if not key1 and source_keys:
        key1 = source_keys[0]
    if not key2 and len(source_keys) >= 2:
        key2 = source_keys[1]
    has_two_keys = bool(key1 and key2)
    key1_mask = mask_key_local(key1)
    key2_mask = mask_key_local(key2) if key2 else ""

    api_type = (run_biz.pick(cfg, ["PROVIDER_API_TYPE", "UPSTREAM_API_TYPE", "API_TYPE"]) or "").strip().lower()
    upstream_base_url = (run_biz.pick(cfg, ["UPSTREAM_BASE_URL", "PROVIDER_BASE_URL", "OPENAI_BASE_URL", "BASEURL", "BASE_URL"]) or "").strip()
    if not api_type and isinstance((source_meta or {}).get("api_type"), str):
        api_type = str(source_meta["api_type"]).strip().lower()
    if not upstream_base_url and isinstance((source_meta or {}).get("base_url"), str):
        upstream_base_url = str(source_meta["base_url"]).strip()
    upstream_base_url, note = run_biz.normalize_provider_base_url(api_type=api_type or "openai", base_url=upstream_base_url)

    log(f"fixture_provider_a: {provider_a}")
    log(f"fixture_provider_b: {provider_b}")
    log(f"source_provider_for_keys: {source_provider or '(none)'} keys_found={len(source_keys)}")
    log(f"upstream_api_type: {api_type or '(unset)'}")
    log(f"upstream_base_url: {run_biz.mask_secret(upstream_base_url, keep=12)}" + (f" ({note})" if note else ""))
    log(f"key1: <REDACTED {run_biz.mask_secret(key1, keep=0)}>")
    log(f"key2: <REDACTED {run_biz.mask_secret(key2, keep=0)}>")

    if not upstream_base_url or not key1:
        msg = "FATAL: 缺少上游 base_url 或 key（需在 .env 配置，或 DB 中存在至少 1 个 provider key 供复用）"
        report_path.write_text(msg + "\n", encoding="utf-8")
        log(msg)
        append_workflow_record_biz3(report_path=report_path, ok=False)
        return 2

    # Isolate by temporarily removing keys from non-biz3 providers with keys.
    for p in providers:
        name = str(p.get("name") or "")
        if not name or name.startswith(prefix):
            continue
        keys = p.get("api_keys") or []
        if not isinstance(keys, list) or not keys:
            continue
        raw = provider_keys_raw(name)
        if raw:
            disabled_keys[name] = raw

    for name, keys in disabled_keys.items():
        ok = delete_keys_batch(name, keys)
        log(f"isolation_disable_keys: provider={name} keys={len(keys)} ok={ok}")
        if not ok:
            msg = "FATAL: 无法完成隔离（删除非 biz3 providers keys 失败），请手工清理后重试"
            report_path.write_text(msg + "\n", encoding="utf-8")
            log(msg)
            # best-effort restore
            for n, ks in disabled_keys.items():
                _ = add_keys_batch(n, ks)
            append_workflow_record_biz3(report_path=report_path, ok=False)
            return 2

    backend_proc: subprocess.Popen[bytes] | None = None
    original_custom = CUSTOM_CONFIG.read_text(encoding="utf-8", errors="replace") if CUSTOM_CONFIG.exists() else ""

    phases = [("round_robin", 8), ("random", 20), ("first_available", 5)]
    phase_rows: dict[str, list[LbRow]] = {}
    phase_ok: dict[str, bool] = {}
    phase_notes: dict[str, str] = {}

    try:
        # Create providers + keys
        if not create_provider(name=provider_a, api_type=api_type or "openai", upstream_base_url=upstream_base_url):
            raise RuntimeError("create provider_a failed")
        if not create_provider(name=provider_b, api_type=api_type or "openai", upstream_base_url=upstream_base_url):
            raise RuntimeError("create provider_b failed")
        if not add_key(provider_a, key1):
            raise RuntimeError("add key1 to provider_a failed")
        if not add_key(provider_b, key1):
            raise RuntimeError("add key1 to provider_b failed")
        if has_two_keys and not add_key(provider_a, key2):
            raise RuntimeError("add key2 to provider_a failed")

        model_ids = refresh_models(provider_a)
        model_id = ""
        if model_ids:
            prefer = run_biz.pick(cfg, ["BIZ_TEST_MODEL", "TEST_MODEL", "OPENAI_MODEL", "MODEL", "CHAT_MODEL"]).strip()
            model_id = run_biz.pick_model_id(model_ids, prefer=prefer)
        if not model_id:
            model_id = "gpt-4o-mini"
            log("WARN: refresh models failed/empty; fallback model=gpt-4o-mini (upstream-dependent)")

        if not update_cache_selected(provider_a, model_id):
            raise RuntimeError("update cache provider_a failed")
        if not update_cache_selected(provider_b, model_id):
            raise RuntimeError("update cache provider_b failed")
        if not upsert_price(provider_a, model_id):
            raise RuntimeError("upsert price provider_a failed")
        if not upsert_price(provider_b, model_id):
            raise RuntimeError("upsert price provider_b failed")

        client_token = create_client_token(client_token_name)
        if not client_token:
            raise RuntimeError("create client token failed")
        log(f"client_token: <REDACTED {run_biz.mask_secret(client_token, keep=0)}>")

        provider_order = sorted([provider_a, provider_b])
        p0, p1 = provider_order[0], provider_order[1]

        for strategy, n in phases:
            log(f"== phase strategy={strategy} n={n} ==")
            if not CUSTOM_CONFIG.exists():
                raise RuntimeError("custom-config.toml missing; cannot switch strategy for runtime verification")
            rewrite_custom_config_strategy(CUSTOM_CONFIG, strategy=strategy)

            # restart backend to apply config
            if backend_proc is not None and backend_proc.poll() is None:
                backend_proc.terminate()
                try:
                    backend_proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    backend_proc.kill()
            stop_by_port(8080, timeout_s=10)
            server_log_path.write_bytes(b"")
            backend_proc = start_backend(server_log_path)
            ok_ready, _out = wait_ready_with_proc(backend_proc, base_url, server_log_path, timeout_s=180)
            if not ok_ready:
                raise RuntimeError(
                    f"backend not ready after restart (strategy={strategy}); server_log={server_log_path}"
                )

            rows: list[LbRow] = []
            for i in range(1, n + 1):
                status = chat_once(client_token=client_token, model_id=model_id)
                entry: dict[str, Any] | None = None
                for _ in range(8):
                    entry = latest_chat_log(client_token=client_token, model_id=model_id)
                    if entry is not None:
                        break
                    time.sleep(0.2)
                provider_used = str((entry or {}).get("provider") or "") or "(unknown)"
                api_key_used_raw = str((entry or {}).get("api_key") or "")
                # Always mask locally to prevent accidental leakage when server is configured as key_log_strategy=plain.
                api_key_used = mask_key_local(api_key_used_raw)
                rt_ms = int((entry or {}).get("response_time_ms") or 0)
                rows.append(LbRow(i=i, http_status=status, rt_ms=rt_ms, provider=provider_used, api_key_hint=api_key_used))
                log(f"req#{i} status={status} provider={provider_used} api_key={api_key_used} rt_ms={rt_ms}")

            providers_seen = [r.provider for r in rows]
            provider_ok = True
            if strategy == "first_available":
                provider_ok = all(p == p0 for p in providers_seen)
            elif strategy == "random":
                provider_ok = (p0 in providers_seen) and (p1 in providers_seen)
            elif strategy == "round_robin":
                expected = [p0 if (k % 2 == 0) else p1 for k in range(n)]
                provider_ok = providers_seen == expected

            key_note = "key_check=SKIP(no_key2)"
            key_ok = True
            if has_two_keys:
                key_note = "key_check=PASS"
                # Provider B only has key1.
                key_seq_b = [r.api_key_hint for r in rows if r.provider == provider_b]
                if any(k != key1_mask for k in key_seq_b):
                    key_ok = False
                    key_note = "key_check=FAIL(provider_b_expected_key1)"

                # Provider A has key1+key2; verify per-strategy behavior on its own subsequence.
                key_seq_a = [r.api_key_hint for r in rows if r.provider == provider_a]
                if strategy == "first_available":
                    if any(k != key1_mask for k in key_seq_a):
                        key_ok = False
                        key_note = "key_check=FAIL(first_available_expected_key1)"
                elif strategy == "round_robin":
                    expected_a = [key1_mask if (i % 2 == 0) else key2_mask for i in range(len(key_seq_a))]
                    if key_seq_a != expected_a:
                        key_ok = False
                        key_note = "key_check=FAIL(round_robin_expected_alternating)"
                elif strategy == "random":
                    # Random: only assert both keys appear when provider_a hit enough times.
                    if len(key_seq_a) >= 8 and (key1_mask not in key_seq_a or key2_mask not in key_seq_a):
                        key_ok = False
                        key_note = "key_check=FAIL(random_expected_both_keys)"
                    elif len(key_seq_a) < 8:
                        key_note = f"key_check=SKIP(provider_a_hits={len(key_seq_a)})"

            ok = provider_ok and key_ok
            phase_ok[strategy] = ok
            phase_rows[strategy] = rows
            phase_notes[strategy] = f"provider_check={'PASS' if provider_ok else 'FAIL'}; {key_note}"
            log(f"phase_result: strategy={strategy} ok={ok} {phase_notes[strategy]}")

        pass_count = sum(1 for v in phase_ok.values() if v)
        fail_count = len(phase_ok) - pass_count

        report_lines: list[str] = []
        report_lines.append("# biz3 负载均衡（多 Key / 多 Provider）业务验证报告\n\n")
        report_lines.append(f"- time_utc: `{run_dt.strftime('%Y-%m-%dT%H:%M:%SZ')}`\n")
        report_lines.append(f"- base_url: `{base_url}`\n")
        report_lines.append(f"- git_sha: `{git_sha_short()}`\n")
        report_lines.append(f"- strategies_tested: `{', '.join([s for s, _ in phases])}`\n")
        report_lines.append(f"- provider_fixture_a: `{provider_a}`\n")
        report_lines.append(f"- provider_fixture_b: `{provider_b}`\n")
        report_lines.append(f"- isolated_other_providers: `{bool(disabled_keys)}`\n")
        report_lines.append(f"- upstream_api_type: `{api_type or '(unset)'}`\n")
        report_lines.append(f"- upstream_base_url: `<REDACTED {run_biz.mask_secret(upstream_base_url, keep=12)}>`\n")
        report_lines.append(f"- model: `{model_id}`\n")
        report_lines.append(f"- multi_key_count(provider_a): `{2 if has_two_keys else 1}`\n")
        report_lines.append("- observable_signal: `GET /admin/logs/requests` 字段 `provider` + `api_key`（按 key_log_strategy 输出 masked/plain/none）\n")
        if not has_two_keys:
            report_lines.append("\n> NOTE: 当前环境缺少第二把上游 key（.env 未配置且 DB 仅 1 把），因此本次仅覆盖 **多 Provider** 的策略验证；多 Key 行为无法做真实运行时断言。\n")
        report_lines.append("\n")

        for strategy, _n in phases:
            ok = phase_ok.get(strategy, False)
            report_lines.append(f"## {strategy}\n\n")
            report_lines.append(f"- result: **{'PASS' if ok else 'FAIL'}**\n\n")
            if strategy in phase_notes:
                report_lines.append(f"- checks: `{phase_notes[strategy]}`\n\n")
            report_lines.append("| # | HTTP | rt_ms | provider | api_key(masked) |\n")
            report_lines.append("|---:|---:|---:|---|---|\n")
            for r in phase_rows.get(strategy, []):
                report_lines.append(f"| {r.i} | {r.http_status} | {r.rt_ms} | `{r.provider}` | `{r.api_key_hint}` |\n")
            report_lines.append("\n")

        report_lines.append("## 汇总\n\n")
        report_lines.append(f"- Pass={pass_count} / Fail={fail_count} / Total={len(phases)}\n")
        report_lines.append(f"- 结论：**{'Pass' if fail_count == 0 else 'Fail'}**\n\n")

        report_lines.append("## Cleanup\n\n")
        report_lines.extend([line + "\n" for line in cleanup_notes] if cleanup_notes else ["- (none)\n"])

        report_lines.append("\n## 自我评估\n\n")
        report_lines.append("- 是否泄露敏感信息：脚本对 JWT/provider keys/client token 做脱敏；报告包含泄露自检\n")
        report_lines.append("- 可重复执行/数据污染：fixture provider 做 best-effort 删除；非 biz3 providers keys 做临时移除并尽力恢复\n")
        report_lines.append("- 费用控制：chat max_tokens=1；price=0 仅影响网关计费统计，不影响上游实际计费\n")
        report_lines.append("- 兼容性：本脚本只走管理/业务接口，不改动对外 API\n")

        report_text = run_biz.redact_text("".join(report_lines))
        try:
            run_biz.assert_no_secret_leak(report_text, where=str(report_path))
        except Exception as exc:
            report_text += f"\n\nFATAL: redaction self-check failed: {exc}\n"
        report_path.write_text(report_text, encoding="utf-8")
        append_workflow_record_biz3(report_path=report_path, ok=(fail_count == 0))

        sys.stdout.write(
            f"{'Pass' if fail_count == 0 else 'Fail'}: report={report_path.relative_to(ROOT_DIR).as_posix()} log={log_path.relative_to(ROOT_DIR).as_posix()}\n"
        )
        return 0 if fail_count == 0 else 1

    except Exception as exc:
        msg = f"FATAL: {exc}"
        report_path.write_text(run_biz.redact_text(msg) + "\n", encoding="utf-8")
        log(msg)
        append_workflow_record_biz3(report_path=report_path, ok=False)
        return 2

    finally:
        # Restore config first, then ensure backend is up for cleanup calls.
        if original_custom and CUSTOM_CONFIG.exists():
            try:
                CUSTOM_CONFIG.write_text(original_custom, encoding="utf-8")
            except Exception:
                pass

        if backend_proc is not None and backend_proc.poll() is None:
            try:
                backend_proc.terminate()
                backend_proc.wait(timeout=10)
            except Exception:
                try:
                    backend_proc.kill()
                except Exception:
                    pass
        stop_by_port(8080, timeout_s=10)
        try:
            server_log_path.write_bytes(b"")
            backend_proc = start_backend(server_log_path)
            _ok, _out = wait_ready_with_proc(backend_proc, base_url, server_log_path, timeout_s=240)
        except Exception:
            pass

        # Restore disabled keys
        for name, keys in disabled_keys.items():
            cleanup_step(
                f"恢复 provider keys provider={name} count={len(keys)}",
                lambda n=name, ks=keys: add_keys_batch(n, ks),
            )

        # Delete fixture providers
        cleanup_step(f"删除 fixture provider_b={provider_b}", lambda: delete_provider(provider_b))
        cleanup_step(f"删除 fixture provider_a={provider_a}", lambda: delete_provider(provider_a))


if __name__ == "__main__":
    raise SystemExit(main())
