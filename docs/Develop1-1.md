## AI Gateway 开发日志 - 阶段一·增量 1（2025-09-27）

本次增量聚焦接口语义优化、日志时间本地化与错误统一：

### 1. 缓存删除接口统一为 JSON Body
- 变更：`DELETE /models/{provider}/cache` 现在使用 JSON Body 传参，替代原 query 形式。
- 请求体：
  - `{"ids": ["id1", "id2", "id3"]}`
- 响应：返回该供应商当前缓存列表；响应头包含变更摘要：
  - `X-Cache-Added` / `X-Cache-Updated` / `X-Cache-Removed` / `X-Cache-Filtered`
- 目的：与 `POST /models/{provider}/cache` 统一风格，便于一次性批量删除与审计。

示例：
- `DELETE /models/openai/cache`
  - Body: `{"ids":["Bge-m3-SiliconCloud","GLM-4.5"]}`

### 2. tracing 日志时间本地化（北京时间）
- 问题：RUST_LOG=debug 启动时，终端日志时间为 `2025-09-27T04:30:13.252229Z`（UTC）。
- 方案：实现 `BeijingTimer` 并在 `tracing_subscriber` 中启用，复用数据库的时间格式与时区。
- 效果：日志时间与数据库一致，格式 `YYYY-MM-DD HH:MM:SS`（UTC+8）。
- 启动示例：`RUST_LOG=debug cargo run`

### 3. 错误处理统一
- 全仓扫描未发现仍在使用 `Box<dyn Error>` 的代码路径；若后续新增模块，请遵循统一错误 `GatewayError`。
- 移除潜在的 `anyhow` 依赖调用（若出现则改为 `GatewayError::Config/Http/...`）。

### 4. 鉴权
- 暂不启用，后续迭代再行设计和落地。

### 影响文件（关键修改）
- 删除接口改为 JSON Body：
  - `src/server/handlers/cache.rs`
- tracing 时间本地化：
  - `src/logging/time.rs`
  - `src/main.rs`
- 依赖：新增 `reqwest-eventsource` 用于稳定 SSE 事件解析

### 回归要点
- `GET /models/{provider}`：不进行任何写入/变更；`refresh=true` 仅从上游拉取返回。
- `POST /models/{provider}/cache`：all/selected 覆盖/追加/过滤能力不变。
- `DELETE /models/{provider}/cache`：仅 JSON Body `ids` 生效；不再支持 query 传参。

### 原则
- KISS：接口职责单一，读写分离，时间格式统一。
- YAGNI：不引入额外复杂配置；仅落地当前明确需求。
- DRY/SOLID：复用时间与缓存接口，模块职责清晰（handlers 已拆分）。

### 追加修复：非流式 SSE 回退聚合丢字问题（2025-09-27）
- 现象：`/v1/chat/completions` 在 `stream=false` 下，个别上游仍以 `text/event-stream` 返回，网关按行解析时因跨 chunk 半行/半事件导致内容缺失，Markdown 渲染偶发缺字。
- 处理：改造 OpenAI 客户端的 SSE 回退聚合逻辑，新增跨 chunk 缓冲：
  - `line_buf`：保留末尾半行，等待下个 chunk 拼接。
  - `event_buf`：按空行作为边界聚合多行 `data:` 形成完整事件，再 JSON 解析。
  - 解析失败时不丢字符，原样追加到内容，确保“最多多字，不会少字”。
- 影响文件：
  - `src/providers/openai/client.rs`
- 回归要点：
  - 非流式正常返回 `application/json` 不受影响。
  - 上游意外 SSE 返回时，聚合为一次性响应且不丢字符。

### 追加优化：参考 ai-gateway 的逐事件转发流式实现（2025-09-27）
- 背景：`ai-gateway/ai-gateway` 采用 `reqwest-eventsource` 将 SSE 事件逐条推送至下游，避免同一 chunk 内多个 `data:` 被丢弃。
- 优化：我们的 `streaming_handlers.rs` 基于 `reqwest-eventsource` 的 `EventSource` 对齐实现，逐事件转发；下游不再丢首个之外的 `data:` 行。
- 效果：
  - 流式响应在大块分包/多事件同 chunk 场景下不再丢字符。
  - `[DONE]` 到达时仅记录一次日志并正确完结；过程中尝试捕获 `usage` 聚合到日志。
- 影响文件：
  - `src/server/streaming_handlers.rs`

### 进一步对齐：移除 bytes_stream 回退，统一使用 reqwest-eventsource（2025-09-27）
- 背景：此前非流式在上游返回 SSE 时采用 `bytes_stream` 手工解析与聚合（冗余且维护成本高）。
- 对齐：彻底移除旧的 `bytes_stream` 回退模块，统一改为基于 `reqwest-eventsource` 的事件聚合，实现与 `ai-gateway` 一致的行为。
- 变更要点：
  - `OpenAIProvider::chat_completions` 先尝试以 SSE 打开（`Accept: text/event-stream`），逐事件聚合；若不支持/失败，则回退为 JSON 一次性解析。
  - 删除文件：`src/providers/openai/sse_fallback.rs`。
  - 删除未使用的流式客户端实现（统一走 `server/streaming_handlers.rs`）。
- 错误处理：统一返回 `GatewayError`，避免闭包式零散处理。
- 影响文件：
  - `src/providers/openai/client.rs`
  - `src/providers/openai.rs`
  - `src/server/provider_dispatch.rs`（错误类型统一）
  - `src/server/handlers/chat.rs`（错误映射移除，直接传播统一错误）
  - `src/server/request_logging.rs`（日志函数参数类型改为 `Result<.., GatewayError>`）
- 依赖：本次不新增依赖；已使用 `reqwest-eventsource = "0.6.0"` 与仓库现有的 `futures-util`/`tokio`。

回归建议
- 非流式：在上游返回 `application/json` 与 `text/event-stream` 两种情况下分别验证，确认内容不丢字；特别针对 Markdown 场景做对比测试。
- 流式：确认 `[DONE]` 正确终止，日志只落一次；网络中断后行为可接受（本轮未引入重连策略）。
