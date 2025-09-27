# 阶段一（续）开发日志：供应商配置迁移到数据库

日期：2025-09-27

本次改动聚焦于将原先从 `custom-config.toml` 读取的供应商配置彻底迁移到数据库，删去文件方式的兼容路径，保持实现简单、单一职责并利于后续动态扩展。

变更摘要
- 新增数据表：`providers(name TEXT PRIMARY KEY, api_type TEXT, base_url TEXT, models_endpoint TEXT NULL)`。
- 提供数据库访问方法：列出/查询/存在性检查/更新供应商；列出供应商名；按需带出密钥。
- 运行期改为从数据库读取供应商与密钥，不再依赖 `custom-config.toml` 中的 `[providers.*]` 配置。
- 清理所有通过配置文件访问 `config.providers` 的调用路径，改为使用数据库接口。
- 保留服务器与日志等非供应商配置的 TOML 解析（仅移除供应商部分的文件兼容逻辑）。
- 移除模型缓存的“自动刷新/新鲜度检查”逻辑，改为完全由接口显式触发（避免命名稳定性被动变化）。

涉及文件
- 新增：`src/logging/database_providers.rs`（供应商表的 CRUD 与查询）
- 修改：
  - `src/logging/database.rs`：初始化 `providers` 表
  - `src/logging/mod.rs`：导出 `database_providers` 模块
  - `src/config/settings.rs`：移除 `Settings.providers` 字段及相关 TOML 读取逻辑
  - `src/server/mod.rs`：移除启动时从配置导入密钥的逻辑
  - `src/server/provider_dispatch.rs`：从数据库选择供应商与密钥，替代基于配置文件的选择
  - `src/server/handlers/{models.rs,cache.rs,provider_keys.rs}`：校验与查询供应商改为走数据库，密钥只从数据库读取
  - `src/server/model_cache.rs`：统计与遍历供应商改为从数据库列名；删除新鲜度检查辅助方法
  - `src/logging/database_cache.rs`、`src/server/storage_traits.rs`：去除缓存“新鲜度”相关接口
- 新增 REST：`src/server/handlers/providers.rs` 与 `src/server/handlers/mod.rs` 路由注册
  - 新增审计日志：`provider_ops_logs` 表与写入逻辑，含操作类型（list/get/create/update/delete）

设计与原则
- KISS：供应商信息统一从数据库读取，避免双通道（文件+数据库）的复杂性。
- YAGNI：仅实现当前所需的增/查接口，不预留额外未使用的管理接口。
- SOLID：
  - 单一职责：数据库接口按“日志/密钥/模型缓存/供应商”分文件组织。
  - 开闭原则：后续新增供应商属性无需修改业务层，仅扩展表/接口即可。
- DRY：删除配置文件路径上的重复逻辑，统一改为数据库查询。

迁移后行为
- 供应商元数据（`name/api_type/base_url/models_endpoint`）与密钥均存储在 SQLite。
- 负载均衡从数据库读取供应商及其密钥列表后进行选择。
- 所有“供应商是否存在”的校验均基于 `providers` 表。
- 不再从 `custom-config.toml` 读取任何供应商相关配置；若文件仍包含 `[providers.*]` 节点，将被忽略。
- 模型缓存不再存在任何定时或阈值触发的“自动刷新”。仅通过以下接口显式拉取或写入缓存：
  - `GET /models/{provider}?refresh=true`：仅拉取上游返回，不落库
  - `POST /models/{provider}/cache`：按请求体策略覆盖/追加缓存

使用提示
- 初次运行前需将供应商信息写入 `providers` 表（例如：openai）。可通过 SQLite 客户端插入：
  ```sql
  INSERT INTO providers(name, api_type, base_url, models_endpoint)
  VALUES ('openai', 'openai', 'https://apis.134257.xyz', '/v1/models');
  ```
  密钥请使用已有接口：
  - 添加密钥：`POST /providers/:provider/keys`，Body: `{ "key": "sk-..." }`
  - 删除密钥：`DELETE /providers/:provider/keys`，Body: `{ "key": "sk-..." }`

供应商管理接口（语义化、单一职责）
- 列表：`GET /providers` → 200，返回 `[ProviderOut]`
- 查询：`GET /providers/{provider}` → 200 | 404
- 创建：`POST /providers`，Body: `{name, api_type, base_url, models_endpoint?}` → 201 | 409
- 更新：`PUT /providers/{provider}`，Body: `{api_type, base_url, models_endpoint?}` → 200（更新）| 201（创建）
- 删除：`DELETE /providers/{provider}` → 204 | 404（同时清理该供应商的密钥与模型缓存）
- 列出密钥：`GET /providers/{provider}/keys` → 200 | 404，返回 `{ "keys": ["sk-12****34", ...] }`（始终掩码）

审计记录
- 新表：`provider_ops_logs(id, timestamp, operation, provider, details)`；时间为北京时间字符串。
- 每次供应商接口调用都会写入该表，并同步写入通用 `request_logs`（使用新 request_type）。密钥列表请求仅记录操作，不记录密钥内容。

安全注意事项（密钥）
- API 响应：`GET /providers/{provider}/keys` 总是返回掩码后的密钥（首尾4位），与 `key_log_strategy` 配置无关，避免通过接口泄露明文密钥。
- 日志落库：遵循 `key_log_strategy`：
  - `none`：不记录密钥
  - `masked`（默认）：记录首尾4位
  - `plain`：记录明文（仅建议在安全环境中使用）
- 数据库存储：
  - `plain` → 明文
  - `masked`/`none` → 可逆轻量混淆（provider+固定盐 异或+hex）

如何修改“密钥存储/展示”策略

策略所在配置项
文件配置：custom-config.toml 或 config.toml（两者其一）
路径/字段：[logging].key_log_strategy
枚举值："none" | "masked" | "plain"
默认策略
默认是 masked（即：数据库混淆存储 + 日志仅首尾4位）
修改示例
将日志与存储都改为明文：
[logging]
key_log_strategy = "plain"
将数据库混淆存储，日志不记录密钥：
[logging]
key_log_strategy = "none"
说明
虽然供应商信息改为数据库管理，但 LoggingConfig 仍从 TOML 配置文件中读取；修改后重启服务生效。
若未设置该字段，程序默认使用 masked。


备注
- 若遇到语法或 API 使用疑问，请通过 context7 MCP 获取最新文档核对。
