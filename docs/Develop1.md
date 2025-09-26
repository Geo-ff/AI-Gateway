# AI Gateway 开发日志 - 阶段一

## 项目概述

基于Rust实现的高性能AI网关，支持多供应商API调用聚合、负载均衡、协议转换和详细日志记录。

## 完成功能

### 配置管理系统

- 实现静态配置文件解析，支持 `custom-config.toml` 和 `config.toml`
- 设计多供应商配置结构，每个供应商支持多密钥池
- 实现模型重定向配置系统，支持 `redirect.toml` 文件

### 负载均衡策略

- 实现三种负载均衡策略：FirstAvailable（默认）、RoundRobin、Random
- 支持供应商级别和密钥级别的负载均衡
- 使用原子计数器确保线程安全的轮询实现

### 协议适配层

- 完整实现OpenAI Chat Completion协议支持
- 实现Anthropic协议转换适配器，支持双向转换
- 统一的请求响应数据结构，支持流式和非流式调用

### 日志记录系统

- 基于SQLite的请求日志记录，包含时间、模型、供应商、状态码等关键信息
- 支持token使用量统计和响应时间监控
- 异步数据库操作，不影响主要业务流程

### 模型管理功能

- 实现模型重定向功能，支持别名映射
- 支持上游模型列表获取，兼容OpenAI标准/models端点
- 提供按供应商查询模型的独立接口

### 数据库自动创建功能

- 程序启动时自动检查并创建 `data` 文件夹
- 如果数据库文件不存在则自动创建
- 默认数据库路径：`data/gateway.db`
- 支持自定义数据库路径配置

## 技术架构

采用模块化设计，核心模块包括：

- `config`: 配置管理和解析
- `routing`: 负载均衡和供应商选择
- `providers`: 协议适配和API调用
- `logging`: 数据库日志记录
- `server`: HTTP服务器和路由处理

## 配置文件示例

```toml
[load_balancing]
strategy = "round_robin"

[server]
host = "0.0.0.0"
port = 8000

[logging]
database_path = "data/gateway.db"

[providers.openai]
api_type = "openai"
base_url = "https://api.openai.com"
api_keys = ["sk-key1", "sk-key2"]

[providers.claude]
api_type = "anthropic"
base_url = "https://api.anthropic.com"
api_keys = ["claude-key1"]
```

## 核心接口

- `POST /v1/chat/completions` - 统一聊天完成接口
- `GET /v1/models` - 获取可用模型列表
- `GET /models/{provider}` - 获取指定供应商模型

## 使用说明

无需手动创建数据库文件，程序会在首次启动时自动创建：

1. 检查 `data` 目录是否存在，不存在则创建
2. 在指定路径创建SQLite数据库文件
3. 初始化请求日志表结构

所有功能均按照KISS原则实现，确保代码简洁可维护。

## 2025-09-26 模型前缀处理功能修复

### 问题描述
通过 `/v1/models` 端点返回的模型列表使用 `provider/model` 格式（如 `openai/Qwen3-Coder-Instruct-MD`），但在 `/v1/chat/completions` 请求时，上游API需要的是实际的模型名称（如 `Qwen3-Coder-Instruct-MD`），前缀处理不正确导致调用失败。

### 解决方案
1. 新增 `model_parser.rs` 模块，实现 `ParsedModel` 结构体用于解析模型名称前缀
2. 扩展 `provider_dispatch.rs` 模块，新增 `select_provider_for_model` 和 `call_provider_with_parsed_model` 函数
3. 修改 `handlers.rs` 中的 `chat_completions` 处理逻辑，正确解析模型前缀并传递实际模型名给上游

### 实现细节
- `ParsedModel::parse()` 方法支持解析 `provider/model` 格式，提取供应商名和实际模型名
- `select_provider_for_model()` 优先根据模型前缀选择对应供应商，回退到负载均衡策略
- `call_provider_with_parsed_model()` 创建修改后的请求，使用实际模型名调用上游API
- 保持日志记录使用原始模型名（含前缀），便于追踪

### 技术实现
- 使用 `String::find()` 和字符串切片进行高效的前缀解析
- 保持向后兼容性，支持无前缀的模型名称
- 遵循现有代码风格，使用 `pub(crate)` 模块可见性

## 2025-09-26 时间格式优化

### 问题分析
数据库中存储的时间戳使用RFC3339格式（如 `2025-09-26T07:10:50.235720675+00:00`），这是代码实现的问题，不是SQLite本身的限制。该格式对人类阅读不够友好。

### 解决方案
修改 `logging/database.rs` 模块，实现北京时间（UTC+8）的人类友好格式存储：
1. 新增 `to_beijing_string()` 函数，将UTC时间转换为北京时间的 `YYYY-MM-DD HH:MM:SS` 格式
2. 新增 `parse_beijing_string()` 函数，将存储的北京时间字符串解析回UTC时间
3. 修改所有时间存储和读取操作，使用新的时间格式函数

### 实现细节
- 使用 `chrono::FixedOffset::east_opt(8 * 3600)` 定义北京时区
- 时间格式为 `%Y-%m-%d %H:%M:%S`，如 `2025-09-26 15:10:50`
- 数据库内部存储北京时间字符串，程序内部仍使用UTC时间处理
- 修复了chrono库弃用警告，使用新的API实现时间解析

### 影响范围
- 请求日志表 `request_logs` 的 `timestamp` 字段
- 模型缓存表 `cached_models` 的 `cached_at` 字段
- 所有相关的时间读取和写入操作

## 2025-09-26 OpenAI响应结构体优化与流式传输功能实现

### OpenAI响应结构体扩展
根据完整的OpenAI API响应格式，扩展了响应数据结构：
1. **Choice结构扩展**：添加了`refs`、`logprobs`、`service_tier`字段
2. **Usage结构增强**：新增`prompt_tokens_details`和`completion_tokens_details`子结构
3. **详细字段支持**：包含`cached_tokens`和`reasoning_tokens`等新字段

### 流式传输功能完整实现
成功实现了Server-Sent Events (SSE)流式传输支持：

#### 核心功能
- **统一处理器**：`chat_completions`处理器自动检测`stream`参数，支持流式和非流式请求
- **SSE响应格式**：完整的Server-Sent Events格式支持，包括`data:`前缀解析
- **实时数据传输**：通过`reqwest::Response::bytes_stream()`实现真正的流式数据传输
- **错误处理机制**：完善的流式传输错误处理和日志记录

#### 技术实现
- **依赖管理**：添加`tokio-stream`、`futures-util`、`thiserror`等流式处理依赖
- **模块架构**：创建`streaming_handlers.rs`专门处理流式传输逻辑
- **数据结构**：设计`StreamMessage`、`StreamChoiceDelta`等流式传输专用数据类型
- **生命周期优化**：通过直接在处理器中创建流来解决Rust异步生命周期问题

#### 兼容性保证
- **向后兼容**：非流式请求保持原有处理逻辑不变
- **供应商支持**：当前支持OpenAI流式传输，为Anthropic预留扩展接口
- **模型前缀**：完美支持已实现的模型前缀解析功能

### 实现亮点
- **零配置切换**：用户只需在请求中设置`"stream": true`即可启用流式传输
- **错误恢复**：流式传输过程中的错误不会中断整个连接
- **性能优化**：使用Axum原生SSE支持，避免手动HTTP响应构建
- **调试友好**：详细的日志记录和错误信息，便于问题排查

此实现遵循OpenAI标准的流式API格式，确保与现有客户端的完美兼容性。

## 2025-09-26 代码精简与错误统一、SSE日志修复

### 精简与清理
- 移除未使用函数：`provider_dispatch::call_provider` 与 `ParsedModel::matches_provider`
- 优化供应商前缀选择：无可用密钥时返回更精确的 `NoApiKeysAvailable`
- 拆分超长文件：将 `logging/database.rs` 拆分为
  - `logging/database.rs`（数据库初始化与请求日志）
  - `logging/database_cache.rs`（模型缓存相关）
  - `logging/types.rs`（数据类型：`RequestLog`、`CachedModel`）
  - `logging/time.rs`（北京时间格式化/解析）

### 统一错误类型（thiserror）
- 新增 `error::GatewayError` 统一错误类型，覆盖 HTTP/JSON/DB/IO/负载均衡/时间解析等
- 在 `server/model_helpers.rs` 与 `logging/time.rs` 中率先落地统一错误；其余模块保持兼容，后续可平滑迁移

### 流式日志修复（SSE）
- `streaming_handlers.rs` 中为流式路径添加日志记录：
  - 请求开始记录 `start_time`
  - 在接收到 `data: [DONE]` 时记录一条完整日志（包含响应耗时与可用的 tokens 使用量）
  - 流式错误发生时同样记录失败日志，避免缺失
  - 解析 SSE JSON 块中的 `usage` 字段并累积，尽可能与非流式保持一致

### 原则与取舍
- KISS/YAGNI：仅保留必要代码路径，避免过度设计
- DRY：时间格式化/解析与类型定义统一抽离复用
- SOLID：按职责拆分模块，降低耦合，便于后续扩展
