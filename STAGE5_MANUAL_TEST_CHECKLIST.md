# 阶段 5 手动测试清单

本轮目标是：在已完成 provider registry、动态表单、草稿测试与配置建模的基础上，把首批已真实接入的国际端点推进到“可手动测试”的状态。

## 本轮可手动测试的 provider

- `azure_openai`：支持创建/编辑配置、草稿连接测试、手动补模型、非流式真实请求
- `google_gemini`：支持创建/编辑配置、草稿连接测试、自动发现/手动补模型、非流式真实请求
- `cohere`：支持创建/编辑配置、草稿连接测试、自动发现/手动补模型、非流式真实请求
- `openai`、`anthropic`、`zhipu`、`doubao`：继续保持现有真实运行能力
- OpenAI-compatible 且当前已工作的 `cloudflare`、`perplexity`、`mistral`、`deepseek`、`siliconcloud`、`moonshot`、`alibaba_qwen`、`custom`、`xai`：继续按既有链路运行

## 当前限制

- `azure_openai`、`google_gemini`、`cohere`：本轮只保证非流式真实请求；`stream=true` 会返回明确的“暂未实现”提示
- `azure_openai`：继续采用 `deployment + api-version + 手动模型优先` 策略，不依赖自动模型发现
- `aws_claude`：仍缺少 AWS SigV4 正式签名链路，本轮仅保留配置语义边界
- `vertex_ai`：仍缺少 OAuth / GCP 凭证链路，本轮仅保留配置语义边界

## 创建渠道时的关键字段

### Azure OpenAI

- `上游 API 地址`：Azure 资源根地址，例如 `https://{resource}.openai.azure.com`
- `API Key`：Azure API Key
- `Azure Deployment`：必填
- `Azure API Version`：必填
- `模型`：手动补录，建议与 deployment 实际对应模型保持一致

### Google Gemini

- `上游 API 地址`：建议使用 `https://generativelanguage.googleapis.com/v1beta`
- `API Key`：Gemini API Key
- `Gemini API Version`：可留默认 `v1beta`，也可显式填 `v1`
- `模型`：可自动发现，也可手动补录，例如 `gemini-2.0-flash`

### Cohere

- `上游 API 地址`：建议使用 `https://api.cohere.com`
- `API Key`：Cohere API Key
- `模型`：可自动发现，也可手动补录，例如 `command-r-plus`

## 最小闭环验证步骤

1. 在 `captok` 渠道管理页创建 provider
2. 填写上游地址、Key、专属配置字段，并保存至少一个模型
3. 在创建/编辑页点击“测试连接”确认草稿配置可连通
4. 若支持自动发现模型，则打开模型选择弹窗获取模型；若不支持，则手动补模型
5. 保存渠道并确保该模型已配置价格、未被禁用
6. 使用有效 Client Token 调用 `POST /v1/chat/completions`
7. 先验证 `stream=false` 的普通请求闭环，再按需要验证已支持的流式 provider

## 真实请求示例

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H 'Authorization: Bearer <client-token>' \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "<provider>/<model>",
    "messages": [
      {"role": "user", "content": "hello"}
    ],
    "stream": false
  }'
```

示例模型：

- Azure OpenAI：`azure-provider/gpt-4o`
- Google Gemini：`gemini-provider/gemini-2.0-flash`
- Cohere：`cohere-provider/command-r-plus`

## 手动测试前检查

- provider 已启用
- 至少存在一个启用中的 provider key
- 模型已加入 provider，可用于请求
- 模型价格已配置，否则网关会拒绝真实请求
- Client Token 已启用且额度/过期状态正常
- 若请求使用 `provider/model` 格式，确认 provider 名称与后台渠道名一致
