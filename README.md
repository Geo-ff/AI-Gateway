# Gateway Zero

Gateway Zero 是一个基于 Rust + Axum 的 AI API 网关：对外提供 OpenAI 兼容的 `/v1/*` 接口，并提供管理端 API（用户/Provider/Token/日志等）。

## 文档入口

- 快速上手：`QUICK_START.md`
- Token 解释（Client Token）：`TOKEN_EXPLAINED.md`
- OpenAPI 规范：`openapi.yaml`
- 架构说明：`ARCHITECTURE.md`

## 运行与配置

### 1) 配置文件

服务启动时会按顺序加载配置：

1. `custom-config.toml`
2. `config.toml`

二者都不存在会启动失败。示例见：`custom-config.toml`。

### 2) 环境变量（认证相关）

管理端 JWT（`/auth/login`、`/auth/me`、`/auth/refresh`）：

- `GW_JWT_SECRET`（必需）：JWT HS256 密钥（字节串；建议足够长且随机）
- `GW_JWT_TTL_SECS`（可选，默认 `28800`=8h）：AccessToken 过期时间
- `GW_REFRESH_TTL_SECS`（可选，默认 `2592000`=30d）：RefreshToken 过期时间
- `GW_ADMIN_PERMISSIONS`（可选）：逗号分隔权限列表（空则按角色默认值）

首次初始化注册（`POST /auth/register`）：

- `GATEWAY_BOOTSTRAP_CODE`（必需）：bootstrap code；未设置时该接口会返回 `401 Unauthorized`

日志：

- `RUST_LOG`（可选）：例如 `info` / `debug`

### 3) 启动

```bash
# 开发启动
RUST_LOG=info cargo run
```

也可参考 `start.sh`（包含启动 Postgres 容器与前端的示例脚本）。

## 认证模型（重要）

本项目存在两类 Token/登录态，语义不同：

1) **Client Token**（用于外部调用 `/v1/*`）  
`Authorization: Bearer <client-token>`

2) **管理端登录态**（用于管理 API 与管理端 UI）
- **JWT AccessToken + RefreshToken**：`/auth/login` 登录发放；AccessToken 过期可用 `/auth/refresh` 无感续期（refresh rotation）
- **TUI/Web Session**：用于 TUI challenge/verify 与 `gw_session` Cookie 会话（仍保持兼容）

## RBAC v1（P3）

- `superadmin`：拥有全部管理端权限（所有 `/admin/*`、`/providers/*`）。
- 其余角色一律视为普通用户：仅可通过用户侧接口查看/管理自己的资源（`/me/*`），不可访问管理端接口（403）。
- ClientToken 引入资源归属：`client_tokens.user_id` 为空视为存量/未绑定 token，仅 `superadmin` 可见/可管理；用户侧创建的 token 会自动绑定当前用户。

## 管理端 JWT 生命周期（P2）

- 登录：`POST /auth/login` 返回 `accessToken + refreshToken`（refresh token 服务端仅存储 hash）
- 刷新：`POST /auth/refresh` 使用 refresh token 换取新 `accessToken + refreshToken`，并撤销旧 refresh token（rotation）
- 登出：`POST /auth/logout` 可携带 refresh token；服务端撤销该 refresh token（server-side revocation），返回 `204`

## 数据存储

- 默认使用 SQLite：`data/gateway.db`
- 配置 `logging.pg_url` 后使用 Postgres（用于日志/缓存/用户/令牌等存储）
- RefreshToken 记录存储在 `refresh_tokens` 表（SQLite/Postgres 均支持），仅存 `token_hash`，不落库明文

## 开发检查

```bash
cargo check
```

## API 规范

接口与请求/响应以 `openapi.yaml` 为准（含 `/auth/login`、`/auth/register`、`/auth/refresh`、`/auth/logout` 等）。
