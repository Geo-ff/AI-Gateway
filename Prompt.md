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



我刚刚进行了测试，并且能够正确请求和记录日志了。现在需要你为我仔细完成下面任务：

1. 因为我们已经有了流式和非流式的请求方式，那么现有的日志属性就需要添加一个 request_type （请你取一个合适的名字）属性，记录某一次请求是流式的还是非流式的，并且如果后续添加了一些其他的请求类型，也可以正确记录
2. 我看了你的建议，我同意 “全面迁移错误类型”，对于 src/main.rs, src/config/settings.rs, src/server/mod.rs 这几个文件也要用统一的错误处理方式，而不是使用 Box<dyn std::error::Error> 这样的动态 Trait
3. 对于你的 “更完善的流式异常完结日志” 建议，先暂时放一放。而 “对上游“意外流式”的兼容” 请你进行实现，以达到更好的兼容性

补充一下：当前项目的 ai-gateway 路径下已经有一个成熟的项目供你参考，核心代码在 ai-gateway/ai-gateway 路径下。这个目录你只能作为参考，我需要的是基于后端核心代码按照我的要求重新实现，因此你禁止修改这个 ai-gateway 目录下的文件内容，并且你也不能完全使用这个目录下已有的代码文件，只能参考其中的写法和思路，但是要以我的要求为准。

你可以根据我的补充去获取一些更好的代码写法和结构，来弥补我当前项目可能存在的不足。


感谢你的改进，现在请你继续小心地修改：
1. 请你继续给 list_models / list_provider_models 也加上日志并写入相应 request_type（其实我想要让你修改一下当前模型的缓存逻辑，改为像 NewApi 项目那样的，添加了供应商之后，不要主动地去请求上游供应商提供的模型，而是应该让用户主动访问一下我们项目中应该有的一个对于不同供应商请求模型名称的接口，从而获取到对应供应商提供的模型，然后进行缓存。如果可以的话，最好实现用户在请求后，可以可选地添加自己想要的模型，而不是一次性将上游供应商所有的模型都添加到我们的数据库缓存中去）
2. 将其余模块（如 handlers）也全面切换到 GatewayError，统一 HTTP 错误映射
3. 在上面所有工作完成之后，适当拆分 src/providers/openai.rs 文件，这个文件代码量已经比较多了



我刚刚进行了测试，更改是成功的！
但是还有一些地方需要你小心地更改：
1. 我观察到使用 GET 请求 http://localhost:8080/models/openai?refresh=true 路径的时候，日志是正常记录 /models/openai?refresh=true 的，但是我使用 GET http://localhost:8080/models/openai?refresh=true&cache=selected&include=GLM-4.5,Qwen3-Coder-Instruct-MD 进行测试的时候，确实成功实现了数据库中的模型缓存刷新，但是日志记录却也是 /models/openai?refresh=true。能否实现详细记录（如果不能或者实现复杂则告诉我让我考虑一下）？而且如果我请求其他供应商，也是这样可选地进行请求的话，数据库中模型缓存应该是追加的吧？请你为我解释一下当前项目如何处理这样的多供应商情况的
2. 如果我对于某一个供应商添加了多个密钥的话，我希望在日志中能够体现出来每次请求使用的是哪一个密钥。并且密钥需要存储在数据库中，是否统一加密存储由用户在配置文件中决定
3. 也请你为 /models/{provider} 的错误场景（如 provider 不存在、无 API key、上游失败）也记录日志



我刚刚进行了测试，改动是成功的！
不过还有一些地方需要你进行小心地修改：
1. 为了未来的扩展考虑，我希望将配置文件中的部分内容放到数据库中进行存储，比如每个供应商已有的密钥，这样可以进行后续的动态扩展。而密钥的加密策略和我们现有的加密策略复用即可，不需要额外进行配置项的增加
2. 不需要将 cache=all 也改为“追加/更新”，保留现有的逻辑即可。不过我希望在 cache=selected 时支持 remove= 参数来移除不需要的模型，这也是为了动态修改而考虑的


你的这次改动我没有进行测试，因为我想和你说：
1. 你误解了我的“供应商密钥加密”的“复用已有加密配置选项”的想法，你看，你上次修改不是为我实现了可以在配置文件中使用 默认 masked（安全），可选 none / plain 这样的选择进行密钥的加密吗？我的意思是复用这个而不是创建一个新的加密方式呢，所以请你修改一下，你说的 通过环境变量 GATEWAY_SECRET 控制 太麻烦了，我追求的是配置文件定义一切，这样简单可靠
2. 请你安全暴露增删密钥的 HTTP 接口（例如 POST/DELETE /providers/{provider}/keys），因为这个功能确实是需要的
3. 你提出的 “对 cache=all 增加“保留列表”支持（例如 exclude=），或返回变更摘要（新增/删除/更新计数）？” 建议我觉得可以接受，请你小心实现




你是一名经验丰富的软件开发工程师，专注于构建高性能、健壮的解决方案。

你的任务是：**审查、理解并迭代式地改进现有代码库**

我当前项目最近的更新日志为：

```
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
```

我现在需要你按照要求进行仔细地优化和修改：

1. 我使用了 GET http://localhost:8080/models/openai?cache=selected&include=GLM-4.5,Qwen3-Coder-Instruct-MD 这样的请求方式测试了项目中已有的接口。但是测试结果并没有成功清除掉该供应商数据库中已经缓存的模型，只保留我选择的两个模型。
2. 同理，我使用了 GET http://localhost:8080/models/openai?cache=selected&remove=Bge-m3-SiliconCloud 尝试从数据库缓存中删除选择的模型也是失败的，数据库中仍有这个模型。
3. 并且，我希望对于这个接口，像这样的额外添加和删除的操作等，你觉得是否应该使用 POST 或者 DELETE 方法将其区别开来？

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




我接受你的建议，将当前这个复合的接口按照清晰的语义进行划分，因为虽然你给我了使用方式，但是我却因为参数和组合过多，而不知道如何去正确调用和进行测试



1. 对于原有的 GET 接口，移除掉已经独立出来的 DELETE 和 POST 方法，我不需要所谓的“兼容性”，我需要的是接口职责单一且语义简单可靠
2. 在你完成对接口的修改后，请你拆分和优化 src/server/handlers.rs 文件，该文件代码太多，而且有些功能是重复的 




我刚刚进行了接口测试，已经成功测试完毕划分出来合适的语义接口了，谢谢你的工作！
不过还有些地方需要优化：
1. 我看到你对 /models/:provider/cache 这个接口，数据发送是放在 Body(JSON) 中的，请你同样地将 DELETE /models/{provider}/cache?ids=id1,id2 这个接口改成 Body(JSON) 形式，这样便于一次性删除多个模型
2. 如果我使用 RUST_LOG=debug cargo run 命令来启动项目并且开启 tracing 的信息输出的话，终端日志显示的时间戳还是原始的形如 2025-09-27T04:30:13.252229Z 的形式，请你参考或者复用数据库的时间处理方法来解决这个问题
3. 项目中还有一部分地方的错误处理是采用 dyn 的动态 Trait 方式处理的，如果合适的话，请你将其统一为我们项目中已有的 error 来进行处理
4. 最后对于鉴权检查，我们先放一放，稍后再处理
本次的工作日志请你写入到 docs/Develop1-1.md 文件中去



我详细测试了一下 /v1/chat/completions 接口的工作情况，发现了一些细微的问题：
对于同一个模型 openai/Kimi-K2-Flow 进行对话测试，在都是非流式传输的前提下，使用：
```
{
    "model": "openai/Kimi-K2-Flow",
    "stream": false,
    "messages": [
      {
        "role": "user",
        "content": "如何 cargo run 的时候显示 tracing 的日志？"
      }
    ]
}
```
进行请求测试。当前项目的网关功能返回结果如下：
```
{
    "id": "chat-",
    "object": "chat.completion",
    "created": 1758950061,
    "model": "kimi-k2-0905",
    "choices": [
        {
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "在 Rust 里用 `tracing` 系列 crate（`tracing`、`tracing-subscriber` 等）时，默认情况下**不会输出任何日志**。  \n要让 `cargo run` 时把 `tracing` 的日志打印到终端，只需要在程序入口处把 subscriber 装上，并且把环境变量 `RUST_LOG` 设成想要的级别即可。\n\n1. 加依赖（`Cargo.toml`）\n\n```toml\n[dependencies]\ntracing = \"0.1\"\ntracing-subscriber = { version = \"0.3\", features = [\"env-filter\"] }\n```\n\n2. 在 `main.rs` 里装 subscriber（越早越好）\n\n```rust\nuse tracing::{info, warn};\n\nfn main() {\n    // 等价于 tracing_subscriber::fmt::init();\n    tracing_subscriber::fmt()\n        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())\n        .init();\n\n    info!(\"hello from tracing\");\n    warn!(\"this is a warning\");\n}\n```\n\n3. 运行\n\n```bash\n# 打印所有级别 >= DEBUG 的日志\nRUST_LOG=debug cargo run\n\n# 只打印当前包里的日志\nRUST_LOG=my_app=debug cargo run\n\n# 打印全链路 TRACE 级别\nRUST_LOG=trace cargo run\n```\n\n补充技巧\n- 不想每次敲环境变量，可以写在 `.cargo/config.toml`：\n\n```toml\n[env]\nRUST_LOG = \"info\"\n```\n\n- 只想看漂亮颜色，不要时间戳：\n\n```rust\ntracing_subscriber::fmt()\n    .with_target(false)\n    .without_time()\n    .init();\n```\n\n一句话总结：  \n**装 `tracing-subscriber`，`main()` 里 `tracing_subscriber::fmt::init()`，然后 `RUST_LOG=debug cargo run` 就能看见日志了。**"
            },
            "refs": null,
            "logprobs": null,
            "finish_reason": "stop",
            "service_tier": null
        }
    ],
    "usage": {
        "prompt_tokens": 27,
        "completion_tokens": 386,
        "total_tokens": 413,
        "prompt_tokens_details": null,
        "completion_tokens_details": null
    }
}
```
但是如果直接请求上游供应商，返回的内容如下：
```
{
    "id": "chat-",
    "object": "chat.completion",
    "created": 1758950433,
    "model": "kimi-k2-0905",
    "usage": {
        "prompt_tokens": 27,
        "completion_tokens": 426,
        "total_tokens": 453
    },
    "extend_fields": {
        "traceId": "210185d917589504231042354e10e6",
        "requestId": "c3d2d2182de4517c7455dcef5a1c48dd"
    },
    "choices": [
        {
            "index": 0,
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": "在 Rust 里用 `tracing`（或 `tracing-subscriber`）时，默认情况下 **Cargo 不会把低于 \"INFO\" 级别的日志打印出来**。  \n要让 `cargo run` 时能看到 **trace / debug** 日志，只需要把环境变量 `RUST_LOG` 设成你想要的过滤规则即可。\n\n------------------------------------------------\n1. 一次性运行（最常见）\n\n```bash\n# 打印当前 crate 的全部 trace 日志\nRUST_LOG=trace cargo run\n\n# 只想看 debug 级别\nRUST_LOG=debug cargo run\n\n# 只打印某个模块的 trace\nRUST_LOG=my_app::module1=trace cargo run\n\n# 同时给依赖 crate 也打开\nRUST_LOG=h2=debug,hyper=info,my_app=trace cargo run\n```\n\n------------------------------------------------\n2. 长期生效（免得每次敲）\n\n```bash\n# Linux / macOS\nexport RUST_LOG=debug          # 放 ~/.bashrc 或 ~/.zshrc\ncargo run\n\n# Windows PowerShell\n$Env:RUST_LOG=\"debug\"\ncargo run\n```\n\n------------------------------------------------\n3. 代码里别忘了装上 subscriber\n\n```rust\nuse tracing_subscriber;   // 在 Cargo.toml 里先加上\n\nfn main() {\n    // 一行就能装上，默认把日志打到 stdout\n    tracing_subscriber::fmt::init();\n\n    tracing::trace!(\"这条只有 RUST_LOG=trace 才看得见\");\n    tracing::debug!(\"这条需要 RUST_LOG=debug 及以上\");\n}\n```\n\n------------------------------------------------\n4. 更多过滤写法（可选）\n\n```bash\nRUST_LOG=trace,hyper=off       # 全局 trace，但把 hyper 关掉\nRUST_LOG=info,my_app=trace     # 全局 info，my_app 开 trace\n```\n\n------------------------------------------------\n一句话总结  \n**`RUST_LOG=trace cargo run`** 就能在终端里看到 tracing 的 trace 级日志；级别和模块名用英文逗号分隔，想持久化就写进 shell 的环境变量。"
            }
        }
    ]
}
```
看似好像没有什么区别（我们先忽略其他的属性，只关注 content 内模型回答的内容），但是如果放在真实的聊天客户端中进行 Markdown 渲染的时候，我们当前项目返回的上游结果会出现很大概率的缺少部分字符的 bug，但是直接请求上游则基本没有见到该问题。当然，这也许是模型本身的问题，因为我更换另外一个模型似乎没有出现类似的问题。

这是非流式的请求情况，我觉得我们当前项目的对话处理逻辑似乎有些复杂，请你使用 context7 MCP 来获取一个请求示例，也许我们需要改进一下当前的请求逻辑。请你仔细分析，我们当前的 /v1/chat/completions 接口非流式请求的时候，逻辑是否合理




我进行了一些测试，观察后发现大部分情况下都能正确解析了，不过为了保险起见，请你在 ai-gateway/ai-gateway 路径下查找对话的功能逻辑代码，这个路径下是一个成熟的项目，请你参考其中的代码逻辑来优化我们当前的代码逻辑



我的想法是尽量对齐 ai-gateway 的设计思路，因为它是成熟的，我们还在探索，所以直接使用成熟的解决方案会比我们这样逐步修改提高代码复杂性的可靠性更高。所以我觉得引入适当的依赖也有助于减少编码复杂度和代码量。因此请你告诉我我需要添加什么依赖？我来主动添加，因为我要去搜索是否有新版本，而不是你直接为我添加。当我完成依赖添加之后，请你告诉我该做些什么



我已经在项目中添加好了 reqwest-eventsource = "0.6.0" 依赖，请你开始对齐工作，以让我们的项目获得更高的成熟度和稳定性。而且如果单个文件代码量超过 200 行左右，那么请你进行合理地拆分和组织



不错的改动，我进行了测试并没有发现什么奇怪的问题出现。
而对于你之前给我的“若你希望非流式也完全改为 reqwest-eventsource 解析（不再使用 bytes_stream 回退）”建议，我好奇这个 “bytes_stream 回退” 是我们项目中之前使用的非流式处理逻辑吗？我现在想要让你继续对齐 ai-gateway 的设计思路，因为直接使用成熟的解决方案会比我们这样逐步修改提高代码复杂性的可靠性更高。
所以如果 “bytes_stream 回退” 是我们项目之前采用的非流式处理逻辑而不是 ai-gateway 的设计思路的话，就将其作为冗余代码移除掉，保持我们项目的简洁。
最后，当你处理好对齐后，统一错误为 GatewayError 而不是原始的闭包处理。
补充：ai-gateway 的设计思路在 ai-gateway/ai-gateway 路径下。请你仔细学习和在我们当前项目中对齐，如有什么需要添加的依赖，请你告诉我，我会手动添加以保持版本最新




1. 我在进行请求测试的时候，上游有时会返回一些未知的错误信息，比如：tream error: Invalid status code: 554 <unknown status code>
请你修改一下数据库存储日志的逻辑，将这种出错的上游供应商返回的消息也记录到数据库中，可以通过添加一个新的属性字段来存储。
2. 继续统一 models 相关路径的错误为 GatewayError，以保持代码的简洁性。
3. 请你继续参考 ai-gateway/ai-gateway 路径下的对接上游的处理逻辑来将我们当前项目中的请求逻辑对齐，如果有改进的地方的话。如果你觉得我们项目暂时不需要进行额外的，对齐 ai-gateway/ai-gateway 项目，那么就不要修改，并且告诉我为什么。




你是一名经验丰富的软件开发工程师，专注于构建高性能、健壮的解决方案。

你的任务是：**审查、理解并迭代式地改进现有代码库**

我们当前项目中的聊天请求逻辑暂时是可以告一段落了。那么请你继续为我小心实现：
1. 将我们原本实现的从 custom-config.toml 文件中读取的：
```
[providers.openai]
name = "openai"
api_type = "openai"
base_url = "https://apis.134257.xyz"
api_keys = ["sk-giseB9gAQ3zi7jNRwXURl7K24vfbDvxchsDefOxysHAeeX0c"]
models_endpoint = "/v1/models"
```
内容，全部采用数据库进行存储，以便于动态扩展。
请你新建一个数据库表来存放不同供应商的信息以取代我们原有的通过配置文件来添加的方式。
2. 在你完成了第 1 个任务之后，不要忘记清理掉我们用于从 custom-config.toml 文件中读取配置的代码，不需要保留这个兼容性了，我们要保持代码的简洁。

在你完成所有的任务之后，将任务日志也精要地追加到 docs/Develop1-2.md 文件中去，并且使用中文。碰到语法错误和问题，请你使用 context7 MCP 来获取最新的开发文档。

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



请你继续修改：
1. 请你移除掉我们原有的 30min 或者多久刷新一次模型缓存的机制，因为我们已经有了用户自定义请求和添加模型缓存的接口，所以为了模型缓存名称的稳定性，不要有“自动刷新”的逻辑
2. 请你为我继续添加管理供应商的 REST 接口，不需要提供 DB 初始化脚本了。一切采用接口进行标准处理。而且，添加的新的 REST 接口一定要遵循标准的语义化和接口单一职责


很好的修改，暂时不需要把 POST /providers 已存在时的返回调整为标准 409 Conflict，而是：
1. 你看我们数据库中目前有个叫做 sqlite_sequence 的表，然后我们不是现在已经有了对供应商的 curd 操作了吗？所以请你新增加一个操作类型，就是对供应商操作的类型和新的日志表，将对供应商的操作也记录入库




不用补充运维接口了，我刚刚进行了测试，很不错。但是我观察到当我为指定供应商添加密钥的时候，这个操作并没有被记录下来，请你也补充上去。对于密钥的处理遵循原有的：
  - 数据库存储：
    - `plain` → 明文存储
    - `masked`/`none` → 可逆轻量混淆存储（基于 provider+固定盐 异或+hex），便于后续切换
  - 日志展示：
    - `none` 不记录
    - `masked` 记录首尾4位
    - `plain` 记录明文（仅建议在安全环境中使用）
配置。
不过我目前不知道这个配置在哪里进行修改了，之前的时候我一直是没有使用这个配置，也就是采用默认的策略的（默认的策略是什么），现在改成数据库存储了，请问如何修改配置呢？



不需要添加了，现在已经很好了。但是我发现一个小问题：
1. 使用 /providers/:provider/keys 接口删除指定供应商的指定密钥的时候，哪怕密钥是随便写的，也是可以操作成功的（虽然并不会影响数据库中的实际数据），你觉得这样是合理的吗？如果不合理，请你修复
2. 在你完成了上面的修改后，请你清理一下我们当前项目中未使用的引入的警告吧



好的，请你引入 NotFound 错误类型并统一改造错误映射；我看到你使用了 #[allow(dead_code)] 注解，使用这个注解合适么？是否会影响到你对于项目中方法的判断和使用？如果可能会有影响，那么我宁愿让你保留警告而不是使用这个可能会引起歧义的注解。而对于修改密钥策略配置，就保持在配置文件中进行定义吧，不用修改了



好的，请你统一将 chat 路径中“指定 provider 前缀不存在”的情况改为 404。然后请问当前项目中有直接获取指定供应商所有的密钥情况的接口吗？


好的，请你补上 GET /providers/{provider}/keys，并在 docs/Develop1-2.md 说明接口语义和安全注意事项








你是一名经验丰富的软件开发工程师，专注于构建高性能、健壮的解决方案。

你的任务是：**审查、理解并迭代式地改进现有代码库**

现在，我们将工作重心重新放到 /v1/chat/completions 接口上来，目前这个接口功能和我们已有的 openai 数据结构是很脆弱的：
1. 我的上游供应商提供了一个可以识别图片的模型，但是通过我们当前项目的接口进行转发对话的时候，却返回了：OK Failed to deserialize the JSON body into the target type: messages[0].content: invalid type: sequence, expected a string at line 1 column 90 错误，这很显然是不合理的

所以当前的工作重心是需要你对齐 ai-gateway/ai-gateway 路径下的 ai-gateway 项目的对于识图模型的解决方案，因为直接使用成熟的解决方案会比我们这样逐步修改提高代码复杂性的可靠性更高。
我猜测对于图片的处理是将图片转为 base64 编码来进行处理，但是这个是我的猜测，具体实现方式你一定要对齐 ai-gateway/ai-gateway 路径下的 ai-gateway 项目的成熟的解决方案。
如果你发现 ai-gateway 项目采用了额外的依赖来解决了这个问题，那么请你将需要添加的依赖告诉我，我来进行添加，

如果你在编码的过程中碰到语法了错误和问题，请你使用 context7 MCP 来获取最新的开发文档来解决。


在你完成所有的任务之后，将任务日志也精要地追加到 docs/Develop2.md 文件中去，并且使用中文。

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




很好的修改，现在已经可以正常对图片进行对话了。
1. 当然要加入 async-openai 并完全替换本地类型，以获得最严谨的类型约束与更全字段覆盖了。还记得我和你说的吗？如果有必要的话，引入新的依赖可以精简代码并获得更好的效果的话，那么就要加上依赖，而不是全都自己手写。我已经加上了 async-openai = "0.29.3" 依赖了，请你开始修改吧！
2. 不需要添加一个最小 curl 示例（含 base64 图片）到 docs 里




1. 当前项目中出现了错误，请你仔细修复：

```
error[E0432]: unresolved import `async_openai::types::Usage`
  --> src/providers/openai/types.rs:14:5
   |
14 |     Usage,
   |     ^^^^^ no `Usage` in `types`
   |
   = help: consider importing this struct instead:
           async_openai::types::responses::Usage

error[E0432]: unresolved import `crate::providers::openai::Role`
 --> src/providers/anthropic.rs:2:120
  |
2 | use crate::providers::openai::{ChatCompletionRequest, ChatCompletionResponse, Message as OpenAIMessage, Choice, Usage, Role};
  |                                                                                                                        ^^^^ no `Role` in `providers::openai`
  |
  = help: consider importing one of these enums instead:
          crate::providers::anthropic::oai::responses::Role
          crate::providers::openai::types::Role
          async_openai::types::Role
          async_openai::types::responses::Role

error[E0432]: unresolved import `crate::providers::openai::Usage`
  --> src/server/streaming_handlers.rs:10:55
   |
10 | use crate::providers::openai::{ChatCompletionRequest, Usage};
   |                                                       ^^^^^ no `Usage` in `providers::openai`
   |
   = help: consider importing one of these items instead:
           crate::providers::openai::types::Usage
           async_openai::types::responses::Usage

error[E0432]: unresolved import `super::types::FinishReason`
  --> src/providers/openai/client.rs:10:101
   |
10 |     ChatCompletionRequest, ChatCompletionResponse, Choice, Message, ModelListResponse, Usage, Role, FinishReason,
   |                                                                                                     ^^^^^^^^^^^^ no `FinishReason` in `providers::openai::types`
   |
   = help: consider importing this enum instead:
           async_openai::types::FinishReason

warning: unused imports: `CompletionTokensDetails`, `CreateChatCompletionStreamResponse`, `Logprobs as LogProbs`, and `PromptTokensDetails`
  --> src/providers/openai/types.rs:9:5
   |
 9 |     CreateChatCompletionStreamResponse,
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
10 |     Logprobs as LogProbs,
   |     ^^^^^^^^^^^^^^^^^^^^
11 |     PromptTokensDetails,
   |     ^^^^^^^^^^^^^^^^^^^
12 |     CompletionTokensDetails,
   |     ^^^^^^^^^^^^^^^^^^^^^^^
   |
   = note: `#[warn(unused_imports)]` on by default

error[E0308]: mismatched types
  --> src/providers/openai/client.rs:86:38
   |
86 | ...                   created: Utc::now().timestamp() as u64,
   |                                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `u32`, found `u64`

warning: use of deprecated field `async_openai::types::CreateChatCompletionRequest::max_tokens`
  --> src/providers/anthropic.rs:72:25
   |
72 |             max_tokens: openai_req.max_tokens.unwrap_or(1024),
   |                         ^^^^^^^^^^^^^^^^^^^^^
   |
   = note: `#[warn(deprecated)]` on by default

warning: use of deprecated field `async_openai::types::ChatCompletionResponseMessage::function_call`
  --> src/providers/anthropic.rs:92:17
   |
92 |                 function_call: None,
   |                 ^^^^^^^^^^^^^^^^^^^

error[E0308]: mismatched types
   --> src/providers/anthropic.rs:102:22
    |
102 |             created: chrono::Utc::now().timestamp() as u64,
    |                      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `u32`, found `u64`

error[E0063]: missing fields `service_tier` and `system_fingerprint` in initializer of `CreateChatCompletionResponse`
  --> src/providers/anthropic.rs:99:9
   |
99 |         ChatCompletionResponse {
   |         ^^^^^^^^^^^^^^^^^^^^^^ missing `service_tier` and `system_fingerprint`

Some errors have detailed explanations: E0063, E0308, E0432.
For more information about an error, try `rustc --explain E0063`.
warning: `gateway` (bin "gateway") generated 3 warnings
error: could not compile `gateway` (bin "gateway") due to 7 previous errors; 3 warnings emitted
```

2. 移除现已无用的本地流式结构定义 src/providers/streaming.rs
3. Anthropic 有关的部分暂时先放着，先解决已有的问题再进行处理




这次编译通过了而且项目也成功启动并且使用也是正常的。但是很显然，因为你的修改，导致现在请求后的日志中并不能包含 token 统计了。之前很显然是可以的，所以请你：
1. 将我们原本有的统计功能加回来
2. 不要因为将类型改为了 async-openai 带的类型而将我们原有的好的功能给抛弃掉
3. 我还是希望你再仔细看看 ai-gateway/ai-gateway 路径下的 ai-gateway 项目是如何处理这个情况的，如果这个项目也没有记录上游 token 信息的话，那我们要保留项目中原有的记录功能，这就是我们的特色
4. 在你完成上面的修改后，max_tokens、function_call 这两个特性也要进行实现，因为这两个功能是很重要的，也请你参考 ai-gateway/ai-gateway 路径下的 ai-gateway 项目来进行实现



我刚刚对于模型的工具调用功能进行了测试，我发现工作是正常的，token 消耗日志也恢复了正常。不过我还有一些疑问：
1. 你说“非流式 SSE 聚合响应依旧保底返回 Usage（为空则填充 0），但当出现 function_call/tool_calls 时改为回退 JSON，避免丢失结构”，我可以理解为我们当前项目有两种策略么?这个是有必要的吗？没有办法通过一个统一的超集去覆盖到所有情况吗？ai-gateway/ai-gateway 路径下的 ai-gateway 项目也是这样进行处理的吗？
2. 如果必要的话再精简一下未使用的 re-export 以减少编译警告，如果没有必要或者可能会导致歧义，那么不要精简
3. 如果 ai-gateway/ai-gateway 路径下的 ai-gateway 项目有对 Anthropic 的多模态转译与 system/developer 消息优化对齐的成熟的思路，那么请你将其应用到我们当前项目中来。如果有需要添加的依赖，请你告诉我




1. 如果你可以在保留我们项目原有的 token 等记录功能的同时完全对齐 ai-gateway 并进一步降低复杂度的话，那么请你这么做来进一步优化和对齐。并且在你这么做之后详细告诉我和我们原来的区别是什么，以及对齐是否是有必要的
2. 我已经按照你的要求添加好了 anthropic-ai-sdk = "0.2.25" 依赖，你可以进行使用和继续对齐 ai-gateway 对 Anthropic 的处理逻辑了






GET /providers/{provider}/keys
行为：返回该供应商密钥的掩码列表
加一个仅供管理员使用的“明文查看”开关吗？默认继续保持安全的掩码返回。


当前项目中，你已经为我实现了当我删除掉一个供应商的时候，同步删除其密钥和缓存的模型。然后我们不是现在又实现了对于密钥删除的日志记录吗？所以请你将这些日志都同步记录进去可以么？如果过于复杂，或者其实没有必要，则不用实现。


我们现在可以开始实现管理员和普通用户的功能了。请你先实现：
1. 如果你知道 OneApi 或者 NewApi 这任意一个项目的话，你就知道这两个 AI 网关项目是管理员才可以创建供应商和添加修改模型和为每个供应商的密钥进行管理，还可以对用户进行 curd 的管理，同时可以修改每个用户的“余额”和可以使用的模型。并且管理员还可以创建属于自己的“令牌”，通过这个新的“令牌”来去调用添加好的供应商提供的模型（而且管理员也可以限制自己的某个令牌可以调用的模型，以及可以使用的余额，超过了就自动禁用令牌，而且还可以设定令牌的过期时间），从而实现所有的供应商的模型聚合，达成 AI 网关的目的；而普通用户的权利只有创建属于自己的模型调用“令牌”的权限，并且也无法修改自己的用户“余额”，只能修改自己对于每个“令牌”的可用余额和过期时间以及可以调用的模型（也就是在管理员对这个用户开放的模型权限的基础上，用户可以再单独对自己的某个令牌做模型调用限制），这个“令牌”可用“余额”看似可以随意修改，但是会受到用户本身的“余额”的约束。


3. 在你完成上面的任务之后，为当前的对话接口加一个超时机制，因为我在测试的时候碰到过上游供应商很久都没有返回的情况，但是却一直在等待，所以请你先进
数据库使用 Postgresql 
识图、思考、工具调用、MCP
密钥分发、TUN 或者 GUI