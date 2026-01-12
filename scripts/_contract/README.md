# OpenAPI Contract Test（schema-based / Schemathesis）

运行（自动创建 venv 并安装依赖，输出脱敏报告）：

```bash
bash scripts/contract_p0_p1.sh
```

写入+清理 profile（fixture 注入真实 path 参数）：

```bash
CONTRACT_PROFILE=write bash scripts/contract_p0_p1.sh
```

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
