### 前后端对接

#### 项目最新状态（2025-12-30 更新）

**前端模块精简：**
- ✅ 保留模块：keys、channels、users、chats、dashboard、auth、settings
- ❌ 删除模块：apps（已在重构中移除）

#### 当前情况
目前的前端项目还没有类似的定义API的文件，只有数据模型的定义作为参考，如下：
  | 模块     | Schema 文件                          | 用途             |
  |----------|--------------------------------------|------------------|
  | Keys     | /home/Geoff001/Code/Project/captok/src/features/keys/data/schema.ts     | API 密钥数据结构 |
  | Channels | /home/Geoff001/Code/Project/captok/src/features/channels/data/schema.ts | 渠道数据结构     |
  | Users    | /home/Geoff001/Code/Project/captok/src/features/users/data/schema.ts    | 用户数据结构     |

**备注：** chats 和 dashboard 模块暂无 Schema 定义，使用模拟数据。
另外，src/lib/handle-server-error.ts 定义了前端期望的错误响应格式。

#### 后端数据模型
后端项目的数据模型定义分布在以下文件中：

| 模块 | 文件路径 | 主要数据结构 | 用途 |
|------|---------|-------------|------|
| 日志类型 | src/logging/types.rs | RequestLog, CachedModel, ProviderOpLog | 请求日志、模型缓存、提供商操作日志 |
| 管理令牌 | src/admin/mod.rs | AdminToken, CreateTokenPayload, UpdateTokenPayload | 管理员令牌管理、创建和更新 |
| 存储特征 | src/server/storage_traits.rs | AdminPublicKeyRecord, TuiSessionRecord, LoginCodeRecord, WebSessionRecord | 管理员密钥、TUI会话、登录码、Web会话 |
| 配置设置 | src/config/settings.rs | Provider, Settings, LoadBalancing, ServerConfig, LoggingConfig | 提供商配置、系统设置、负载均衡 |
0
#### 当前任务（实时更新）
1. 可以参考前端schema，将其转换为后端数据模型，为与后端数据库的调整做准备
2. 先拿后端API规范过来参考，与前端schema以及预期的api进行适配，记录差异并协商调整，为与后端API的对接做准备
#### 任务完成情况（实时更新）
1. 后端数据模型的定义和分布已指出
2. api规范文件已完成，路径：/home/Geoff001/Code/Project/Graduation_Project/gateway_zero/openapi.yaml

---

## 前后端 API 对比分析报告

> 📅 更新时间：2025-12-30
> 🔄 本次更新：前端项目精简（删除 apps 模块），核心模块 Schema 无变化

### 一、概念映射关系

| 前端模块 | 后端对应 | 映射关系 |
|---------|---------|---------|
| **Keys** (API 密钥) | **AdminToken** (管理员令牌) | ⚠️ 部分对应，字段差异大 |
| **Channels** (渠道) | **Provider** (提供商) | ⚠️ 概念相近，结构不同 |
| **Users** (用户) | ❌ 无对应 | 🔴 后端缺失用户管理 |

---

### 二、Keys ↔ AdminToken 字段对比

#### 2.1 字段映射表

| 前端字段 | 后端字段 | 状态 | 说明 |
|---------|---------|------|------|
| `id` | ❌ 无 | 🔴 缺失 | 后端用 `token` 作为唯一标识 |
| `name` | ❌ 无 | 🔴 缺失 | 后端无密钥名称字段 |
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
| 🔴 高 | 添加 `name` 字段 | 后端 | 密钥需要可读名称 |
| 🔴 高 | 添加 `id` 字段 | 后端 | 用于前端列表操作 |
| 🟡 中 | 添加 `remark` 字段 | 后端 | 备注功能常用 |
| 🟡 中 | 添加 IP 白/黑名单 | 后端 | 安全控制需求 |
| 🟢 低 | `isFavorite` | 前端存储 | 可用 localStorage |
| 🟢 低 | Token 统计字段 | 前端适配 | 展示后端统计数据 |

---

### 三、Channels ↔ Provider 字段对比

#### 3.1 字段映射表

| 前端字段 | 后端字段 | 状态 | 说明 |
|---------|---------|------|------|
| `id` | ❌ 无 | 🔴 缺失 | 后端用 `name` 作为标识 |
| `name` | `name` | 🟢 对应 | 完全一致 |
| `status` | ❌ 无 | 🔴 缺失 | 后端无启用/禁用状态 |
| `organizationId` | ❌ 无 | 🔴 缺失 | 后端无组织概念 |
| `upstreamEndpointType` | `api_type` | 🟡 转换 | 语义相近 |
| `gatewayEndpointType` | ❌ 无 | 🔴 缺失 | 后端无网关端点类型 |
| ❌ 无 | `base_url` | 🔵 新增 | 后端特有 |
| ❌ 无 | `api_keys` | 🔵 新增 | 后端特有 |
| ❌ 无 | `models_endpoint` | 🔵 新增 | 后端特有 |

**前端特有字段（后端缺失）：**

| 字段 | 说明 |
|-----|------|
| `performanceMetrics` | 性能指标（延迟、成功率、请求数） |
| `quota` / `usedQuota` | 额度管理 |
| `tags` | 标签分类 |
| `providerKeys` | 密钥轮换管理 |
| `keyRotationStrategy` | 密钥轮换策略 |
| `isFavorite` | 收藏功能 |

#### 3.2 调整建议

| 优先级 | 调整项 | 负责方 | 说明 |
|-------|-------|-------|------|
| 🔴 高 | 前端适配 `base_url` | 前端 | 必要配置字段 |
| 🔴 高 | 前端适配 `api_keys` | 前端 | 与 `providerKeys` 整合 |
| 🟡 中 | 添加 `status` 字段 | 后端 | 渠道启用/禁用控制 |
| 🟡 中 | 性能指标统计 | 后端 | 从日志聚合计算 |
| 🟢 低 | `tags` 标签 | 后端 | 可选功能 |

---

### 四、Users 模块分析

#### 4.1 现状

**后端完全缺失用户管理功能**，前端定义了完整的用户模型：

```typescript
// 前端 User Schema
{
  id, firstName, lastName, username, email, phoneNumber,
  status: 'active' | 'inactive' | 'invited' | 'suspended',
  role: 'superadmin' | 'admin' | 'cashier' | 'manager',
  createdAt, updatedAt
}
```

#### 4.2 调整建议

| 优先级 | 调整项 | 说明 |
|-------|-------|------|
| 🔴 高 | 后端新增用户管理模块 | 包含 CRUD 接口 |
| 🔴 高 | 实现认证授权系统 | JWT / Session |
| 🟡 中 | 角色权限控制 | RBAC 模型 |

---

### 五、API 端点对比

#### 5.1 后端已有端点

| 分类 | 端点 | 方法 | 说明 |
|-----|------|-----|------|
| **令牌管理** | `/admin/tokens` | GET | 获取令牌列表 |
| | `/admin/tokens` | POST | 创建令牌 |
| | `/admin/tokens/{token}` | GET/PATCH/DELETE | 令牌 CRUD |
| **提供商** | `/providers` | GET/POST | 提供商列表/创建 |
| | `/providers/{provider}` | GET/PUT/DELETE | 提供商 CRUD |
| | `/providers/{provider}/keys` | GET/POST/DELETE | 密钥管理 |
| **日志** | `/admin/logs` | GET | 请求日志 |
| **统计** | `/admin/metrics/usage` | GET | 使用量统计 |
| **价格** | `/admin/model-prices` | GET/POST | 模型价格 |

#### 5.2 前端需要但后端缺失的端点

| 模块 | 端点建议 | 方法 | 说明 |
|-----|---------|-----|------|
| **用户** | `/admin/users` | GET/POST | 用户列表/创建 |
| | `/admin/users/{id}` | GET/PUT/DELETE | 用户 CRUD |
| **认证** | `/auth/login` | POST | 用户登录 |
| | `/auth/logout` | POST | 用户登出 |
| | `/auth/me` | GET | 当前用户信息 |

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
├── 1. 前端创建 API 适配层（字段映射）
├── 2. 后端 AdminToken 添加 name、id 字段
└── 3. 对接 Keys 模块基础 CRUD

阶段二：功能完善
├── 4. 后端新增用户管理模块
├── 5. 实现认证授权系统
└── 6. 对接 Channels/Provider 模块

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