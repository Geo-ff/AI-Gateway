### 前后端对接

#### 项目最新状态（2026-01-10 更新）

✅ P0 完成清单：captok 已接入 Auth(JWT+refresh)+Users 真数据 CRUD+Keys/Token(=ClientToken) 真数据 CRUD+toggle；后端 OpenAPI/.env.example 已对齐并拆分前端/后端状态避免误判
✅ P1 完成清单：captok Channels 已对接 Provider 全量 CRUD+keys 管理（含 raw 列表用于删除）；后端 /providers* 响应已与 OpenAPI 对齐并统一错误/时间为 ISO-8601；workflow 状态已按“后端/前端/验证”拆分避免误判

**前端模块精简：**
- ✅ 保留模块：keys、channels、users、chats、dashboard、auth、settings
- ❌ 删除模块：apps（已在重构中移除）

#### 当前情况
前端已完成 Auth（JWT+refresh 单飞重试）接入，并已将 Users/Keys(Token=ClientToken)/Channels(Provider) 页面从 mock data 切换为真实接口（/admin/users*、/admin/tokens*、/providers*），Schema/DTO 定义如下：
  | 模块     | Schema 文件                          | 用途             |
  |----------|--------------------------------------|------------------|
  | Keys     | /home/Geoff001/Code/Project/captok/src/features/keys/data/schema.ts     | API 密钥数据结构 |
  | Channels | /home/Geoff001/Code/Project/captok/src/features/channels/api/providers-api.ts | Provider DTO↔Domain |
  | Users    | /home/Geoff001/Code/Project/captok/src/features/users/data/schema.ts    | 用户数据结构     |

**备注：** chats 和 dashboard 模块暂无 Schema 定义，使用模拟数据。
另外，src/lib/handle-server-error.ts 定义了前端期望的错误响应格式。

#### 后端数据模型
后端项目的数据模型定义分布在以下文件中：

| 模块 | 文件路径 | 主要数据结构 | 用途 |
|------|---------|-------------|------|
| 日志类型 | src/logging/types.rs | RequestLog, CachedModel, ProviderOpLog | 请求日志、模型缓存、提供商操作日志 |
| 管理令牌 | src/admin/mod.rs | ClientToken, CreateTokenPayload, UpdateTokenPayload | 客户端令牌（Client Token）管理、创建和更新 |
| 存储特征 | src/server/storage_traits.rs | AdminPublicKeyRecord, TuiSessionRecord, LoginCodeRecord, WebSessionRecord | 管理员密钥、TUI会话、登录码、Web会话 |
| 配置设置 | src/config/settings.rs | Provider, Settings, LoadBalancing, ServerConfig, LoggingConfig | 提供商配置、系统设置、负载均衡 |
| 用户管理 | src/users.rs | User, CreateUserPayload, UpdateUserPayload, UserStore | 用户数据结构与 CRUD 存储抽象 |

#### 当前任务（实时更新：后端实现 / 前端接入 / 已验证 拆分）

| 功能 | 后端实现状态 | 前端接入状态 | 已验证状态 | 备注 |
|------|-------------|-------------|-----------|------|
| Auth（JWT+refresh） | ✅ | ✅ | ✅（命令） | `/auth/login` `/auth/me` `/auth/refresh` `/auth/logout`；401 refresh 单飞重试 |
| Users（Admin Users CRUD） | ✅ | ✅ | ✅（命令） | `/admin/users*`；DTO↔Domain 映射 |
| Keys/Tokens（ClientToken CRUD+toggle） | ✅ | ✅ | ✅（命令） | `/admin/tokens*` `/admin/tokens/{id}/toggle`；expires_at 输入兼容 RFC3339/旧北京格式；时间输出 ISO-8601 |
| Channels（Providers CRUD+keys） | ✅ | ✅ | ✅（命令） | `/providers*` + `/providers/{provider}/keys(/raw)`；标识策略：`id = name`，不支持改名 |
| 错误响应结构 | ✅ | ✅（handle-server-error） | ✅（命令） | `{code,message}`；401/403/400/500 统一语义 |
| 时间字段输出 | ✅ | ✅（前端按 ISO-8601 解析） | ✅（命令） | 后端统一输出 ISO-8601 / RFC3339(UTC)；DB 仍存北京字符串 |

注：✅（命令）= `gateway_zero` 通过 `cargo fmt --check`/`cargo clippy -- -D warnings`/`cargo test`；`captok` 通过 `npm run lint`/`npm run build`（仍建议手动点 UI 链路再确认交互与提示）。

#### 接口测试记录（curl 冒烟 P0/P1）

- 测试时间(UTC)：2026-01-11T14:45:36Z
- BASE_URL：http://localhost:8080
- git：a167e14
- 脚本：`scripts/smoke_p0_p1.sh`
- 完整逐用例报告（含脱敏日志+逐接口断言）：`scripts/_smoke/smoke_20260111T144536Z_120918.md`
- 汇总：Pass=49 / Fail=0 / Total=49

| 覆盖链路 | 关键断言（摘录） | 结果 |
|---|---|---|
| 连通性 | `GET /auth/me` 预期 `401` 且 `{code,message}` | Pass |
| Auth（login/me/refresh/logout + rotation） | 必需字段存在（`accessToken/refreshToken/expiresAt/...`）；`expiresAt/refreshExpiresAt` 可解析；旧 refresh 二次 refresh=`401`；logout 后 refresh=`401` | Pass |
| Admin Users CRUD | `/admin/users*` 列表/创建/读取/更新/删除全链路；时间字段 `created_at/updated_at` 可解析 | Pass |
| Admin Tokens CRUD+toggle | `/admin/tokens*` 创建/读取/更新/删除；toggle 生效；`created_at` 可解析；不记录 token 明文 | Pass |
| Providers CRUD + keys（含 raw） | `/providers*` CRUD；`/keys` 增删查；`/keys/raw` 结构校验（不记录 key 原文） | Pass |
| 失败结构抽样 | `401/403/404` 各抽样 1+ 用例，均为 `{code,message}` | Pass |

<details><summary>关键日志片段（脱敏）</summary>

```text
== Gateway Zero curl smoke (P0/P1) ==
time_utc: 2026-01-11T14:45:36Z
git_sha : a167e14
base_url: http://localhost:8080
email   : mahougeg…(len=19)

CASE: C1 POST /auth/login -> 200
REQ : POST /auth/login (expect 200)
ACT : 200 => Pass

CASE: C3 POST /auth/refresh -> 200 (rotation)
REQ : POST /auth/refresh (expect 200)
ACT : 200 => Pass

CASE: D2 POST /admin/users -> 201
REQ : POST /admin/users (expect 201)
ACT : 201 => Pass

CASE: E5 POST /admin/tokens/{id}/toggle -> 200
REQ : POST /admin/tokens/atk_32aad85f82f2c4dc0902a3fc/toggle (expect 200)
ACT : 200 => Pass

CASE: F10 DELETE /providers/{provider} -> 200
REQ : DELETE /providers/smoke_20260111T144536Z_120918_provider (expect 200)
ACT : 200 => Pass
```
</details>

✅ 生成并执行 P0/P1 curl 冒烟测试并回填结果

#### 任务完成情况（实时更新）
1. ✅ 后端数据模型的定义和分布已指出
2. ✅ api规范文件已完成，路径：/home/Geoff001/Code/Project/Graduation_Project/gateway_zero/openapi.yaml
3. ✅ 后端已新增用户管理模块（Admin Users CRUD），并落库支持 SQLite/Postgres
   - 路由：`/admin/users`、`/admin/users/{id}`
   - 代码：`src/server/handlers/admin_users.rs`、`src/users.rs`
4. ✅ 已补充用户模块最小单元测试覆盖（`cargo test` 将包含 users 相关用例）
5. ✅ 前端已新增 Users API 适配层（DTO↔Domain）：`src/features/users/api/*`
6. ✅ 前端已新增最小请求层 `axios` client：`src/lib/api-client.ts`（自动附加 `Authorization: Bearer <token>`；baseURL 读取 `VITE_API_BASE_URL`）
7. ✅ ClientToken 已补齐 `id/name` 字段，并同步更新 OpenAPI、前端令牌管理页面与 TUI（路由按 `{id}` 操作）

---

## 前后端 API 对比分析报告

> 📅 更新时间：2025-12-30
> 🔄 本次更新：Channels 从 mock 切换为 Provider 真接口（CRUD+keys）；时间输出统一为 ISO-8601；错误结构对齐 handle-server-error

### 一、概念映射关系

| 前端模块 | 后端对应 | 映射关系 |
|---------|---------|---------|
| **Keys** (API 密钥) | **ClientToken** (客户端令牌) | ⚠️ 部分对应，字段差异大 |
| **Channels** (渠道) | **Provider** (提供商) | ✅ 已按 Provider 配置对接（P1：CRUD+keys） |
| **Users** (用户) | **User**（用户管理模块） | ✅ 后端已实现；✅ 前端已接入（字段映射 + CRUD） |

---

### 二、Keys ↔ ClientToken 字段对比

#### 2.1 字段映射表

| 前端字段 | 后端字段 | 状态 | 说明 |
|---------|---------|------|------|
| `id` | `id` | 🟢 对应 | 非敏感标识，用于管理端列表/操作（CRUD 按 `{id}`） |
| `name` | `name` | 🟢 对应 | 密钥可读名称（可选，不填则后端自动生成） |
| `status` | `enabled` | 🟡 转换 | 前端 enum，后端 boolean |
| `validFrom` | ❌ 无 | 🔴 缺失 | 后端无生效时间 |
| `validUntil` | `expires_at` | 🟢 对应 | 格式需统一 |
| `neverExpire` | ❌ 无 | 🟡 推导 | 可由 `expires_at=null` 推导 |
| `quota` | `max_amount` | 🟢 对应 | 语义一致 |
| `unlimitedQuota` | ❌ 无 | 🟡 推导 | 可由 `max_amount=null` 推导 |
| `usedQuota` | `amount_spent` | 🟢 对应 | 语义一致 |
| `remark` | ❌ 无 | 🔴 缺失 | 后端无备注字段 |
| `allowedModels` | `allowed_models` | 🟢 对应 | 完全一致 |
| `allowedIps` | ❌ 无 | 🔴 缺失 | 后端无 IP 白名单 |
| `blockedIps` | ❌ 无 | 🔴 缺失 | 后端无 IP 黑名单 |
| `organizationId` | ❌ 无 | 🔴 缺失 | 后端无组织概念 |
| `secretKey` | `token` | 🟢 对应 | 密钥字符串 |
| `createdAt` | `created_at` | 🟢 对应 | 命名风格不同 |
| `isFavorite` | ❌ 无 | 🔴 缺失 | 后端无收藏功能 |
| ❌ 无 | `prompt_tokens_spent` | 🔵 新增 | 后端特有统计 |
| ❌ 无 | `completion_tokens_spent` | 🔵 新增 | 后端特有统计 |
| ❌ 无 | `total_tokens_spent` | 🔵 新增 | 后端特有统计 |

#### 2.2 调整建议

| 优先级 | 调整项 | 负责方 | 说明 |
|-------|-------|-------|------|
| ✅ 完成 | 添加 `name` 字段 | 后端 | 密钥需要可读名称 |
| ✅ 完成 | 添加 `id` 字段 | 后端 | 用于前端列表操作（管理端接口按 `{id}`） |
| 🟡 中 | 添加 `remark` 字段 | 后端 | 备注功能常用 |
| 🟡 中 | 添加 IP 白/黑名单 | 后端 | 安全控制需求 |
| 🟢 低 | `isFavorite` | 前端存储 | 可用 localStorage |
| 🟢 低 | Token 统计字段 | 前端适配 | 展示后端统计数据 |

---

### 三、Channels ↔ Provider 字段对比

#### 3.1 字段映射表

| 前端字段 | 后端字段 | 状态 | 说明 |
|---------|---------|------|------|
| `id` | `name` | 🟢 已对齐 | 标识策略：前端 `id = name`；后端路径参数 `{provider}` 即 `name`（不支持改名） |
| `name` | `name` | 🟢 对应 | 完全一致 |
| `status` | ❌ 无 | 🔴 缺失 | 后端无启用/禁用状态 |
| `apiType` | `api_type` | 🟢 对应 | openai/anthropic/zhipu；未知值前端兜底为 unknown |
| `baseUrl` | `base_url` | 🟢 对应 | 必填配置字段 |
| `apiKeys` | `api_keys` | 🟢 对应 | `/providers*` 返回脱敏列表；原文列表用 `/providers/{provider}/keys/raw` |
| `modelsEndpoint` | `models_endpoint` | 🟢 对应 | 可选字段 |

**P1 已排除/留到 P2+ 的字段（后端缺失或未做）：**

| 字段 | 说明 |
|-----|------|
| `organizationId` | 组织维度（P2+） |
| `collectionId` | 合集（P2+） |
| `gatewayEndpointType` | 网关端点类型（P2+） |
| `performanceMetrics` | 性能指标（延迟、成功率、请求数）（P2+） |
| `quota` / `usedQuota` | 额度管理（P2+） |
| `tags` | 标签分类（P2+） |
| `providerKeys` | 密钥轮换/权重等高级管理（P2+；P1 仅做 keys 增删查） |
| `keyRotationStrategy` | 密钥轮换策略（P2+） |
| `isFavorite` | 收藏功能（P2+） |

#### 3.2 调整建议

| 优先级 | 调整项 | 负责方 | 说明 |
|-------|-------|-------|------|
| ✅ 完成 | 前端适配 `base_url` | 前端 | 已在 Channels 页面中作为必填配置字段 |
| ✅ 完成 | 前端适配 `api_keys` | 前端 | 已通过 `/providers/{provider}/keys(/raw)` 管理，列表/新增/删除可用 |
| 🟡 中 | 添加 `status` 字段 | 后端 | 渠道启用/禁用控制 |
| 🟡 中 | 性能指标统计 | 后端 | 从日志聚合计算 |
| 🟢 低 | `tags` 标签 | 后端 | 可选功能 |

---

### 四、Users 模块分析

#### 4.1 现状

后端已实现用户管理模块（Admin Users CRUD），前端定义的用户模型可以直接对接，但需要做字段映射：

```typescript
// 前端 User Schema
{
  id, firstName, lastName, username, email, phoneNumber,
  status: 'active' | 'inactive' | 'invited' | 'suspended',
  role: 'superadmin' | 'admin' | 'cashier' | 'manager',
  createdAt, updatedAt
}
```

后端返回字段（snake_case）：
- `first_name` ↔ `firstName`
- `last_name` ↔ `lastName`
- `phone_number` ↔ `phoneNumber`
- `created_at` ↔ `createdAt`
- `updated_at` ↔ `updatedAt`

#### 4.2 调整建议

| 优先级 | 调整项 | 说明 |
|-------|-------|------|
| ✅ 完成 | 前端补齐字段映射与请求层 | Users 模块已从 mock data 迁移到真实接口（/admin/users*） |
| ✅ 完成 | 实现认证授权系统 | JWT + refresh token rotation + logout revocation；前端已接入并支持 401 自动续期 |
| 🟡 中 | 角色权限控制 | RBAC 模型 |

---

### 五、API 端点对比

#### 5.1 后端已有端点

| 分类 | 端点 | 方法 | 说明 |
|-----|------|-----|------|
| **令牌管理** | `/admin/tokens` | GET | 获取令牌列表 |
| | `/admin/tokens` | POST | 创建令牌 |
| | `/admin/tokens/{id}` | GET/PUT/DELETE | 令牌 CRUD（按 id） |
| | `/admin/tokens/{id}/toggle` | POST | 启用/禁用令牌 |
| **提供商** | `/providers` | GET/POST | 提供商列表/创建 |
| | `/providers/{provider}` | GET/PUT/DELETE | 提供商 CRUD |
| | `/providers/{provider}/keys` | GET/POST/DELETE | 密钥管理 |
| **日志** | `/admin/logs` | GET | 请求日志 |
| **统计** | `/admin/metrics/usage` | GET | 使用量统计 |
| **价格** | `/admin/model-prices` | GET/POST | 模型价格 |

#### 5.2 前端需要的认证端点（后端已实现，前端已接入）

| 模块 | 端点 | 方法 | 状态/说明 |
|-----|---------|-----|------|
| **认证** | `/auth/login` | POST | ✅ 后端已实现；✅ 前端已接入 |
| | `/auth/logout` | POST | ✅ 后端已实现；✅ 前端已接入 |
| | `/auth/me` | GET | ✅ 后端已实现；✅ 前端已接入 |
| | `/auth/refresh` | POST | ✅ 后端已实现；✅ 前端已接入（401 自动刷新后重试） |

---

### 六、总结与行动计划

#### 6.1 差异统计

| 类型 | 数量 |
|-----|------|
| 🟢 可直接对接 | 5 个字段 |
| 🟡 需转换适配 | 4 个字段 |
| 🔴 后端需新增 | 12+ 个字段 |
| 🔵 前端需适配 | 6 个字段 |

#### 6.2 推荐行动顺序

```
阶段一：基础对接（优先）
├── 1. ✅ 前端创建 API 适配层（字段映射） 
├── 1.1 ✅ 配置 `VITE_API_BASE_URL` 并接入 Users 页面（替换 mock data）
├── 2. ✅ 后端 ClientToken 添加 name、id 字段（管理端 CRUD 按 `{id}`）
└── 3. ✅ 对接 Keys/Token（ClientToken）管理端 CRUD + toggle（/admin/tokens*）

阶段二：功能完善
├── 4. ✅ 后端新增用户管理模块（已完成）
├── 5. ✅ 实现认证授权系统（邮件密码找回前端还未实现界面对接）
└── 6. ✅ 对接 Channels/Provider 模块（/providers* + keys 管理）

阶段三：增强功能
├── 7. IP 白/黑名单功能
├── 8. 性能指标统计
└── 9. 标签、收藏等辅助功能
```

#### 6.3 命名风格统一建议

| 位置 | 当前风格 | 建议 |
|-----|---------|------|
| 前端字段 | camelCase | 保持 |
| 后端字段 | snake_case | 保持 |
| 适配层 | 自动转换 | 前端添加映射函数 |

---

> **备注**：本文档将随对接进度持续更新

✅ 术语对齐：统一 Client Token / Admin Identity 术语，并同步更新 OpenAPI 与文档（全局无残留）

- ✅ P0：新增 `/auth/login` `/auth/me` `/auth/logout`（JWT AccessToken），管理端路由支持 Bearer JWT，并对齐 401/403 鉴权错误码
- ✅ P1：users 表新增 password_hash（首个用户即管理员），前端改为调用 /auth/login 获取 JWT 驱动 accessToken
- ✅ 鉴权一致性：/admin/* 与 /providers/* 全量切到 JWT 优先鉴权，并统一 401/403 返回码
- ✅ P2：/auth/register 增加 bootstrap code；引入 refresh token + rotation + server-side revocation，前端接入无感续期与 logout 撤销
- ✅ P3：落地 RBAC v1（仅 superadmin 可访问 /admin/* /providers/*），ClientToken 绑定 user_id 并新增用户侧只读/自管接口（/model-prices、/me/*、/auth/change-password）
- ✅ P4：实现 Resend 邮件找回密码（/auth/forgot-password + /auth/reset-password），reset token 一次性可过期且仅存 hash，重置后撤销 refresh tokens
- ✅ P5：文档声明 gateway_zero 内置 frontend/frontend_tui 为过渡形态，captok 为长期主前端并规划弃用旧前端
