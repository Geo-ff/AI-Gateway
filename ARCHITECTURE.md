# Gateway Zero 后端架构分析

## 项目概述

**Gateway Zero** 是一个基于 Rust 和 Axum 框架开发的 AI API 网关系统。它提供统一的 OpenAI 兼容接口，支持多个 AI 提供商（OpenAI、Anthropic、智谱 AI 等），并具备负载均衡、令牌管理、用量统计等企业级功能。

## 技术栈

- **核心框架**: Axum 0.8 + Tokio (异步运行时)
- **HTTP 客户端**: Reqwest (支持流式响应)
- **数据库**: PostgreSQL (主) / SQLite (备选)
- **序列化**: Serde + JSON + TOML
- **加密**: Ed25519-dalek (数字签名)
- **日志**: Tracing + Tracing-subscriber
- **AI SDK**: async-openai, anthropic-ai-sdk

## 系统架构

### 1. 客户端层

系统支持三种类型的客户端：

- **Web 前端**: 基于 React/Vue 的管理控制台
- **TUI 客户端**: 终端界面，使用 Ed25519 签名认证
- **第三方应用**: 任何支持 OpenAI API 的应用（使用 Admin Token 认证）

### 2. 网关入口层

#### Axum HTTP Server
- **监听地址**: 可配置 `{host}:{port}`
- **异步运行时**: Tokio（全异步 I/O）
- **路由**: 基于 Axum Router

#### 中间件栈（按顺序）
1. **CORS 中间件**: 跨域资源共享，支持开发环境的前端联调
2. **认证中间件**: 验证 Admin Token 或 TUI Session
3. **请求日志**: 记录所有请求到数据库
4. **Tower Trace**: 分布式追踪和性能监控
5. **路由分发**: 将请求分发到对应的处理器

### 3. 路由与认证层

#### 认证模块 (`src/admin/`, `src/server/login.rs`)
- **Admin Token 验证**: 
  - 支持令牌白名单、额度限制、过期时间
  - 追踪用量和成本
- **TUI Session 管理**:
  - Challenge-Response 认证流程
  - Ed25519 数字签名验证
  - Session 生命周期管理
- **密钥管理**:
  - 管理员公钥存储和指纹识别
  - 自动生成初始管理员密钥

#### API 处理器 (`src/server/handlers/`)

| 处理器 | 功能 | 主要端点 |
|--------|------|---------|
| **chat.rs** | 聊天补全 | `POST /v1/chat/completions` |
| **models.rs** | 模型列表 | `GET /v1/models` |
| **token_info.rs** | 令牌信息 | `GET /v1/token/balance` |
| **admin_tokens.rs** | 令牌管理 | `/admin/tokens/*` |
| **providers.rs** | 提供商管理 | `/providers/*` |
| **provider_keys.rs** | API Key 管理 | `/providers/{provider}/keys` |
| **admin_metrics.rs** | 统计分析 | `/admin/metrics/*` |
| **admin_logs.rs** | 日志查询 | `/admin/logs/*` |
| **cache.rs** | 模型缓存 | `/models/{provider}/cache` |
| **admin_prices.rs** | 价格管理 | `/admin/model-prices` |

#### 配置管理 (`src/config/`)
- **TOML 配置文件**: `custom-config.toml`, `redirect.toml`
- **模型重定向规则**: 将特定模型名映射到其他模型
- **负载均衡策略**: FirstAvailable, RoundRobin, Random

### 4. 业务逻辑层

#### 模型解析 (`src/server/model_parser.rs`)
```
支持格式: provider/model
示例: openai/gpt-4, anthropic/claude-3-5-sonnet-20241022
```
- 解析提供商前缀和模型名称
- 应用模型重定向规则
- 生成上游 API 调用的实际模型名

#### 负载均衡器 (`src/routing/load_balancer.rs`)

| 策略 | 描述 |
|------|------|
| **FirstAvailable** | 总是选择第一个可用的提供商和 API Key |
| **RoundRobin** | 轮询所有可用提供商和 API Key |
| **Random** | 随机选择提供商和 API Key |

**特性**:
- 支持多提供商和多 API Key
- 自动过滤禁用的提供商
- 原子计数器保证线程安全的轮询

#### 提供商调度 (`src/server/provider_dispatch.rs`)
1. **选择提供商**: 
   - 如果请求指定了提供商前缀，直接使用该提供商
   - 否则使用负载均衡器选择
2. **请求适配**: 将 OpenAI 格式转换为目标提供商格式
3. **响应转换**: 将提供商响应转换回 OpenAI 格式

#### 流式处理 (`src/server/streaming/`)
- **SSE (Server-Sent Events)**: 实时流式响应
- **Token 计数**: 统计输入/输出 token 数量
- **错误处理**: 捕获流中的错误并优雅降级

### 5. AI 提供商层

#### OpenAI Provider (`src/providers/openai.rs`)
- **支持模型**: GPT-4, GPT-3.5-turbo, GPT-4-turbo 等
- **兼容 API**: 支持 OpenRouter、DeepSeek 等 OpenAI 兼容接口
- **流式支持**: 完整的 SSE 流式响应

#### Anthropic Provider (`src/providers/anthropic.rs`)
- **支持模型**: Claude 3.5 Sonnet, Claude 3 Opus 等
- **格式转换**: 
  - 请求: OpenAI → Anthropic Messages API
  - 响应: Anthropic → OpenAI Chat Completion
- **流式支持**: 支持 Anthropic 的流式事件

#### Zhipu Provider (`src/providers/zhipu.rs`)
- **支持模型**: GLM-4, GLM-4-flash 等
- **适配**: 智谱 AI 的特殊参数和响应格式
- **本地化**: 支持中文模型和场景

### 6. 存储层

#### PostgreSQL / GaussDB (主存储)
```sql
表结构:
- request_logs: 请求日志 (时间、模型、用量、成本)
- model_cache: 模型缓存 (提供商、模型列表、更新时间)
- providers: 提供商配置 (名称、类型、地址、密钥)
- admin_tokens: 令牌管理 (令牌、权限、额度、统计)
- admin_keys: 管理员密钥 (公钥、指纹、启用状态)
- model_prices: 模型价格 (输入/输出单价)
- tui_sessions: TUI 会话
- operation_logs: 操作日志
```

**特性**:
- 连接池管理 (配置池大小)
- 自动 Schema 迁移
- Keepalive 心跳防止连接回收
- 支持自定义 Schema

#### SQLite (备选存储)
- **文件路径**: `data/gateway.db`
- **使用场景**: 
  - 开发环境
  - 单机部署
  - PostgreSQL 不可用时的回退方案
- **限制**: 不支持并发写入优化

### 7. 支持服务层

#### 日志系统 (`src/logging/`)
- **Tracing**: 结构化日志和分布式追踪
- **北京时间**: 自定义时间格式化器
- **日志级别**: 通过环境变量 `RUST_LOG` 配置

#### 加密模块 (`src/crypto/`)
- **Ed25519 签名**: 
  - TUI 客户端认证
  - Challenge-Response 防重放攻击
  - 公钥指纹识别

#### 监控统计
- **使用量统计**: 
  - Prompt tokens (输入)
  - Completion tokens (输出)
  - Total tokens (总计)
- **成本追踪**: 
  - 基于模型价格计算
  - 支持自定义定价
  - 按令牌/时间/模型聚合

#### 错误处理 (`src/error.rs`)
```rust
enum GatewayError {
    Config,      // 配置错误
    Db,          // 数据库错误
    Auth,        // 认证错误
    NotFound,    // 资源不存在
    Provider,    // 提供商错误
    ...
}
```
- 统一错误类型
- 自动转换为 HTTP 状态码
- 详细错误信息返回

## 核心特性

### 1. 统一接口
- ✅ 提供 OpenAI 兼容的 API 接口
- ✅ 一套代码接入多个 AI 提供商
- ✅ 无需修改客户端代码即可切换提供商

### 2. 智能路由
- ✅ 模型名称解析 (`provider/model` 格式)
- ✅ 模型重定向规则 (配置文件管理)
- ✅ 自动选择可用提供商
- ✅ 负载均衡策略可配置

### 3. 多 API Key 管理
- ✅ 每个提供商支持多个 API Key
- ✅ Key 级别的负载均衡
- ✅ 自动重试和故障转移
- ✅ Key 使用统计和日志

### 4. 令牌管理
- ✅ 创建、更新、删除令牌
- ✅ 模型白名单限制
- ✅ 额度限制 (金额/Token 数)
- ✅ 过期时间设置
- ✅ 实时用量统计

### 5. 安全认证
- ✅ Admin Token 认证（API 调用）
- ✅ Ed25519 签名认证（TUI 客户端）
- ✅ Session 管理和撤销
- ✅ 防重放攻击（Challenge-Response）

### 6. 监控与日志
- ✅ 完整的请求日志（时间、模型、用量、成本）
- ✅ 聊天补全详细记录
- ✅ 操作审计日志
- ✅ 统计分析（按时间/模型/令牌聚合）
- ✅ 模型使用分布图表

### 7. 流式支持
- ✅ SSE 流式响应
- ✅ 实时 Token 计数
- ✅ 流中错误处理
- ✅ 兼容所有提供商的流式接口

### 8. 灵活存储
- ✅ PostgreSQL 生产环境
- ✅ SQLite 开发/测试环境
- ✅ 自动 Schema 迁移
- ✅ 连接池和性能优化

## 请求处理流程

```
1. 客户端发送请求 
   ↓
2. Axum 接收并通过中间件栈
   ↓ (CORS → Auth → Logging → Tracing → Routing)
3. 路由到对应的 Handler
   ↓
4. 验证 Admin Token / TUI Session
   ↓
5. 解析模型名称（provider/model）
   ↓
6. 应用模型重定向规则
   ↓
7. 负载均衡器选择提供商和 API Key
   ↓
8. 提供商调度器适配请求格式
   ↓
9. 调用目标 AI 提供商 API
   ↓
10. 转换响应格式为 OpenAI 格式
    ↓
11. 记录请求日志、更新统计
    ↓
12. 返回响应给客户端
```

## 配置示例

### custom-config.toml
```toml
[server]
host = "0.0.0.0"
port = 8080

[load_balancing]
strategy = "RoundRobin"  # FirstAvailable | RoundRobin | Random

[logging]
database_path = "data/gateway.db"
pg_url = "postgresql://user:pass@localhost/gateway"  # 可选
pg_schema = "public"
pg_pool_size = 4
key_log_strategy = "masked"  # full | masked | none
```

### redirect.toml
```toml
[[redirect]]
from = "gpt-4"
to = "openai/gpt-4-turbo"

[[redirect]]
from = "claude"
to = "anthropic/claude-3-5-sonnet-20241022"
```

## 性能优化

1. **异步 I/O**: 全异步架构，高并发支持
2. **连接池**: 数据库连接复用，减少开销
3. **流式响应**: 降低首字节延迟
4. **模型缓存**: 减少重复查询
5. **负载均衡**: 分散请求，避免单点瓶颈

## 部署建议

### 开发环境
```bash
# 使用 SQLite
RUST_LOG=info cargo run
```

### 生产环境
```bash
# 使用 PostgreSQL
RUST_LOG=warn cargo run --release

# Docker 部署
docker build -t gateway-zero .
docker run -p 8080:8080 \
  -e RUST_LOG=info \
  -v ./data:/app/data \
  gateway-zero
```

### 高可用部署
- 多实例部署 + 负载均衡器（Nginx/HAProxy）
- PostgreSQL 主从复制
- Redis 缓存层（可选）
- 监控告警（Prometheus + Grafana）

## 扩展性

### 添加新的 AI 提供商
1. 在 `src/providers/` 创建新模块
2. 实现请求/响应转换
3. 在 `src/server/provider_dispatch.rs` 添加调度逻辑
4. 更新 `ProviderType` 枚举

### 添加新的存储后端
1. 实现 `storage_traits.rs` 中的 trait
2. 在 `src/server/mod.rs` 的 `create_app` 中添加选择逻辑

### 添加新的认证方式
1. 在 `src/server/handlers/` 添加新的认证 handler
2. 实现认证逻辑和 Session 管理
3. 更新路由配置

## 安全注意事项

1. **API Key 保护**: 
   - 数据库中明文存储（考虑加密）
   - 日志中默认脱敏
   - 仅通过管理接口访问

2. **Admin Token**: 
   - 使用强随机生成器
   - 设置合理的过期时间
   - 定期轮换

3. **TUI Session**: 
   - Ed25519 签名保证身份
   - Challenge-Response 防重放
   - Session 过期自动失效

4. **HTTPS**: 
   - 生产环境必须使用 HTTPS
   - 建议前置 Nginx 处理 TLS

## 监控指标

- 请求总数和成功率
- 平均响应时间
- Token 使用量和成本
- 各提供商使用分布
- API Key 健康状态
- 数据库连接池状态

## 总结

Gateway Zero 是一个功能完整、架构清晰的 AI API 网关系统。它通过统一接口、智能路由、负载均衡等特性，简化了多 AI 提供商的集成和管理。基于 Rust 的高性能实现和灵活的存储方案，使其能够适应从开发到生产的各种场景。
