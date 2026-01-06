# Gateway Zero 快速上手指南

## 🔑 重要概念：两种认证方式

```
┌─────────────────────────────────────────────────────────────────┐
│  1. Admin Token（API 调用认证）                                  │
│     ├─ 用途：第三方应用/SDK 调用 /v1/chat/completions 等 API    │
│     ├─ 格式：随机字符串，如 "my-token-12345"                    │
│     ├─ 存储：PostgreSQL admin_tokens 表                         │
│     └─ 使用：curl -H "Authorization: Bearer <token>"            │
│                                                                 │
│  2. Ed25519 管理员密钥（TUI 登录认证）← 你已有的                  │
│     ├─ 用途：管理员通过 TUI/Web 登录管理后台                     │
│     ├─ 格式：Ed25519 公私钥对                                   │
│     ├─ 存储：私钥文件 + PostgreSQL admin_keys 表                │
│     └─ 使用：TUI 客户端签名登录                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 🚀 方案 A：直接启动全部服务（推荐！）

如果你已经配置好环境，直接用现有命令启动即可：

### 终端 1：启动数据库和后端
```bash
# 启动 PostgreSQL 数据库
docker start gateway-postgres

# 启动后端（release 模式）
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero
source ~/.cargo/env
RUST_LOG=info ./target/release/gateway &
```

### 终端 2：启动前端
```bash
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero/frontend
pnpm dev &
```

### 验证服务是否正常
```bash
# 检查后端
curl http://localhost:8080/v1/models

# 检查 PostgreSQL
docker exec gateway-postgres psql -U postgres -c "SELECT 1"
```

---

## 🔧 方案 B：开发模式逐步启动

如果你想深入学习，可以按以下步骤逐步操作：

### 步骤 1：启动 PostgreSQL（终端 1）
```bash
# 启动已有的 PostgreSQL 容器
docker start gateway-postgres

# 验证数据库连接
docker exec -it gateway-postgres psql -U postgres -d gateway -c "\dt"
```

### 步骤 2：创建 Admin Token（用于 API 调用）

**注意**：这个 Token 和 TUI 登录用的管理员密钥是**不同的东西**！

**方式 A：通过 Web/TUI 界面创建**（推荐）
1. 启动前端：`cd frontend && pnpm dev`
2. 打开 http://localhost:5173
3. 使用 TUI 的管理员密钥登录
4. 在"令牌管理"页面创建新 Token

**方式 B：直接操作 PostgreSQL**
```bash
# 进入 PostgreSQL
docker exec -it gateway-postgres psql -U postgres -d gateway

# 创建 Token（在 psql 提示符下执行）
INSERT INTO admin_tokens (
    name,
    token, 
    enabled, 
    created_at, 
    amount_spent, 
    prompt_tokens_spent, 
    completion_tokens_spent, 
    total_tokens_spent
)
VALUES (
    'my-first-token',
    'my-api-token-12345', 
    true, 
    NOW(), 
    0, 
    0, 
    0, 
    0
);

-- 查看结果
SELECT token, enabled, created_at FROM admin_tokens;

-- 退出
\q
```

### 步骤 3：启动后端（终端 2）
```bash
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero
source ~/.cargo/env

# 开发模式（带详细日志）
RUST_LOG=debug cargo run

# 或使用已编译的 release 版本
RUST_LOG=info ./target/release/gateway
```

### 步骤 4：启动前端（终端 3）
```bash
cd /home/Geoff001/Code/Project/Graduation_Project/gateway_zero/frontend
pnpm dev
```

---

## ✅ 验证服务（终端执行）

### 步骤 5：测试 Admin Token

```bash
curl http://localhost:8080/v1/token/balance \
  -H "Authorization: Bearer my-test-token-12345"
```

**成功输出**：
```json
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

---

### 步骤 4：添加 Provider（终端 2）

```bash
# 创建 Provider
curl -X POST http://localhost:8080/providers \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-openai",
    "api_type": "OpenAI",
    "base_url": "https://api.openai.com/v1",
    "enabled": true
  }'

# 添加 API Key（替换为你的真实 Key）
curl -X POST http://localhost:8080/providers/my-openai/keys \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "key": "sk-proj-你的真实Key",
    "description": "My test key"
  }'
```

---

### 步骤 5：发送聊天请求（终端 2）

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "my-openai/gpt-3.5-turbo",
    "messages": [
      {"role": "user", "content": "Hello! Say hi in one sentence."}
    ]
  }'
```

---

### 步骤 6：查看统计（终端 2）

```bash
# 查看你的使用量
curl http://localhost:8080/v1/token/usage \
  -H "Authorization: Bearer my-test-token-12345"

# 查看全局统计
curl http://localhost:8080/admin/metrics/summary \
  -H "Authorization: Bearer my-test-token-12345"
```

---

### 步骤 7：测试流式响应（终端 2）

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-test-token-12345" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "my-openai/gpt-3.5-turbo",
    "messages": [
      {"role": "user", "content": "Count from 1 to 3"}
    ],
    "stream": true
  }'
```

观察逐字输出的效果！

---

## 🔧 可选：使用 Postman 或其他工具

如果你不喜欢 curl，也可以用：

### Postman
1. 创建新请求
2. 方法选 POST
3. URL: `http://localhost:8080/v1/chat/completions`
4. Headers:
   - `Authorization`: `Bearer my-test-token-12345`
   - `Content-Type`: `application/json`
5. Body (raw JSON):
```json
{
  "model": "my-openai/gpt-3.5-turbo",
  "messages": [
    {"role": "user", "content": "Hello!"}
  ]
}
```

### HTTPie（更友好的 curl）
```bash
# 安装
pip install httpie

# 使用
http POST localhost:8080/v1/chat/completions \
  Authorization:"Bearer my-test-token-12345" \
  model="my-openai/gpt-3.5-turbo" \
  messages:='[{"role":"user","content":"Hello!"}]'
```

---

## 📝 常见问题

### Q: 没有 OpenAI API Key 怎么办？
A: 可以用其他兼容 API，例如：
- **智谱 AI**（免费额度）：https://open.bigmodel.cn/
- **DeepSeek**（便宜）：https://platform.deepseek.com/
- **Moonshot**：https://platform.moonshot.cn/

只需修改 `base_url` 和 `api_key`。

### Q: 如何查看数据库内容？
```bash
sqlite3 data/gateway.db

sqlite> .tables  # 查看所有表
sqlite> SELECT * FROM admin_tokens;  # 查看 tokens
sqlite> SELECT * FROM request_logs ORDER BY timestamp DESC LIMIT 5;  # 最近5条日志
sqlite> .exit
```

### Q: 如何重置数据库？
```bash
rm data/gateway.db
# 重启服务器会自动创建新数据库
```

### Q: curl 命令太长，有简化方法吗？
创建环境变量：
```bash
export TOKEN="my-test-token-12345"
export BASE_URL="http://localhost:8080"

# 然后可以这样用
curl $BASE_URL/v1/token/balance -H "Authorization: Bearer $TOKEN"
```

---

## 🎯 学习检查清单

完成这些步骤后，你应该能够：
- [ ] 启动网关服务器
- [ ] 创建和管理 Admin Token
- [ ] 添加 AI Provider 和 API Key
- [ ] 发送非流式聊天请求
- [ ] 发送流式聊天请求
- [ ] 查看使用统计和日志
- [ ] 理解请求从客户端到 Provider 的完整流程

---

## 📚 下一步

完成快速上手后，建议：
1. 阅读 `LEARNING_GUIDE.md` 深入学习
2. 查看 `ARCHITECTURE.md` 理解架构设计
3. 打开 `architecture.svg` 查看可视化架构图
4. 开始阅读源码，从 `src/main.rs` 开始

祝学习顺利！🚀
