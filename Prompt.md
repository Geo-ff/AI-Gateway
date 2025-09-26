我现在需要你为我开发一个基于 rust 的高性能AI网关，以便于进行 AI API 的调用聚集和请求转发以及详细的日志记录。当前项目下已经有一个成熟的项目供你参考，后端核心代码在  ai-gateway/ai-gateway 路径下。这个目录你只能作为参考，我需要的是基于后端核心代码按照我的要求重新实现，因此你禁止修改这个目录下的文件内容，并且你也不能完全使用这个目录下已有的代码文件，只能参考其中的写法和思路，但是要以我的要求为准。
首先，先为我完成开发的第一阶段功能：
```
阶段一：核心代理与多渠道转发
目标: 快速验证核心代理能力，并实现对多个主流AI提供商的无缝兼容。
核心功能:
静态配置驱动: 用户可以通过创建一个 custom-config.toml 文件（其他名字都可），支持配置多个上游渠道（Providers），包括其 API 地址和密钥池（多个密钥用逗号分割，便于轮询）。
密钥池与轮询功能:
支持为一个渠道配置多个 API Key。
实现基础的负载均衡策略：顺序轮询、随机轮询。（这两个功能应该是可选的，在配置文件中可以自行选择，如果用户不选择负载均衡策略，就默认使用第一个密钥）
协议转换适配器功能:
首要目标: 完美兼容 OpenAI
扩展目标: 实现对 Anthropic (Claude) 协议的转换，能将标准的 OpenAI Chat Completion 请求体，自动转换为 Claude 的格式，反之亦然。
基础日志记录功能:
使用 SQLite 作为默认存储，将每一次请求的关键信息（时间、模型、路径、状态码）记录到数据库。（这是用户未选择配置额外数据库连接时使用的默认策略）
后续需要支持 PostgreSQL（但放在第二部分进行开发） 
模型重定向功能: 可以在一个新的配置文件中比如 redirect.toml 中设置模型别名，例如 {"gpt-4-fast": "azure-gpt-4-turbo"}。
获取上游模型功能：根据用户配置文件中给出的供应商的对应 URL 来获取对应的上游供应模型名（比如 OpenAI 标准的 /models 目录，而不应是写死的模型名，并且用户可以选择性地添加，也就是用户可以主动对这个地址发起请求，获取可选的模型名，再自己决定是否把模型名加到模型列表里） 
```
你在为我开发的时候，需要有简短但是精炼的开发日志（我已经在 docs 目录下创建好了一个 Develop1 文件，本次开发日志就写到该文件中去），禁止使用任何 emoji 以及大于三级的标题，并且用中文。并且使用的项目依赖应该是渐进式的并且让我来手动添加而不是你直接添加（我当前项目中已有一定的配置文件），开发准则是遵循 KISS 原则的，你的数据结构设计应该是合理的。禁止执行任何命令，要执行的命令都必须要告诉我，让我代执行并且告诉你结果（cargo check 等检查命令除外）。禁止直接创建任何测试代码，测试代码应该在开发完一个单独的功能点之后，和我确认后，单独创建在单独的文件夹下。



对于数据库文件，需要我自己进行创建吗?我需要在哪里手动创建一个 sqlite 文件？我希望在启动的时候，检查当前目录下有无 data 文件夹，并且其中有无数据库，如果没有这个目录且没有数据库文件，则自动进行创建



这个：
- `GET /models/{provider}` - 获取指定供应商模型
路由的作用是什么？如果通过这个路由获取到指定供应商的模型数据，是否会进行存储以便于在 `GET /v1/models` 路由进行获取？



目前我正在开发一个基于 rust 的高性能AI网关，以便于进行 AI API 的调用聚集和请求转发以及详细的日志记录。当前项目的 ai-gateway 路径下已经有一个成熟的项目供你参考，核心代码在 ai-gateway/ai-gateway 路径下。这个目录你只能作为参考，我需要的是基于后端核心代码按照我的要求重新实现，因此你禁止修改这个 ai-gateway 目录下的文件内容，并且你也不能完全使用这个目录下已有的代码文件，只能参考其中的写法和思路，但是要以我的要求为准。
目前，当前 src 文件夹下已有的项目大体已经完成了通过 /v1/models 端点进行模型的获取，我的测试效果如下：

```
{
  "object": "list",
  "data": [
    {
      "id": "openai/Bge-m3-SiliconCloud",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Bge-reranker-v2-m3-SiliconCloud",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Claude-4-Foxcode",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Claude-4-FunnyL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Claude-4-Ocool",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Claude-4-OpenRouter",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Claude-4T-Foxcode",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Deepseek-3.1-Flow",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/DeepSeek-R1-HuoShan",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/DeepSeek-V3-HuoShan",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/DeepSeek-V3-SophNet",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Doubao-1.6-Thinking-Pro-M",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Doubao-Seed1.6F",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Doubao-version-lite",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Doubao-version-Pro",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/DS-R1-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/DS-V3-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Gemini-2.5P-Any",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Gemini-2.5P-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Gemini-2.5Pro-TL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/glm-4.5-air",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5Air",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5Airx",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5FFree",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5V",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GLM-4.5X",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1-FunnyL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1-Ocool",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1-OpenRouter",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1-Search-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4.1M-TL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-4o-XiaoAi",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-5-Neuro",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-5H",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-5L",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-5M",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-5Mini",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-o3-XiaoAi",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/GPT-o4M-NYL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Kimi-K2-Flow",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Kimi-K2-Moonshot",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Kimi-K2-Silicon",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/o3M-FunnyL",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Qwen3-Coder-30B-Silicon",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Qwen3-Coder-Flow",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Qwen3-Coder-Instruct-MD",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Qwen3-Embedding-8B-Silicon",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/Qwen3-PAli",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/v0-1.0-md",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/v0-1.5-lg",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    },
    {
      "id": "openai/v0-1.5-md",
      "object": "model",
      "created": 1626777600,
      "owned_by": "custom"
    }
  ]
}
```

正确获取了指定 URL 的模型情况，并且返回了正确的模型名称。但是我通过 /v1/chat/completions 端点去进行聊天请求的时候，需要传递的模型名称为：

```
{
    "model": "Qwen3-Coder-Instruct-MD",
    "messages": [
      {
        "role": "user",
        "content": "你好呀，请你自我介绍一下你自己！"
      }
    ]
}
```

才可以正常调用，否则会失败。你可以看到，需要去掉 openai/ 部分才可以正常调用成功。因此当前项目中的第一个问题是，当用户通过获取到的模型列表的模型名字比如 openai/Qwen3-Coder-Instruct-MD 去请求对话的时候，要正确地处理前缀问题，以保证传递给上游的模型名称是一致的。

请你在保留 openai/Qwen3-Coder-Instruct-MD 前缀的前提下，仔细地解决这个问题。

在你修复后需要有简短但是精炼的开发日志（我已经在 docs 目录下创建好了一个 Develop1.md 文件，本次修复日志就追加到该文件中去），禁止使用任何 emoji 以及大于三级的标题，并且用中文。并且使用的项目依赖应该是渐进式的并且让我来手动添加而不是你直接添加（我当前项目中已有一定的配置文件），开发准则是遵循 KISS 原则的，你的数据结构设计应该是合理的。禁止执行任何命令，要执行的命令都必须要告诉我，让我代执行并且告诉你结果（cargo check 等检查命令除外）。禁止直接创建任何测试代码，测试代码应该在开发完一个单独的功能点之后，和我确认后，单独创建在单独的文件夹下。

如果有你不能直接修复的语法错误，请你使用 context7 MCP 来获取最新的文档。修复时应当保持与当前项目中已有的代码风格一致。



修复成功了，现在可以正确进行请求了。
现在需要你继续小心地进行接下来的修改：
1. 当前的代码逻辑，在调用成功之后，添加到数据库中的日志中的 timestamp 和 cached_at 属性，都是 2025-09-26T07:10:50.235720675+00:00 这样的时间戳。请你解释这个是 sqlite 的原因还是代码本身的原因导致不能以常规的人类易读的方式进行记录？如果是 sqlite 本身的原因则保持原样，如果是代码的原因，则将其改为人类易读的并且是北京时间的时区的时间日志


日期的显示修复也成功了，现在请你继续小心地进行接下来的修改，如有语法问题请你及时使用 context7 MCP 来获取最新的开发文档：

1. 当前的 OpenAI 的返回信息的结构体的属性不够全面，我给你一个较为完整的版本，请你按照完整的版本进行优化：

```
{
  "id": "0217588708746295b40003b657011e4d2373216f927e185d3667f",
  "object": "chat.completion",
  "created": 1758870885,
  "model": "DeepSeek-V3-Fast",
  "usage": {
    "prompt_tokens": 13,
    "completion_tokens": 386,
    "total_tokens": 399,
    "prompt_tokens_details": {
      "cached_tokens": 0
    },
    "completion_tokens_details": {
      "reasoning_tokens": 0
    }
  },
  "choices": [
    {
      "index": 0,
      "message": {
        "content": "你好呀！我是 **DeepSeek-V3**，由 **深度求索（DeepSeek）** 打造的一款智能对话助手。有什么问题或者需求，尽管问我吧！",
        "role": "assistant"
      },
      "refs": null,
      "logprobs": null,
      "finish_reason": "stop",
      "service_tier": null
    }
  ]
}
```

2. 当前项目已有的功能只能处理非流式传输的对话，但是如果上游是通过流式进行返回的，或者用户在请求的时候主动添加了 stream 参数，则无法进行正确处理。请你获取最新的开发文档来为我解决流式传输功能

代码不要都放在一个文件里面，要进行适当的拆分。请你开始仔细解决我提出的两个问题。


你是一名经验丰富的软件开发工程师，专注于构建高性能、健壮的解决方案。

你的任务是：**审查、理解并迭代式地改进现有代码库**

我当前项目最近的更新日志为：

```
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
```

我现在需要你按照要求进行仔细地优化和修改：

1. 请你检查 模型前缀处理功能 是否简洁正确地修复成功了。我的测试是完全没有问题的，修复是正确且有效的，但是我对于代码的简洁度不放心，请你为我检查一下
2. 同样的对于 时间格式优化 这个功能，也请你为我检查是否简洁有效地进行修复了，我测试也是正常的，但是代码请你再审查一下
3. 最后的流式传输也是成功实现了，同样的请你审查代码实现是否简洁，而不是使用了复杂的实现方式。我测试是成功流式响应的，但是如果使用了 "stream": true 参数启用流式或者供应商使用了流式返回（有时候没有加 "stream": true 参数上游供应商也会流式返回，这个是无法控制的），数据库中并没有正确记录日志，我希望也能和非流式请求一样在数据库中能够正确记录请求日志
4. 当前项目中我看到对于错误的传播似乎都是采用 Box<dyn std::error::Error> 这样的特征对象模式，但是我当前项目中已经有了 thiserror 依赖，我希望创建一个大一统的错误结构体来进行所有的错误处理，而不是各自处理
5. 最后请你检查是否有些文件代码过多，比如超过了 200 行，对于这类文件，如果可以，请进行拆分，以保持良好的阅读性

在你完成所有的任务之后，将任务日志也精要地追加到 docs/Develop1.md 文件中去，并且使用中文。碰到语法错误和问题，请你使用 context7 MCP 来获取最新的开发文档。

在整个工作流程中，你必须内化并严格遵循以下核心编程原则，确保你的每次输出和建议都体现这些理念：

- **简单至上 (KISS):** 追求代码和设计的极致简洁与直观，避免不必要的复杂性。
- **精益求精 (YAGNI):** 仅实现当前明确所需的功能，抵制过度设计和不必要的未来特性预留。
- **坚实基础 (SOLID):**
  - **S (单一职责):** 各组件、类、函数只承担一项明确职责。
  - **O (开放/封闭):** 功能扩展无需修改现有代码。
  - **L (里氏替换):** 子类型可无缝替换其基类型。
  - **I (接口隔离):** 接口应专一，避免“胖接口”。
  - **D (依赖倒置):** 依赖抽象而非具体实现。
- **杜绝重复 (DRY):** 识别并消除代码或逻辑中的重复模式，提升复用性。