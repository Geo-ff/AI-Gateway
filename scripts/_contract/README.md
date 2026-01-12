# OpenAPI Contract Test（schema-based / Schemathesis）

运行（自动创建 venv 并安装依赖，输出脱敏报告）：

```bash
bash scripts/contract_p0_p1.sh
```

写入+清理 profile（fixture 注入真实 path 参数）：

```bash
CONTRACT_PROFILE=write bash scripts/contract_p0_p1.sh
```

## CI（fast/slow 分层）

- fast（read / GET-only）：GitHub Actions 在每次 `pull_request` + 所有分支 `push` 自动运行，命令为 `bash scripts/contract_p0_p1.sh`
- slow（write）：GitHub Actions 在 `workflow_dispatch`（手动触发）/ nightly schedule / `push` 到 `main` 运行，命令为 `CONTRACT_PROFILE=write CONTRACT_MAX_EXAMPLES=10 bash scripts/contract_p0_p1.sh`

CI 会自动：
- 启动 Postgres service（映射到 runner 的 `127.0.0.1:15432`，账号/密码/库与 `custom-config.toml` 匹配）
- 启动后端并后台运行，等待 `GET /auth/me` 返回 `401/200` 后再跑契约测试
- 运行前生成临时 `.env`（不提交），如需自定义账号/密钥可在仓库 Secrets 配置 `CONTRACT_EMAIL`/`CONTRACT_PASSWORD`/`GATEWAY_BOOTSTRAP_CODE`/`GW_JWT_SECRET`

配置来源（优先 `.env`，否则 `.env.example`；兼容 `KEY=VALUE` 与 `KEY:VALUE`）：
- `GATEWAY_BASE_URL`
- `EMAIL`
- `PASSWORD`
- `GATEWAY_BOOTSTRAP_CODE`（仅当首次空库且 login=401 时才会触发 bootstrap 注册兜底）

默认覆盖（GET-only，避免外部模型调用/费用）：
- `/auth/me`
- `/admin/users*`
- `/admin/tokens*`
- `/providers*`、`/providers/*/keys*`

默认跳过：
- `/auth/login` `/auth/refresh` `/auth/register`
- `/v1/*`（尤其 `/v1/chat/completions`）

产物（均脱敏，不会写入 JWT/refreshToken/password/provider key 明文）：
- `scripts/_contract/contract_<UTC时间戳>_<随机>.md`
- `scripts/_contract/contract_<UTC时间戳>_<随机>.log`
- `scripts/_contract/contractw_<UTC时间戳>_<随机>.md`
- `scripts/_contract/contractw_<UTC时间戳>_<随机>.log`
