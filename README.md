# Gateway Zero

Gateway Zero 是一个基于 Rust + Axum 的 AI API 网关后端。它对外提供 OpenAI 兼容的 `/v1/*` 接口，对内提供 Provider 管理、Client Token 管理、用户与权限、请求日志、模型价格、余额计费、订阅套餐和 Request Lab 等管理能力，适合作为多模型接入与二次分发场景的统一后端。

> 前端管理端已从本仓库拆分，当前主前端为独立项目 `captok`。本仓库专注后端服务、API 与数据存储。

## 功能特性

- **OpenAI 兼容入口**：支持 `/v1/chat/completions`、`/v1/models`、Token 用量与余额查询等接口。
- **多 Provider 接入**：内置 OpenAI、Anthropic、智谱、Gemini、Azure OpenAI、AWS Claude、Moonshot、DeepSeek、通义千问、豆包、MiniMax、讯飞星火、腾讯混元等 Provider 类型，并支持自定义 OpenAI 兼容端点。
- **负载均衡与 Key 管理**：支持 `first_available`、`round_robin`、`random` 策略，可管理 Provider、API Key、Key 启停、权重、连通性测试与 Key 统计。
- **Client Token 体系**：外部调用使用独立 Client Token，可限制模型、统计用量、控制启停，并与用户资源归属绑定。
- **管理端认证与 RBAC**：管理端使用 JWT AccessToken + RefreshToken，支持注册、登录、刷新、登出、修改密码、密码重置；`superadmin` 拥有管理权限，普通用户只能访问自己的 `/me/*` 资源。
- **日志、计费与分析**：记录请求模型、Provider、Token、耗时、状态、错误、Token 用量与金额；提供管理端指标、模型分布、成本序列和资源健康数据。
- **模型价格与余额**：支持模型价格维护、同步、价格缺失策略、用户余额与交易流水。
- **Request Lab**：支持请求记录回放、对比、快照、模板与实验调试，便于排查模型调用行为。
- **SQLite / PostgreSQL 存储**：默认可使用本地 SQLite，配置 `logging.pg_url` 后切换到 PostgreSQL。
- **流式响应与适配层**：支持 OpenAI 兼容流式响应，并为不同 Provider 做请求、鉴权与响应规范化。

## 技术栈

- Rust 2024
- Axum / Tokio
- Reqwest / Tower / Tower HTTP
- Rusqlite / Tokio Postgres
- Serde / TOML / OpenAPI
- Argon2 / HMAC / Ed25519
- Tracing

## 项目结构

```text
.
├── src/
│   ├── main.rs                 # 服务入口
│   ├── config/                 # 配置加载与 Provider 类型定义
│   ├── server/                 # Axum 路由、处理器、中间件与业务编排
│   ├── providers/              # Provider 适配器与上游协议转换
│   ├── routing/                # 负载均衡与 Key 轮转
│   ├── logging/                # SQLite/PostgreSQL 存储实现
│   ├── users.rs                # 用户模型
│   ├── balance.rs              # 余额模型
│   ├── subscription.rs         # 订阅套餐模型
│   └── refresh_tokens.rs       # RefreshToken 生命周期
├── openapi.yaml                # API 规范
├── custom-config.toml          # 本地配置示例
├── start.sh                    # 本地启动脚本示例
├── scripts/                    # 合约、RBAC、业务链路测试脚本
├── benches/                    # Criterion benchmark
└── docs/                       # 论文、设计图与辅助文档
```

## 快速开始

### 1. 准备环境

请先安装：

- Rust stable
- SQLite（默认模式无需额外服务）
- PostgreSQL 或兼容数据库（可选）

### 2. 配置环境变量

创建 `.env` 文件，至少提供管理端 JWT 密钥和首次注册码：

```env
GW_JWT_SECRET=please_replace_with_a_long_random_secret
GW_JWT_TTL_SECS=28800
GW_REFRESH_TTL_SECS=2592000
GATEWAY_BOOTSTRAP_CODE=please_replace_with_bootstrap_code
RUST_LOG=info
```

如果需要密码重置邮件，还可以配置：

```env
RESEND_API_KEY=your_resend_api_key
RESEND_FROM="Gateway Zero <noreply@example.com>"
CAPTOK_BASE_URL=http://localhost:5173
GATEWAY_BASE_URL=http://localhost:8080
RESET_PASSWORD_PATH=/reset-password
```

### 3. 配置服务

服务启动时会按顺序查找：

1. `custom-config.toml`
2. `config.toml`

二者都不存在时启动失败。最小 SQLite 配置示例：

```toml
[load_balancing]
strategy = "round_robin"

[server]
host = "0.0.0.0"
port = 8080

[logging]
database_path = "data/gateway.db"
key_log_strategy = "masked"
```

使用 PostgreSQL 时配置：

```toml
[logging]
pg_url = "host=127.0.0.1 port=5432 dbname=gateway_zero user=postgres password=your_password"
pg_schema = "public"
pg_pool_size = 4
key_log_strategy = "masked"
```

### 4. 启动后端

```bash
cargo run
```

启动成功后，服务默认监听：

```text
http://localhost:8080
```

同一组 API 也会挂载在 `/api/*` 下，便于前端代理或反向代理部署。

## 基本使用

### 首次注册管理员

```bash
curl -X POST http://localhost:8080/auth/register \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "change-me",
    "bootstrapCode": "please_replace_with_bootstrap_code"
  }'
```

### 登录管理端

```bash
curl -X POST http://localhost:8080/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "change-me"
  }'
```

登录后会返回 `accessToken` 与 `refreshToken`。`accessToken` 用于访问 `/auth/me`、`/me/*`、`/admin/*`、`/providers/*` 等接口。

### 调用 OpenAI 兼容聊天接口

外部调用 `/v1/*` 使用 Client Token，而不是管理端 JWT：

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer <client-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      { "role": "user", "content": "Hello Gateway Zero" }
    ]
  }'
```

## 认证模型

Gateway Zero 有两套语义不同的认证凭据：

- **Client Token**：用于外部客户端调用 `/v1/*`，例如聊天补全、模型列表、Token 余额与用量查询。
- **管理端登录态**：用于管理 API 和前端管理端，主要是 JWT AccessToken + RefreshToken，也保留 TUI/Web Session 兼容能力。

RefreshToken 服务端只保存 hash，不保存明文；刷新接口会进行 refresh token rotation，登出时可撤销服务端记录。

## 主要 API 分组

- `/v1/*`：OpenAI 兼容调用、模型列表、Client Token 余额与用量。
- `/auth/*`：注册、登录、刷新、登出、当前用户、密码修改与重置。
- `/me/*`：普通用户的模型、Token、日志、余额、请求回放和 Request Lab 数据。
- `/admin/*`：管理员 Token、用户、组织、日志、指标、模型价格、模型启用状态。
- `/providers/*`：Provider、API Key、模型发现、模型重定向、连通性测试。
- `/subscription/*`：订阅套餐列表与购买。

完整接口以 [openapi.yaml](./openapi.yaml) 为准。

## 前端说明

本仓库不再维护内置 Web 前端和旧 TUI 客户端。管理端 UI 请使用独立前端项目 `captok`，后端可通过 `CAPTOK_BASE_URL` 配置密码重置等跳转地址。

后端开发时可直接启动本服务，再由前端项目代理到：

```text
http://localhost:8080
```

## 开发与测试

常用命令：

```bash
cargo fmt
cargo check
cargo test
```

运行 benchmark：

```bash
cargo bench --bench endpoints
```

脚本目录中包含部分端到端检查：

```bash
scripts/contract_p0_p1.sh
scripts/rbac_p0_p1.sh
scripts/biz_p0_p1.sh
scripts/biz3_lb.sh
```

这些脚本通常依赖本地后端、数据库和测试数据，请根据实际环境确认配置后再运行。

## 部署提示

- 生产环境请使用足够长且随机的 `GW_JWT_SECRET`。
- 不要把 `.env`、真实 API Key、数据库文件、dump 文件或管理员私钥提交到公开仓库。
- 建议生产环境使用 PostgreSQL，并对 `pg_url`、邮件密钥、Provider Key 等敏感配置使用环境变量或密钥管理服务托管。
- `key_log_strategy` 推荐使用 `masked` 或 `none`，避免日志记录明文 API Key。
- 当前 CORS 逻辑偏开发友好，生产环境建议收敛允许来源，并通过 HTTPS 暴露服务。
- 首次启动可能生成管理员 Ed25519 私钥，请妥善备份并限制文件权限。

## GitHub 发布前检查

上传公开仓库前，建议确认以下文件没有包含真实敏感信息：

- `.env`
- `.env.example`
- `custom-config.toml`
- `data/*.key`
- `data/*.db`
- `postgres.dump`
- `gateway.log`
- `session_cache.json`

如果这些文件仅用于本地开发，建议加入 `.gitignore` 或改为提交脱敏模板。

## License

本项目许可证见 [LICENSE.txt](./LICENSE.txt)。
