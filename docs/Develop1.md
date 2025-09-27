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

## 2025-09-27 模型缓存 selected 语义增强与易用性修复（已被更清晰接口替代）

### 背景与问题
- 期望：通过 `GET /models/{provider}?cache=selected&include=A,B` 仅保留所选模型；通过 `...&remove=X` 删除指定模型。
- 现状：此前实现将 selected/include 作为“追加/更新”，且仅在 `refresh=true` 时生效；因此
  - 未带 `refresh=true` 的 selected 请求不会更改缓存。
  - 无“仅保留所选”的直达能力，需要先全量再排除，操作不直观。

### 改动概要（向后兼容）
- 支持在未设置 `refresh=true` 的情况下执行 `cache=selected` 的 include/remove 变更。
- 新增查询参数 `replace=true`：在 `cache=selected` + `include` 时，仅保留 include 中的模型（覆写该供应商缓存）。
- `cache=all` 分支行为不变：仍需 `refresh=true`，可配合 `exclude` 完整重建缓存。
- 响应头继续返回摘要：`X-Cache-Added`、`X-Cache-Updated`、`X-Cache-Removed`、`X-Cache-Filtered`。

注：后续已对接口进行语义化拆分，GET 不再承载任何“写入/变更”能力，以下用法现已由 POST/DELETE 替代，详见下一节。

### 设计说明
- KISS：把 include/remove 的最常见诉求放在一个端点内完成；`replace=true` 显式表达“仅保留所选”。
- YAGNI：不引入额外复杂模式，默认行为保持“追加/更新”，只有在 `replace=true` 时才覆盖。
- DRY：沿用现有缓存层接口（覆盖、追加、移除），避免重复代码路径。

### 方法语义建议（后续可选）
- 出于 REST 语义与代理缓存友好性：建议新增写操作端点，同时保留现有 GET 以兼容：
  - `POST /models/{provider}/cache`：Body 支持 `{mode: "all"|"selected", include:[], exclude:[], replace:bool}`
  - `DELETE /models/{provider}/cache?ids=...`：精确删除
- 当前迭代未变更路由，仅改进 GET 的直观性与可用性。

### 任务小结
- 允许 `cache=selected` 的 include/remove 在无 `refresh=true` 时生效。
- 新增 `replace=true` 支持“仅保留所选模型”的覆盖写入。
- 更新了响应摘要头并保持兼容；建议未来引入 POST/DELETE 语义化端点。

## 2025-09-27 接口拆分与职责简化（POST/DELETE 生效，GET 只读）

为降低参数组合复杂度，彻底拆分接口职责：GET 仅用于读取，写操作迁移至 POST/DELETE。

- POST `/models/{provider}/cache`
  - Body(JSON)：`{ mode: "all"|"selected", include?: string[], exclude?: string[], replace?: bool }`
  - 语义：
    - `mode=all`：从上游拉取并全量覆盖，可用 `exclude` 过滤。
    - `mode=selected`：仅处理 `include` 列表；`replace=true` 覆盖，仅保留所选；否则为追加/更新。
  - 响应：返回该供应商当前缓存列表；Header 含 `X-Cache-*` 摘要。

- DELETE `/models/{provider}/cache?ids=id1,id2`
  - 语义：精确删除所列模型；响应返回当前缓存列表；Header 含 `X-Cache-Removed`。

示例：
- 仅保留两个模型（覆盖）：
  - `POST /models/openai/cache`，Body：`{"mode":"selected","include":["GLM-4.5","Qwen3-Coder-Instruct-MD"],"replace":true}`
- 选择性追加/更新（不清空）：
  - `POST /models/openai/cache`，Body：`{"mode":"selected","include":["GLM-4.5","Qwen3-Coder-Instruct-MD"]}`
- 全量重建并排除：
  - `POST /models/openai/cache`，Body：`{"mode":"all","exclude":["id1","id2"]}`
- 精确删除：
  - `DELETE /models/openai/cache?ids=Bge-m3-SiliconCloud`

日志：
- 新增请求类型：`provider_models_cache_update`、`provider_models_cache_delete`。

同时调整 GET 端点行为：
- `GET /v1/models`：仅返回缓存结果。
- `GET /models/{provider}`：
  - 无 `refresh`：仅返回该供应商缓存结果。
  - `refresh=true`：直接拉取上游并返回，但不落库、不修改缓存。

## 2025-09-26 日志增强与“意外流式”兼容、错误统一收敛

### 日志增强：请求类型
- 新增日志字段 `request_type`（TEXT）：用于标记请求类型（当前使用）
  - `chat_once`：非流式聊天
  - `chat_stream`：流式聊天
  - 预留：`models_list`、`provider_models_list`
- 数据库迁移：在启动阶段尝试 `ALTER TABLE request_logs ADD COLUMN request_type ...`，若已存在则忽略。
- 所有聊天请求均写入对应类型，保证后续统计更清晰。

### “意外流式”兼容（非流式路径）
- 当用户未设置 `stream: true`，但上游返回 `text/event-stream` 时：
  - 自动检测 `Content-Type`，聚合SSE分片为一次性 `ChatCompletionResponse` 返回给客户端
  - 尽可能提取最终 `usage`，并正常记录日志（类型：`chat_once`）
  - 实现位置：`providers/openai.rs::chat_completions`

### 错误处理统一（收敛到 GatewayError）
- 将以下文件签名切换为统一错误类型：
  - `src/main.rs`、`src/config/settings.rs`、`src/server/mod.rs`
- `GatewayError` 扩展 `Toml` 变体，覆盖配置解析错误
- 持续推进其余模块平滑迁移，减少 `Box<dyn Error>` 使用

### 影响文件（新增/修改）
- 新增：`src/logging/database_cache.rs`
- 修改：
  - `src/logging/types.rs`（新增 `request_type` 与常量）
  - `src/logging/database.rs`（DDL/插入/查询同步更新）
  - `src/server/request_logging.rs`、`src/server/streaming_handlers.rs`（写入 `request_type`）
  - `src/providers/openai.rs`（上游“意外流式”聚合为一次性响应）
  - `src/main.rs`、`src/config/settings.rs`、`src/server/mod.rs`（错误类型统一）

### 模型缓存策略变更（更贴近 NewApi 思路）
- 不再由 `/v1/models` 主动触发上游获取；仅返回数据库中的缓存结果（可能为空）
- 通过 `/models/{provider}` 按需刷新：
  - 支持查询参数：
    - `refresh=true`：访问上游拉取模型
    - `cache=all`：将上游返回的所有模型写入缓存
    - `cache=selected&include=id1,id2`：仅将选择的模型写入缓存
    - 缺省 `cache`：仅预览，不写入缓存
  - 仍返回 OpenAI 兼容的模型列表
- 两个接口均写入请求日志：
  - `/v1/models` → `request_type=models_list`
  - `/models/{provider}` → `request_type=provider_models_list`

## 2025-09-26 密钥管理与缓存策略增强（本次会话）

### 供应商密钥入库（复用现有加密策略）
- 策略复用：沿用配置项 `logging.key_log_strategy`（none/masked/plain），不新增配置项；该策略同时作用于：
  - 数据库存储：
    - `plain` → 明文存储
    - `masked`/`none` → 可逆轻量混淆存储（基于 provider+固定盐 异或+hex），便于后续切换
  - 日志展示：
    - `none` 不记录
    - `masked` 记录首尾4位
    - `plain` 记录明文（仅建议在安全环境中使用）
- 数据结构：新增表 `provider_keys(provider, key_value, enc, active, created_at)`，自动建表
- 启动导入：程序启动时将配置内密钥批量导入数据库（不存在时插入）
- 选择使用：优先从数据库读取密钥，其次回退到配置文件中的密钥
- 代码位置：
  - 存取实现：`src/logging/database_keys.rs`
  - 轻量混淆：`src/crypto/mod.rs`（protect/unprotect，按策略与provider派生材料）
  - 调度复用：`src/server/provider_dispatch.rs`（选择供应商时优先 DB 密钥）
  - 启动导入：`src/server/mod.rs`

### 安全管理接口（HTTP）
- 添加密钥：`POST /providers/:provider/keys`，Body：`{"key":"sk-..."}`，返回201
- 删除密钥：`DELETE /providers/:provider/keys`，Body：`{"key":"sk-..."}`，返回200
- 错误返回统一：使用 `GatewayError`，JSON错误体
- 日志：
  - `request_type=provider_key_add` / `provider_key_delete`
  - 路径与状态码完整记录

### 模型缓存增强
- `/v1/models`：仅返回缓存结果（可能为空），不主动请求上游；记录 `request_type=models_list`
- `/models/{provider}`：按需刷新，完整记录 path+query，并在错误时同样落库
  - `cache=all` 支持 `exclude=id1,id2`，从上游结果中过滤后全量重建该供应商缓存（不影响其他供应商）
  - `cache=selected` 支持 `include=id1,id2` 追加/更新，`remove=id3,id4` 精确移除（不清空）
  - 返回头包含变更摘要：
    - `X-Cache-Added` / `X-Cache-Updated` / `X-Cache-Removed` / `X-Cache-Filtered`
- 多供应商说明：缓存以 `(id, provider)` 为主键，不同供应商互不影响；`cache=all` 仅影响对应供应商；`cache=selected` 采用追加/更新

### 日志一致性
- 记录 `request_type` 与 `api_key`（遵循策略 none/masked/plain）
- 流式与非流式聊天均记录（流式在 `[DONE]` 或错误时落库）
- `/models/{provider}` 含完整 path+query，错误场景（provider不存在/无密钥/上游失败）均有日志

### 建议与后续工作
- 可选新增 `GET /providers/:provider/keys`（返回 masked 列表），便于运维审计
- 将 `GatewayError` 继续扩展替换其余模块的 `Box<dyn Error>`，全链路统一错误风格
- 流式异常完结（连接被动断开）时的兜底日志，需更细的生命周期钩子，建议后续评估
- 为 `/models/{provider}` 增加 `?summary=json` 返回JSON摘要（保持现有Header不变），便于程序化消费
- 强安全场景可替换轻量混淆为成熟AEAD方案（接口保持不变），并结合密钥轮换/审计

### 快速使用示例
- 添加密钥：`POST /providers/openai/keys`，Body：`{"key":"sk-xxx"}`
- 删除密钥：`DELETE /providers/openai/keys`，Body：`{"key":"sk-xxx"}`
- 刷新并全量缓存（排除两个ID）：`GET /models/openai?refresh=true&cache=all&exclude=id1,id2`
- 选择性缓存与移除：`GET /models/openai?refresh=true&cache=selected&include=id3,id4&remove=id5`

### 本次会话更新小结（变更日志）
- 复用 `logging.key_log_strategy` 实现供应商密钥的数据库存储与日志展示策略统一
- 新建 `provider_keys` 表，启动时导入配置内密钥；选择供应商优先使用DB密钥
- 新增密钥管理接口：`POST/DELETE /providers/:provider/keys`，记录操作日志
- `/models/{provider}` 增强：`cache=all` 支持 `exclude`，`cache=selected` 支持 `remove`；返回头携带变更摘要
- 日志增强：记录完整 path+query、错误场景、以及 `api_key`（按策略 none/masked/plain）
