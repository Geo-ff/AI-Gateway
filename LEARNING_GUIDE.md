# Gateway Zero 项目学习指南

## 📚 学习方法论：五步消化法

### 第一步：理解业务价值（Why - 为什么需要它）

#### 核心问题
AI 应用开发者面临的痛点：
- 🔄 **多提供商管理复杂**：OpenAI、Anthropic、智谱AI 等，每个 API 格式不同
- 💰 **成本难以控制**：每个团队成员都持有 API Key，无法统一管理和监控
- 📊 **使用情况不透明**：不知道谁用了多少 tokens，花了多少钱
- ⚡ **可用性不稳定**：单一 API Key 有速率限制，容易被封禁
- 🔐 **安全隐患**：API Key 分散在各处，泄露风险高

#### Gateway Zero 的解决方案
```
传统方式：
应用 → OpenAI API (需要修改代码)
应用 → Anthropic API (需要修改代码)
应用 → 智谱 API (需要修改代码)

使用 Gateway Zero：
应用 → Gateway (统一 OpenAI 格式) → 自动路由到最佳提供商
     ↓
   监控、限额、日志、统计
```

**类比**：就像酒店前台，客人不需要知道房间具体在哪，前台负责分配、记录、管理。

---

### 第二步：掌握核心概念（What - 它是什么）

#### 6 个核心概念

##### 1. 提供商（Provider）
```rust
// 配置示例
Provider {
    name: "openai-main",
    api_type: "OpenAI",
    base_url: "https://api.openai.com/v1",
    api_keys: ["sk-xxx", "sk-yyy"],  // 支持多个 Key
    enabled: true
}
```
**理解要点**：
- 一个提供商 = 一个 AI 服务商配置
- 可以配置多个同类型提供商（如 openai-backup）
- 每个提供商可以有多个 API Key（负载均衡）

##### 2. 模型名称解析
```
客户端请求格式：provider/model
例如：openai/gpt-4
      anthropic/claude-3-5-sonnet-20241022
      gpt-4  （不指定 provider，自动负载均衡）

网关解析：
openai/gpt-4 → 直接路由到 openai provider，调用 gpt-4
gpt-4        → 负载均衡器选择可用 provider，调用 gpt-4
```

##### 3. 负载均衡策略
```rust
enum BalanceStrategy {
    FirstAvailable,  // 总是用第一个
    RoundRobin,      // 轮流使用（1→2→3→1）
    Random           // 随机选择
}
```
**适用场景**：
- FirstAvailable：简单场景，有主备概念
- RoundRobin：流量均衡，避免单 Key 限流
- Random：避免"踩踏效应"，更分散

##### 4. Admin Token
```
作用：客户端调用网关的凭证（类似酒店房卡）
功能：
  - 权限控制（allowed_models: ["gpt-4"]）
  - 额度限制（max_amount: 100.0）
  - 过期时间（expires_at: "2024-12-31"）
  - 使用统计（已花费多少 tokens/金额）
```

##### 5. 认证方式
```
两种认证：
1. Admin Token 认证（API 调用）
   Authorization: Bearer admin-token-xxx
   
2. Ed25519 签名认证（TUI 管理界面）
   Challenge-Response 机制，防重放攻击
```

##### 6. 流式响应（Streaming）
```
非流式：等待全部生成完成才返回（慢）
流式：边生成边返回（快，像打字机）

实现：Server-Sent Events (SSE)
data: {"choices":[{"delta":{"content":"你"}}]}
data: {"choices":[{"delta":{"content":"好"}}]}
data: [DONE]
```

---

### 第三步：跟踪主流程（How - 它如何工作）

#### 🔍 实践任务：手动追踪一个请求

**场景**：客户端调用 `POST /v1/chat/completions`，请求 `gpt-4` 模型

#### 文件阅读顺序（按流程）

##### 1. 入口：`src/main.rs`
```rust
// 启动流程
1. 初始化日志系统
2. 加载配置文件（custom-config.toml）
3. 创建 Axum 应用（create_app）
4. 绑定端口，启动服务器
```
**关键代码**：
```rust
let config = config::Settings::load()?;
let app = server::create_app(config).await?;
axum::serve(listener, app).await?;
```

##### 2. 应用初始化：`src/server/mod.rs::create_app()`
```rust
// 做了什么
1. 连接数据库（PostgreSQL 或 SQLite）
2. 初始化存储层（日志/缓存/令牌/提供商）
3. 确保管理员密钥存在
4. 创建 AppState（全局共享状态）
5. 加载路由（handlers::routes()）
6. 添加 CORS 中间件
```
**关键概念**：`AppState` 包含了所有共享资源
```rust
struct AppState {
    config: Settings,           // 配置
    log_store: Arc<dyn RequestLogStore>,
    model_cache: Arc<dyn ModelCache>,
    providers: Arc<dyn ProviderStore>,
    token_store: Arc<dyn TokenStore>,
    login_manager: Arc<LoginManager>
}
```

##### 3. 路由定义：`src/server/handlers/mod.rs::routes()`
```rust
// 找到我们关注的路由
.route("/v1/chat/completions", post(chat::chat_completions))
```
跳转到 → `src/server/handlers/chat.rs`

##### 4. 请求处理：`src/server/handlers/chat.rs::chat_completions()`
```rust
// 伪代码流程
async fn chat_completions(
    State(state): State<Arc<AppState>>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(request): Json<ChatCompletionRequest>
) -> Result<Response> {
    // 步骤 1：验证 Admin Token
    let token = extract_token(&auth)?;
    let admin_token = state.token_store.get_token(&token).await?;
    check_token_valid(&admin_token)?;  // 检查启用/过期/额度
    
    // 步骤 2：模型解析和重定向
    let model_name = apply_redirect(&request.model)?;
    
    // 步骤 3：选择提供商
    let (provider, parsed_model) = 
        select_provider_for_model(&state, &model_name).await?;
    
    // 步骤 4：调用提供商 API
    let response = 
        call_provider_with_parsed_model(&provider, &request, &parsed_model).await?;
    
    // 步骤 5：记录日志和统计
    log_request(&state, &request, &response).await?;
    update_token_usage(&state, &token, &response.usage).await?;
    
    // 步骤 6：返回响应
    Ok(Json(response))
}
```

##### 5. 提供商选择：`src/server/provider_dispatch.rs`
```rust
// 两种模式
if parsed_model.has_provider_prefix() {
    // 模式 1：指定了提供商（openai/gpt-4）
    直接使用该提供商
} else {
    // 模式 2：未指定提供商（gpt-4）
    调用负载均衡器选择
}
```
跳转到 → `src/routing/load_balancer.rs`

##### 6. 负载均衡：`src/routing/load_balancer.rs`
```rust
impl LoadBalancer {
    pub fn select_provider(&self) -> Result<SelectedProvider> {
        match self.strategy {
            RoundRobin => {
                // 原子递增计数器
                let index = self.counter.fetch_add(1) % self.providers.len();
                &self.providers[index]
            }
            // ... 其他策略
        }
        
        // 同样的策略选择 API Key
        let api_key = select_api_key(provider);
        
        Ok(SelectedProvider { provider, api_key })
    }
}
```

##### 7. 调用提供商：`src/providers/openai.rs`（以 OpenAI 为例）
```rust
pub async fn chat_completions(
    base_url: &str,
    api_key: &str,
    request: &ChatCompletionRequest
) -> Result<ChatCompletion> {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(request)
        .send()
        .await?;
    
    response.json().await
}
```

#### 📊 流程图总结
```
客户端请求
    ↓
[CORS 中间件] → 检查跨域
    ↓
[认证中间件] → 验证 Admin Token
    ↓
[路由] → /v1/chat/completions
    ↓
[chat_completions handler]
    ├─ 模型解析（openai/gpt-4）
    ├─ 模型重定向（gpt-4 → gpt-4-turbo）
    ├─ 负载均衡（选择 provider + api_key）
    ├─ 请求适配（OpenAI 格式 → Anthropic 格式）
    ├─ 调用上游 API
    ├─ 响应转换（Anthropic 格式 → OpenAI 格式）
    └─ 记录日志和统计
    ↓
返回响应给客户端
```

---

### 第四步：实践操作（Learn by Doing）

#### 任务 1：本地运行项目（30 分钟）

```bash
# 1. 检查配置文件
cat custom-config.toml

# 2. 确保 data 目录存在
mkdir -p data

# 3. 使用 SQLite 启动（开发模式）
RUST_LOG=debug cargo run

# 4. 查看启动日志，找到监听地址
# 输出：Gateway server running on http://0.0.0.0:8080
```

**期望输出**：
```
[INFO] Using SQLite for logs and cache
[INFO] Gateway server running on http://0.0.0.0:8080
```

---

#### 任务 2：创建第一个 Admin Token（15 分钟）

**⚠️ 重要**：创建 Token 的接口需要认证，这里有个"先有鸡还是先有蛋"的问题。有三种解决方案：

##### 方案 A：直接操作数据库（最简单，推荐学习用）

```bash
# 停止服务器（Ctrl+C），直接在数据库创建 token
sqlite3 data/gateway.db

# 在 SQLite 提示符下执行：
CREATE TABLE IF NOT EXISTS admin_tokens (
    token TEXT PRIMARY KEY,
    allowed_models TEXT,
    max_tokens BIGINT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at TEXT,
    created_at TEXT NOT NULL,
    max_amount DOUBLE PRECISION,
    amount_spent DOUBLE PRECISION DEFAULT 0,
    prompt_tokens_spent BIGINT DEFAULT 0,
    completion_tokens_spent BIGINT DEFAULT 0,
    total_tokens_spent BIGINT DEFAULT 0
);

INSERT INTO admin_tokens (
    token, 
    enabled, 
    created_at, 
    amount_spent, 
    prompt_tokens_spent, 
    completion_tokens_spent, 
    total_tokens_spent
)
VALUES (
    'my-test-token-12345', 
    1, 
    '2024-12-12 00:00:00', 
    0, 
    0, 
    0, 
    0
);

# 查看结果
SELECT * FROM admin_tokens;

# 退出 SQLite
.exit
```

**然后重启服务器**：
```bash
RUST_LOG=debug cargo run
```

##### 方案 B：使用自动生成的管理员密钥（生产方式）

项目启动时会自动生成管理员密钥对，但这需要通过 TUI 客户端使用。

```bash
# 1. 查看自动生成的私钥
cat data/admin_ed25519.key

# 2. 需要启动 TUI 客户端（frontend_tui）才能使用这个密钥
# 这个方式比较复杂，暂时跳过
```

##### 方案 C：修改代码临时禁用认证（仅开发）

可以临时注释掉认证中间件，但不推荐。

---

#### 任务 3：测试 Token 是否可用（5 分钟）

**在终端执行**：
```bash
# 测试 token 信息接口
curl http://localhost:8080/v1/token/balance \
  -H "Authorization: Bearer my-test-token-12345"

# 期望输出（token 存在但没有额度限制）
{
  "balance": null,
  "usage": {
    "total_tokens": 0,
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "amount_spent": 0.0
  }
}
```

**如果返回 401 Unauthorized**：
- 检查 token 是否正确
- 检查数据库中 enabled 字段是否为 1
- 检查服务器是否正常运行

---

#### 任务 4：添加一个 Provider（20 分钟）

**在终端执行**（需要你自己的真实 API Key）：

```bash
# 1. 创建 Provider
curl -X POST http://localhost:8080/providers \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-openai",
    "api_type": "OpenAI",
    "base_url": "https://api.openai.com/v1",
    "enabled": true
  }'

# 期望输出
{
  "name": "my-openai",
  "api_type": "OpenAI",
  "base_url": "https://api.openai.com/v1",
  "enabled": true
}

# 2. 添加 API Key（替换为你的真实 Key）
curl -X POST http://localhost:8080/providers/my-openai/keys \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "key": "sk-proj-xxxxxxxxxxxxxxxx",
    "description": "My first OpenAI key"
  }'

# 期望输出
{
  "masked_key": "sk-proj-****xxxx",
  "description": "My first OpenAI key"
}

# 3. 查看所有 Providers
curl http://localhost:8080/providers \
  -H "Authorization: Bearer my-test-token-12345"
```

**⚠️ 注意**：
- 需要一个真实的 OpenAI API Key（或兼容 API）
- 如果没有 OpenAI Key，可以用其他免费的兼容 API（如智谱、DeepSeek）
- API Key 会存储在数据库中（当前是明文，生产环境需加密）

---

#### 任务 5：发送第一个 Chat 请求（15 分钟）

**在终端执行**：

```bash
# 发送一个简单的聊天请求
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "my-openai/gpt-3.5-turbo",
    "messages": [
      {"role": "user", "content": "Hello! Say hi back in one sentence."}
    ]
  }'

# 期望输出（实际内容会不同）
{
  "id": "chatcmpl-xxx",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "gpt-3.5-turbo",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! It'\''s great to hear from you!"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 15,
    "completion_tokens": 10,
    "total_tokens": 25
  }
}
```

**如果失败**：
- 检查 Provider 是否创建成功
- 检查 API Key 是否有效
- 查看服务器日志（RUST_LOG=debug 模式）
- 确认上游 API 可访问

---

#### 任务 6：查看日志和统计（10 分钟）

**在终端执行**：
```bash
# 1. 查看请求日志（最近的请求）
curl http://localhost:8080/admin/logs/requests \
  -H "Authorization: Bearer my-test-token-12345"

# 2. 查看 Token 使用情况（统计你的用量）
curl http://localhost:8080/v1/token/usage \
  -H "Authorization: Bearer my-test-token-12345"

# 期望输出
{
  "total_tokens": 25,
  "prompt_tokens": 15,
  "completion_tokens": 10,
  "amount_spent": 0.00025
}

# 3. 查看统计摘要（全局统计）
curl http://localhost:8080/admin/metrics/summary \
  -H "Authorization: Bearer my-test-token-12345"

# 4. 查看模型使用分布
curl http://localhost:8080/admin/metrics/models-distribution \
  -H "Authorization: Bearer my-test-token-12345"
```

---

#### 任务 7：测试流式响应（15 分钟）

**在终端执行**：
```bash
# 发送流式请求（注意 "stream": true）
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "my-openai/gpt-3.5-turbo",
    "messages": [
      {"role": "user", "content": "Count from 1 to 5, one number per line"}
    ],
    "stream": true
  }'

# 观察 SSE 流式输出（逐字输出）
# data: {"id":"chatcmpl-xxx","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}
# data: {"id":"chatcmpl-xxx","choices":[{"index":0,"delta":{"content":"1"},"finish_reason":null}]}
# data: {"id":"chatcmpl-xxx","choices":[{"index":0,"delta":{"content":"\n"},"finish_reason":null}]}
# data: {"id":"chatcmpl-xxx","choices":[{"index":0,"delta":{"content":"2"},"finish_reason":null}]}
# ...
# data: [DONE]
```

**对比**：
- 非流式：等待 3-5 秒后一次性返回完整结果
- 流式：立即开始返回，像打字机一样逐字输出

---

## 🎉 完成第四步后你将掌握

- ✅ 如何启动和配置网关
- ✅ 如何管理 Admin Token
- ✅ 如何添加 AI Provider 和 API Key
- ✅ 如何发送聊天请求（非流式和流式）
- ✅ 如何查看日志和统计
- ✅ 网关的基本工作流程

---

### 第五步：深入模块（Dive Deep）

#### 学习顺序（由易到难）

##### Level 1：辅助模块（先理解工具）
```
1. src/error.rs          - 错误处理（简单枚举）
2. src/config/           - 配置加载（TOML 反序列化）
3. src/crypto/           - Ed25519 签名（调用库）
4. src/logging/          - 日志系统（Tracing 使用）
```

##### Level 2：存储层（理解数据持久化）
```
5. src/db/               - 数据库连接工具
6. src/admin/mod.rs      - Admin Token 的 CRUD
7. src/server/storage_traits.rs  - 存储接口定义（重要！）
8. src/logging/postgres_store.rs - PostgreSQL 实现
9. src/logging/database.rs       - SQLite 实现
```

**学习技巧**：对比 PostgreSQL 和 SQLite 两种实现，理解 Trait 的抽象

##### Level 3：业务逻辑（核心算法）
```
10. src/server/model_parser.rs      - 模型名称解析
11. src/server/model_redirect.rs    - 模型重定向
12. src/routing/load_balancer.rs    - 负载均衡算法
13. src/server/provider_dispatch.rs - 提供商选择逻辑
```

##### Level 4：提供商适配（格式转换）
```
14. src/providers/openai.rs         - OpenAI API 封装
15. src/providers/anthropic.rs      - Anthropic 格式转换（复杂）
16. src/providers/zhipu.rs          - 智谱 AI 适配
17. src/server/streaming/           - 流式响应处理（最复杂）
```

**重点**：Anthropic Provider 展示了如何转换不同 API 格式

##### Level 5：HTTP 层（框架使用）
```
18. src/server/handlers/chat.rs        - 聊天接口实现
19. src/server/handlers/admin_tokens.rs - Token 管理
20. src/server/handlers/providers.rs    - Provider 管理
21. src/server/login.rs                 - TUI 认证（高级）
```

---

## 🔧 学习工具推荐

### 1. 代码导航工具
```bash
# 使用 ripgrep 快速搜索
rg "async fn chat_completions"  # 找函数定义
rg "impl.*Provider"              # 找所有 Provider 实现
rg "pub trait"                   # 找所有 Trait 定义

# 使用 tokei 统计代码量
cargo install tokei
tokei src/
```

### 2. 调试技巧
```rust
// 在关键位置添加日志
tracing::info!("Selected provider: {:?}", provider);
tracing::debug!("Request payload: {:?}", request);
tracing::error!("Failed to call API: {}", err);

// 使用 dbg! 宏快速调试
dbg!(&admin_token);
dbg!(&parsed_model);
```

### 3. 数据库查看
```bash
# SQLite
sqlite3 data/gateway.db
.tables
SELECT * FROM admin_tokens;
SELECT * FROM request_logs ORDER BY timestamp DESC LIMIT 10;

# PostgreSQL
psql -U user -d gateway
\dt
SELECT * FROM request_logs ORDER BY timestamp DESC LIMIT 10;
```

---

## 📝 学习检查清单

### 第一周：理解整体架构
- [ ] 能画出请求流程图
- [ ] 理解 6 个核心概念
- [ ] 成功启动项目并发送请求
- [ ] 能解释 Provider、Token、负载均衡的作用

### 第二周：深入关键模块
- [ ] 理解 Trait 如何实现存储抽象
- [ ] 能修改负载均衡策略
- [ ] 理解 Anthropic 的格式转换
- [ ] 能添加一个简单的 API 端点

### 第三周：实战扩展
- [ ] 添加一个新的 Provider（如 DeepSeek）
- [ ] 实现一个新的统计指标
- [ ] 优化某个性能瓶颈
- [ ] 编写单元测试

---

## 🎓 进阶学习资源

### Rust 异步编程
- Tokio 官方文档：https://tokio.rs/
- Async Book：https://rust-lang.github.io/async-book/

### Axum 框架
- Axum 文档：https://docs.rs/axum/latest/axum/
- 示例项目：https://github.com/tokio-rs/axum/tree/main/examples

### AI API 标准
- OpenAI API 文档：https://platform.openai.com/docs/api-reference
- Anthropic API 文档：https://docs.anthropic.com/claude/reference

---

## 💡 学习心态建议

1. **不要试图一次理解所有代码**
   - 聚焦主流程，忽略细节
   - 先理解"做什么"，再研究"怎么做"

2. **实践优先于阅读**
   - 先运行起来，再看代码
   - 修改代码观察效果

3. **带着问题学习**
   - "如果我要添加新功能，需要改哪里？"
   - "这个设计为什么这样做？"

4. **做笔记和总结**
   - 画图（流程图、架构图）
   - 写注释（用自己的话解释代码）

5. **不懂就问**
   - 善用 AI 助手解释代码
   - 查官方文档和示例

---

## 🚀 下一步行动

### 今天就开始（选一项）
- [ ] 阅读 ARCHITECTURE.md 和 architecture.svg
- [ ] 在纸上画出请求流程图
- [ ] 启动项目并发送第一个请求
- [ ] 阅读 src/main.rs 和 src/server/mod.rs

### 本周完成
- [ ] 完成第三步的请求追踪练习
- [ ] 完成第四步的 6 个实践任务
- [ ] 理解 Level 1 和 Level 2 的模块

### 本月目标
- [ ] 掌握所有 Level 1-4 的模块
- [ ] 能够独立添加一个新功能
- [ ] 能够修复一个 Bug

---

**记住**：学习一个项目就像剥洋葱，一层一层来，不要着急。🧅
