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