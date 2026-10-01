# Bamboo API 文档

欢迎使用 Bamboo AI Agent API 文档。Bamboo 提供完全自包含的 AI Agent 后端框架，内置 HTTP/HTTPS 服务器能力。

## 概览

Bamboo 提供一组 RESTful API 端点，用于创建和管理 AI Agent 会话、执行 agent 循环以及流式传输实时事件。

## 基础 URL

```
http://localhost:9562/api/v1
```

## API 端点

### 聊天操作

#### 创建聊天消息

```http
POST /api/v1/chat
```

创建新的聊天会话，或向现有会话添加消息。

当客户端可能在结果不明确的超时后重试时，可以附带可选的 `Idempotency-Key` 请求头。在 10 分钟内，等价的重试会返回第一次的响应，而不会再次追加该消息。用同一个 key 搭配不同的 payload 复用时，会返回 `409 idempotency_key_conflict`。回执仅存在于进程内且数量有限；省略该请求头则保持正常行为。

**请求体：**

```json
{
  "message": "Help me write a function",
  "session_id": "optional-session-id",
  "model": "claude-sonnet-4-6",
  "system_prompt": "You are a helpful assistant",
  "enhance_prompt": "Additional instructions",
  "workspace_path": "/path/to/workspace"
}
```

Root 会话可以在聊天请求中通过 `"root_orchestration_prompt": true` 选择启用委派指导。该选择会随会话一起保存，并作用于首次执行以及后续恢复；省略该字段表示保持当前选择，填 `false` 则将其关闭。该指导覆盖子任务规划、进度检查、纠偏、范围控制与最终证据。子会话不会收到该指导。该 prompt 选择不会改变工具权限。

Root 会话还可以选择 `"root_orchestration_only": true`。这一持久化执行模式即使在未设置 `root_orchestration_prompt` 时也会提供同样的委派指导。它将 Root 限制为恰好以下九个工具执行身份：`SubAgent`、`Plan`、`Task`、`session_history_current`、`Read`、`Grep`、`Glob`、`GetFileInfo` 和 `ViewImage`。其他工具（包括 shell 与编辑类工具）对该 Root 不可用；被委派的子会话保留各自的工具权限。该模式与仅 prompt 的选择彼此独立。

针对新 Root 的第一次聊天请求可以通过 `root_orchestration_only` 选择该模式。对于**已存在**的 Root，应在 `POST /chat` 中省略该字段，并使用下文可恢复的模式操作。在已有 Root 的聊天请求中显式传入该值，会在追加消息之前返回 `428 root_mode_operation_required`。Child 会话无法选择或清除该模式。权威的选择结果由 `GET /api/v1/sessions/{session_id}` 返回；客户端应在重新加载后读取该结果，而不是把本地选择当作已持久化的状态。

**响应：** `201 Created`

```json
{
  "session_id": "uuid-string",
  "stream_url": "/api/v1/events/session-id",
  "status": "streaming"
}
```

**后续步骤：** 创建聊天后，调用 `POST /api/v1/execute/{session_id}` 启动 agent。

---

#### 选择或恢复已有 Root 的模式

`GET /api/v1/sessions/{session_id}` 会为 Root 返回以下仅在详情接口中出现的字段：

```json
{
  "session": {
    "root_orchestration_only": false,
    "root_mode_transition_epoch": 0,
    "root_mode_birth_token": "opaque-64-character-hex-token"
  }
}
```

要更改已有的 Root，请生成一个规范的小写 UUID，并把操作 ID 组成 `<expected_epoch>:<uuid>` 的形式，例如 `0:550e8400-e29b-41d4-a716-446655440000`。将详情中的 birth token、epoch 和期望的模式发送到：

```http
POST /api/v1/sessions/{session_id}/root-mode-operations/{operation_id}
```

```json
{
  "birth_token": "opaque-64-character-hex-token",
  "expected_epoch": 0,
  "enabled": true
}
```

提交成功的响应为 `200 OK`，并带有 `Cache-Control: no-store`：

```json
{
  "status": "committed",
  "operation_id": "0:550e8400-e29b-41d4-a716-446655440000",
  "expected_epoch": 0,
  "resulting_epoch": 1,
  "enabled_at_completion": true,
  "root_tool_authority_revision": 1
}
```

如果响应丢失或超时，请保留**同一个**操作 ID 和请求体，并调用 `POST /api/v1/sessions/{session_id}/root-mode-operations/{operation_id}/recover`。如果选择操作先完成，恢复会返回已提交的终态回执。如果恢复先到达持久化写入端，它会记录一条终态 `fenced` 回执，阻止迟到的选择更改模式。在该栅栏之后重放普通的选择操作会返回 `409 root_mode_operation_fenced`。如果在该回执被淘汰之后，后续操作已经推进了 epoch，恢复会返回 `200`，其中包含 `status: "fenced_by_successor"`、经过校验的 `operation_id` 与 `expected_epoch`、`current_epoch`、`current_enabled` 以及 `root_tool_authority_revision`；旧的选择操作依旧无法提交。

服务器会保留最近八条终态回执，用于跨进程重启的精确重试。更早的回执会被持久化的后继 epoch 取代；其原始操作 ID 无法重新绑定到另一个 epoch。复用已保留的操作 ID 但 `enabled` 值不同，会返回 `409 root_mode_operation_conflict`。epoch 过期或 Root 的 birth 发生变化时返回 `412`；已选择的 Skill 或 Workflow，或处于激活状态的旧版 PlanMode，会产生持久的 `rejected_incompatible` 终态结果（选择时返回 `409`，恢复时返回 `200`）。不支持该操作的存储后端返回 `503`，且不做任何模式更改。其他存储或证明错误返回 `503 root_mode_outcome_unconfirmed`：提交可能已经发生，因此请保留操作 ID 并执行恢复。默认 Supervisor 在刷新下一个 provider 目录时会使用其现有的严格管理证明。

没有任何终态模式操作的 Root 仍可凭其旧版 v1 权限证明读取。第一次终态操作会写入 v2 证明，因此旧后端中仅支持 v1 的写入端会安全失败（fail closed），而不是丢弃自己的 epoch/历史。请使用当前版本的后端执行模式操作；此次升级后，旧进程无法继续为该 Root 提供服务。不支持降级权限证明。

一次成功的模式响应与其后的普通聊天是两个独立的操作。在这两步之间，其他客户端可能更改模式；如果该聊天使用的模式很重要，客户端应检查会话详情。正在运行的工具批次会保留其获准的目录快照，而后续的工具边界会采用持久化的 Root 权限。持有语义不明的合并式 `POST /chat` 标记的旧客户端无法通过这个新操作清除它：旧的 POST 没有操作 ID，因此其终态结果无法在此处得到证明。

---

### Agent 执行

#### 执行 Agent

```http
POST /api/v1/execute/{session_id}
```

为会话启动 agent 执行循环。

`Idempotency-Key` 遵循与聊天相同的可选 10 分钟重放契约。等价的重试会返回原始的状态、响应体和 `run_id`，不会启动另一次运行。规范的嵌套路由 `POST /api/v1/sessions/{session_id}/execute` 共享同一份回执。

**路径参数：**

- `session_id` - 来自 `/api/v1/chat` 的会话标识符

**请求体：**

```json
{
  "model": "claude-sonnet-4-6"
}
```

**响应：** `202 Accepted`

```json
{
  "session_id": "session-id",
  "status": "started",
  "events_url": "/api/v1/events/session-id"
}
```

**注意：** `model` 参数为**必填项**，每次请求都必须提供。

---

### 事件流

#### 订阅事件（推荐）

```http
GET /api/v1/events/{session_id}
```

通过 Server-Sent Events（SSE）订阅实时 agent 事件。

**路径参数：**

- `session_id` - 会话标识符

**响应：** `200 OK`（text/event-stream）

**事件格式：**

```
data: {"type":"token","content":"Hello"}
data: {"type":"tool_start","tool_call_id":"call_1","tool_name":"Read","arguments":{"file_path":"README.md"}}
data: {"type":"tool_complete","tool_call_id":"call_1","result":{ /* ToolResult */ }}
data: {"type":"complete","usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}
```

`type` 判别字段是 `AgentEvent` 变体的 snake_case 形式（该枚举为 `#[serde(tag = "type", rename_all = "snake_case")]`）。

**终止事件：**

- `complete` - Agent 成功完成
- `cancelled` - 运行被用户取消
- `error` - Agent 遇到错误

**示例（JavaScript）：**

```javascript
const eventSource = new EventSource('/api/v1/events/session-123');

eventSource.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('Event:', data);

  if (data.type === 'Complete' || data.type === 'Error') {
    eventSource.close();
  }
};
```

> **其他事件传输方式。** 除了上文按会话提供的 `GET /api/v1/events/{session_id}` 事件流之外，还有一个**全账号范围、可恢复**的变更流 `GET /api/v1/stream`（SSE），它把所有会话的事件多路复用在一条流中——可通过 `?since=<seq>` 或 `Last-Event-ID` 请求头续传。此外还有 `/v2/stream` 上的**实时 WebSocket** 传输通道（按设备 token 认证），它是 web/桌面客户端使用的主要传输通道；SSE 事件流仍保留给简单/curl 客户端使用。

---

### 会话管理

#### 复制会话

```http
POST /api/v1/sessions/{session_id}/copy
```

基于现有会话创建一个独立的根会话。副本会获得一个新的 id，并保留完整的对话记录、持久化配置、权限模式、Project 分配、Workspace 分配以及附件。附件 URL 会被重写为复制后的会话 id。持久化的工作流选择/激活快照会被保留，以便重建对话；而工作流运行 id、生命周期 outbox/缓存数据，以及其他临时性的执行、待处理审批/提问、子会话身份、调度、放置与运行状态数据不会被复制。复制子会话得到的始终是一个没有父级链的独立根会话。

该操作是失败原子的：存储或附件复制失败不会留下目标会话或索引项。并发请求会按设计创建各自不同的副本。

**响应：** `201 Created` 并附带 `{ "session": SessionSummary }`；源会话不存在时返回 `404 Not Found`；副本无法提交时返回 `500 Internal Server Error`。

---

#### 删除会话

```http
DELETE /api/v1/sessions/{session_id}
```

删除会话，并取消任何正在运行的执行。

**路径参数：**

- `session_id` - 会话标识符

**响应：** `200 OK`（无响应体）或 `404 Not Found`

**副作用：**

- 会话从存储中移除
- 会话从内存中移除
- 正在运行的执行被取消

---

#### 获取会话历史

```http
GET /api/v1/sessions/{session_id}/history
```

获取会话的消息历史。

**路径参数：**

- `session_id` - 会话标识符

**响应：** `200 OK`

```json
{
  "session_id": "session-id",
  "messages": []
}
```

**注意：** 目前返回空的 messages 数组。完整实现已在计划中。

---

### 执行控制

#### 停止 Agent 执行

```http
POST /api/v1/stop/{session_id}
```

取消正在运行的 agent 执行。

**路径参数：**

- `session_id` - 会话标识符

**响应：** `200 OK`

```json
{
  "success": true,
  "message": "Agent execution stopped"
}
```

**行为：**

- 完成当前的 LLM 请求
- 取消待执行的工具调用
- 保存会话状态
- 将状态更新为 `Cancelled`

---

### 交互式提问

#### 获取待处理问题

```http
GET /api/v1/sessions/{session_id}/question
```

检查 agent 是否正在等待用户输入。

**路径参数：**

- `session_id` - 会话标识符

**响应（有待处理问题）：** `200 OK`

```json
{
  "has_pending_question": true,
  "question": "Which language should I use?",
  "options": ["TypeScript", "JavaScript", "Python"],
  "allow_custom": false,
  "tool_call_id": "call_123"
}
```

**响应（无问题）：** `200 OK`

```json
{
  "has_pending_question": false
}
```

---

#### 提交用户回复

```http
POST /api/v1/sessions/{session_id}/respond
```

针对待处理问题提交回复；这些问题可能来自可暂停的自定义工具、权限门控或兼容的持久化会话。

**路径参数：**

- `session_id` - 会话标识符

**请求体：**

```json
{
  "response": "TypeScript"
}
```

**响应：** `200 OK`

```json
{
  "success": true,
  "message": "Response recorded. Agent loop will continue.",
  "response": "TypeScript"
}
```

**校验：** 如果 `allow_custom` 为 false，回复必须与提供的选项之一匹配。

---

### 工具执行

#### 直接执行工具

```http
POST /api/v1/tools/execute
```

在不运行完整 agent 循环的情况下直接执行一个内置工具。

**请求体：**

```json
{
  "tool_name": "read_file",
  "parameters": [
    {"name": "path", "value": "/path/to/file"}
  ]
}
```

**响应：** `200 OK`

```json
{
  "result": "{\"tool_name\":\"read_file\",\"result\":\"file contents\",\"display_preference\":\"Default\"}"
}
```

**可用工具：**

- `read_file` - 读取文件内容
- `write_file` - 写入文件内容
- `execute_command` - 执行 shell 命令
- `list_directory` - 列出目录内容
- `file_exists` - 检查文件是否存在
- `get_file_info` - 获取文件元数据
- `git_status` - 获取 git 仓库状态
- `git_diff` - 获取 git diff
- 以及更多……

---

### 健康检查

#### 健康检查端点

```http
GET /health
```

面向负载均衡器和监控系统的简单健康检查。

**响应：** `200 OK`（纯文本 “OK”）

**用途：**

- 负载均衡器健康探测
- 监控系统
- Kubernetes 存活/就绪探针

---

## 典型工作流

### 1. 创建聊天会话

```bash
curl -X POST http://localhost:9562/api/v1/chat \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Help me write a Rust function",
    "model": "claude-sonnet-4-6"
  }'
```

响应中包含 `session_id` 和 `stream_url`。

### 2. 启动 Agent 执行

```bash
curl -X POST http://localhost:9562/api/v1/execute/{session_id} \
  -H "Content-Type: application/json" \
  -d '{"model": "claude-sonnet-4-6"}'
```

响应中包含 `events_url`。

### 3. 订阅事件

```javascript
const eventSource = new EventSource('/api/v1/events/{session_id}');
eventSource.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log(data);
  if (data.type === 'Complete' || data.type === 'Error') {
    eventSource.close();
  }
};
```

### 4. 处理交互式提问（可选）

如果 agent 提出了问题：

```bash
# 检查待处理问题
curl http://localhost:9562/api/v1/sessions/{session_id}/question

# 提交回复
curl -X POST http://localhost:9562/api/v1/sessions/{session_id}/respond \
  -H "Content-Type: application/json" \
  -d '{"response": "Use async/await"}'
```

### 5. 停止或删除（可选）

```bash
# 停止执行
curl -X POST http://localhost:9562/api/v1/stop/{session_id}

# 删除会话
curl -X DELETE http://localhost:9562/api/v1/sessions/{session_id}
```

---

## 错误处理

所有端点都返回格式一致的错误响应：

```json
{
  "error": "Error message",
  "session_id": "session-id"  // 如适用
}
```

**常见 HTTP 状态码：**

- `200 OK` - 成功
- `201 Created` - 资源已创建
- `202 Accepted` - 请求已接受，正在处理
- `400 Bad Request` - 请求无效或缺少参数
- `404 Not Found` - 资源不存在
- `500 Internal Server Error` - 服务器错误

---

## 事件类型

### AgentEvent 类型

`type` 字段是 `AgentEvent` 变体的 snake_case 名称（`crates/core/bamboo-agent-core/src/agent/events.rs`）。最常见的流式变体：

| `type` | 说明 | 字段 |
|--------|-------------|--------|
| `token` | 助手文本 token | `content` |
| `reasoning_token` | 推理/思考 token | `content` |
| `tool_token` | 运行中工具的实时输出 | `tool_call_id`、`content` |
| `tool_start` | 工具执行开始 | `tool_call_id`、`tool_name`、`arguments` |
| `tool_complete` | 工具成功完成 | `tool_call_id`、`result` |
| `tool_error` | 工具失败 | `tool_call_id`、`error` |
| `token_budget_updated` | token 用量/预算更新 | `usage` |
| `complete` | 执行结束 | `usage` |
| `cancelled` | 运行被用户取消 | `message` |
| `error` | 执行失败 | `message` |

该枚举还携带会话、任务、计划以及子代理生命周期相关的变体（例如 `tool_lifecycle`、`need_clarification`、`task_list_updated`、`sub_agent_started`、`plan_mode_entered`、`message_appended`）；完整列表和确切的字段结构请查阅 `events.rs`。

---

## 配置

Bamboo 可以通过命令行参数或环境变量进行配置：

```bash
bamboo serve --port 9562 --data-dir ~/.local/share/bamboo
```

**环境变量：**

- `BAMBOO_PORT` - 服务器端口（默认：9562）
- `BAMBOO_DATA_DIR` - 数据目录
- `BAMBOO_BIND` - 绑定地址（默认：127.0.0.1）
- `BAMBOO_WORKSPACE_ROOT` - 没有显式路径的会话 workspace 的根目录（默认：
  `<data-dir>/workspaces`）。未配置/未显式指定 workspace 的会话会得到
  `<workspace-root>/<session-id>`，而不是服务器进程的工作目录。
- `BAMBOO_WORKSPACE_CONFINE` - 设为 `1`/`true` 时，要求每个显式 workspace
  路径都先规范化再限制在 `BAMBOO_WORKSPACE_ROOT` 之下（通过 `..`、符号链接
  或指向别处的绝对路径逃逸的，会被重新安置到根目录之下，而不是按原样接受）。
  本地单用户场景默认关闭，因为该场景必须继续允许 bamboo 指向磁盘上任意位置
  的现有项目目录；当显式设置 `BAMBOO_WORKSPACE_ROOT` 时会隐式启用。面向
  希望“一个文件夹 = 一个租户的全部状态”的编排式/多租户部署。

### 运行时配置补丁（`POST /v1/bamboo/config`）

运行中的 `config.json`（providers、subagents、notifications、MCP 服务器、bamboo-connect 平台……）通过局部 JSON PATCH 更新——只需发送想要修改的字段。两条规则：

- **省略的键保持现有值不变。** 这是无条件的向后兼容：补丁中没有提到的字段永远不会被改动。
- **显式的 JSON `null` 会删除该字段**，按值逐个选用（[RFC 7386](https://www.rfc-editor.org/rfc/rfc7386) JSON Merge Patch 语义）。“删除”的含义取决于该位置的内容：
  - 可选字段（例如 `subagents.claude_code_binary`）→ 清除为未设置。
  - 整棵对象子树（例如 `notifications: null`）→ 重置为默认值。
  - 动态映射中的单个条目（例如 `provider_instances: {"<id>": null}`、`mcpServers: {"<name>": null}`）→ 只删除该条目，其余条目不受影响。
  - 整个数组（例如 `connect.platforms: null`）→ 清空。数组元素*内部*的 `null` 从不作为删除标记——数组总是整体替换，而不是逐元素合并。

  敏感字段（`api_key`、`token`、`device_key`、`app_secret`）将 `null` 视为显式清除，等价于发送 `""`——二者都不同于掩码占位符（`****...****`，表示“保留现有密钥”）。

  通过选择把哪一层置为 null 来控制影响范围：将单个叶子置为 null（`providers.openai.api_key: null`）只清除该字段；将外层对象置为 null（`providers.openai: null`）会清空整个 provider 的配置。

完整的语义表，以及相对掩码占位符解析的优先级规则，请参阅 `bamboo_config::patch::deep_merge_json` 的文档注释。

---

## 架构

Bamboo 采用基于会话的架构和统一的服务器实现：

1. **Session**：包含对话历史和状态
2. **Agent 循环**：处理消息并执行工具
3. **LLM Provider**：与 AI 模型 API 通信（OpenAI、Anthropic、Gemini、Copilot）
4. **工具执行器**：运行内置工具（read、write、execute 等）
5. **事件广播器**：通过 Server-Sent Events 流式传输实时事件
6. **统一服务器**：单一 HTTP 服务器，采用显式路由（约 120 条路由）

### 服务器架构

- **`bamboo-server` crate**：采用显式路由的统一 HTTP 服务器
- **显式路由**：所有路由都注册在 `crates/bamboo-server/src/routes/` 中
- **直接访问 provider**：不对自身发起 HTTP 回调（消除代理模式）
- **处理器组织**（`crates/bamboo-server/src/handlers/`）：
  - 核心 agent 处理器位于 `handlers/agent/`（chat、execute、events、stop、history、respond 等）
  - provider 处理器位于 `handlers/`（openai/、anthropic/、gemini/、copilot_auth/、agent_api.rs）
  - 功能处理器位于 `handlers/`（settings/、tools/、workspace/、skill/、command/）

---

## 许可证

MIT 许可证

---

## 支持

- GitHub Issues：https://github.com/bigduu/Bamboo-agent/issues
- 文档：https://docs.rs/bamboo-agent
