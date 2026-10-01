# Bamboo crate 架构

分层的 Cargo workspace。依赖只**向下**指向——没有环。HTTP 服务器是薄薄的最上层；agent 循环与用例位于 `bamboo-engine`；port 通过依赖倒置连接通用工具与服务器的 `AppState`。

## 1. crate 依赖分层

```mermaid
flowchart TD
    subgraph APP["Application / HTTP"]
        SERVER["bamboo-server<br/>handlers · routes · AppState<br/>adapters · schedule_app"]
    end

    subgraph TOOLS["Generic tools (subsystem-independent)"]
        STOOLS["bamboo-server-tools<br/>memory · session_inspector · skill_runtime<br/>compact · overlay · sub_agent · ToolSurfaceFactory"]
    end

    subgraph ENGINE["Engine"]
        ENG["bamboo-engine<br/>runtime (agent loop) · session_app (use-cases)<br/>ports: ChildSessionPort · SubagentResolutionPort"]
    end

    subgraph CAP["Capability crates"]
        MEM["bamboo-memory"]
        TLS["bamboo-tools"]
        HOOKS["bamboo-hooks<br/>registry · matching · handler runtimes"]
        SK["bamboo-skills"]
        MCP["bamboo-mcp"]
        PERM["bamboo-permission"]
        MET["bamboo-metrics"]
        CMP["bamboo-compression"]
    end

    subgraph CORE["Core / Infrastructure"]
        AC["bamboo-agent-core<br/>Tool · ToolExecutor · ToolExecutionContext<br/>Session · Storage · AgentEvent"]
        INFRA["bamboo-infrastructure<br/>Config · SessionStoreV2 · ProviderRegistry"]
        LLM["bamboo-llm"]
        CFG["bamboo-config"]
    end

    subgraph FND["Foundation"]
        DOM["bamboo-domain<br/>core types · schedule model · subagent registry"]
    end

    SERVER --> STOOLS
    SERVER --> ENG
    STOOLS --> ENG
    STOOLS --> AC
    ENG --> MEM
    ENG --> TLS
    ENG --> HOOKS
    ENG --> SK
    ENG --> MCP
    ENG --> AC
    ENG --> INFRA
    MEM --> AC
    TLS --> AC
    HOOKS --> AC
    HOOKS --> INFRA
    PERM --> INFRA
    AC --> DOM
    INFRA --> DOM
    INFRA --> LLM
    LLM --> CFG
    LLM --> DOM

    classDef new fill:#1f6f43,stroke:#39d98a,color:#fff;
    classDef port fill:#1e4d8c,stroke:#5aa9ff,color:#fff;
    class STOOLS new;
    class ENG port;
```

- **`bamboo-server-tools`**（绿色）——在 L1 中抽出。存放属于通用 agent 能力的工具。只依赖下层的 crate，绝不依赖 `bamboo-server`/`AppState`。
- **`bamboo-engine`**（蓝色）——拥有 agent 循环**以及**让通用工具在不依赖服务器的前提下访问服务器运行时状态的 port trait。
- **`bamboo-hooks`**——负责生命周期注册、匹配器求值、确定性分发、命令执行以及外部脚本运行时的选择。引擎拥有生命周期缝合点，并应用其返回的控制或上下文效果。

## 2. port 模式（依赖倒置）

展示一个通用工具（`SubAgentTool`）如何在不依赖 `bamboo-server` 的情况下访问绑定于 `AppState` 的运行时状态，以及调度工具为何留在自己的子系统内。

```mermaid
flowchart LR
    subgraph STOOLS["bamboo-server-tools"]
        SUBAGENT["SubAgentTool<br/>holds Arc&lt;dyn Port&gt;"]
        GENERIC["memory · skills · compact<br/>overlay · session_inspector"]
    end

    subgraph ENGINE["bamboo-engine"]
        PORTS["ChildSessionPort<br/>SubagentResolutionPort<br/>(trait definitions)"]
    end

    subgraph SERVER["bamboo-server (composition root)"]
        AS["AppState<br/>sessions · runners · stores<br/>event senders · config"]
        ADAPTER["ChildSessionAdapter<br/>impl both ports"]
        FACTORY["ToolSurfaceFactory<br/>bundles tools into executors"]
        subgraph SA["schedule_app (self-contained vertical slice)"]
            CORE2["trigger · store · manager · session_factory"]
            SCHEDTOOL["scheduler_tool<br/>facade over this subsystem"]
        end
    end

    SUBAGENT -- depends on --> PORTS
    ADAPTER -- implements --> PORTS
    AS -- builds + holds --> ADAPTER
    ADAPTER -. injected as Arc&lt;dyn Port&gt; .-> SUBAGENT
    FACTORY --> SUBAGENT
    FACTORY --> GENERIC
    FACTORY --> SCHEDTOOL
    SCHEDTOOL --> CORE2

    classDef port fill:#1e4d8c,stroke:#5aa9ff,color:#fff;
    classDef glue fill:#6b3fa0,stroke:#b794f6,color:#fff;
    class PORTS port;
    class ADAPTER,FACTORY glue;
```

**解读：**

- 依赖边 `SubAgentTool → AppState` 已消失。现在指向 `SubAgentTool → Port`（engine）和 `Adapter → Port`（server）。箭头被*倒置*了：工具与服务器都依赖引擎拥有的 trait。
- `ChildSessionAdapter`（紫色）是唯一绑定 `AppState` 运行时状态的地方。它实现了 `ChildSessionPort`（session 生命周期：加载/保存/运行/取消 + 等待父级 + 活跃子级）和 `SubagentResolutionPort`（subagent_type → 模型/元数据/prompt）。
- `ToolSurfaceFactory` 是组合根，负责组装 agent 运行时使用的各表面工具执行器（Base / Child / WithTask / Root）。
- **`scheduler_tool`** 留在 `schedule_app` *内部*：它是该子系统的门面（其参数内嵌调度领域类型；返回调度 DTO），而不是一个与子系统无关的能力。让它与子系统同址保留了内聚性，也使 `schedule_app` 成为一个随时可以独立成 crate 的干净切片。

## 3. L1 改变了什么

| | 之前 | 之后 |
|---|---|---|
| 通用工具（memory、skills、compact、overlay、session_inspector） | `bamboo-server::server_tools` | `bamboo-server-tools` crate |
| `SubAgentTool` | 持有具体的 `Arc<ChildSessionAdapter>` | 持有 `Arc<dyn ChildSessionPort>` + `Arc<dyn SubagentResolutionPort>`；位于 `bamboo-server-tools` |
| port | 无 | `bamboo-engine` 中的 `ChildSessionPort`（已扩展）+ `SubagentResolutionPort` |
| 调度工具 | `bamboo-server::tools::schedule_tasks` | `bamboo-server::schedule_app::scheduler_tool`（连同其子系统） |
| `bamboo-server/src` | 49,465 LOC | 45,488 LOC |

这一步为后续迁移铺路：**L2** 把 handler 编排下沉到 `engine::session_app` 用例；**L5** 把整个 `schedule_app` 切片（包括其工具）上提为独立的 `bamboo-schedule` crate。
