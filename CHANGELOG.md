# 更新日志

本文件记录本项目的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/en/1.0.0/)。
历史上的 `0.x` 版本遵循[语义化版本](https://semver.org/spec/v2.0.0.html)；
此后项目发布**按日期编号的 nightly 版本**（见下文）。

## [Unreleased] — nightly（按日期编号）

自 `0.3.0` 起，项目发布由发布列车裁剪的**按日期编号的 nightly 版本**（如 `2026.7.x`），
而非 SemVer 点版本。nightly 之间的变更记录在 git 历史与已合并 PR（事实源）中；
下面的 SemVer 小节仅为历史上的 `0.x` 版本保留。

- 微信引用回复原文找回（协议层的最优补救）：iLink 只给被引消息的
  msg_id/时间戳指针、无按 id 取消息的 API，但被引的是同一场对话的早前
  消息——就在会话历史里。引用指针（quote_ref）随 reply_ctx 进入桥接层，
  按 create_time_ms 在会话历史做最近邻匹配（±2 分钟，容忍微信服务端与
  本地落盘时间偏差），命中即把被引原文（截断 200 字）附为 `[引用内容]`
  行；未命中（跨会话引用/已被上下文压缩/撤回）保持"请用户复述"提示。
- 微信引用回复一等提示（实测线格式落地）：取证确认引用元数据在条目级
  `ref_msg.message_item`，但只含被引消息的 msg_id/时间戳指针，**无原文**
  （iLink 也不提供按 msg_id 取消息的 API）；被引媒体则作为普通媒体条目
  随消息照常送达。现在检测到 ref_msg 即给模型一行 `[引用回复]` 可见
  提示（无原文时让用户复述、不要假装读过），被引媒体路径照常附带；
  基础提示词的引用教学同步改为真实形态。引用原文不可达是渠道限制，
  非客户端可解。
- 微信入站静默丢弃路径留证：非 1 的 message_type、解析不出内容的用户
  消息、缺 from_user_id/context_token 的消息此前一行日志都没有——引用
  消息等未公开格式正是落进这些路径才"机器人看不见"（实测引用消息既
  未触发未知类型占位、也未进入会话）。现在此类丢弃计数并以 WARN 记录
  原始 getupdates 载荷（前 2000 字符），getupdates 解析错误也附 body
  头部，供协议跟进定位真实线格式。另实测定性：引用**媒体**消息时被引
  内容以普通文件条目照常送达；引用**文字**消息时只送达用户手打文本——
  为捕获可能藏于 text_item/条目/信封级未映射字段中的引用元数据，
  IlinkMessage/IlinkItem/IlinkTextItem 全部 flatten 保留未知字段，非空
  即 WARN 留证原始 JSON（普通消息不触发）。
- 标记面审计：出站 5 个/入站 7 个标记功能各不相同，无重复；标记 vs
  斜杠命令 vs 自然短语是同一功能的多入口（模型输出/用户输入各有其
  actor），属设计而非冗余。修复一处会出问题的作用域缺口：基础提示词
  的切换指令未限定聊天渠道——Web 会话里用户要求切换时模型可能输出
  裸 [SET_*] 标记（Web 不解析、用户会看到原文），现明确"标记仅限聊天
  渠道，Web 用界面控件切换，绝不输出标记"（SEND_FILE/SEND_VOICE 同步
  补注）。代码去重：marker_line 新增通用 extract_marker_lines，
  wechat 的 SEND_FILE/SEND_VOICE 提取函数收敛为薄封装。
- 微信审批模式补齐第三档（与网页 PATCH 对齐）：会话权限实为
  Default/Auto/Bypass 三档，聊天渠道的 [SET_PERMISSION:] 与自然短语此前
  只接 auto/default，bypass 在聊天渠道永远够不着。现标记接受三档
  （default/ask/confirm/默认/恢复确认、auto/always/自动、
  bypass/skip/跳过/全部跳过），提示词同步教**用法**（按用户意图选档：
  嫌每次确认→auto、要恢复询问→default、明确要求跳过全部权限→先提醒
  风险确认后再 bypass、绝不主动建议 bypass）；"接下来都允许"/"恢复确
  认"短语仍映射 Auto/Default 不变。
- 修复标记切换被运行收尾落盘冲掉的竞态（实测：模型正确输出
  [SET_MODEL:]、网关匹配成功且写入返回 Ok，但 model_ref 始终为空）：
  运行中立即写会话覆盖，会在约 1 秒后被引擎收尾的最终落盘用运行开始
  时的旧会话快照覆盖。标记路径改为**排队**（chat_state），下一条消息
  进入 process_one 时（运行早已结束、无竞态）真正落盘并广播——语义
  恰好就是"下一条消息生效"；/new 旋转时丢弃排队（全新开始不带旧会话
  的切换）；/model、/think 命令成功后清除对应排队（"最后请求赢"——
  命令在运行外直写、无此竞态，保持直写并取代排队的标记）。
- 微信切模型支持动态供应商（网页前端同款能力）：DeepSeek 等只配
  base_url+凭据的动态实例（模型来自供应商实时清单）此前不在网关的
  静态候选里——网页能切、微信切不了，模型被问到切换还会钻进源码自证。
  现在 /model 与 [SET_MODEL:] 静态未命中时自动实时拉取供应商模型清单
  兜底匹配（label 命中的实例优先；"DeepSeek V4 PRO" ≡ deepseek-v4-pro，
  大小写/空格/连字符/下划线/点归一化），未匹配的纠错文案附该供应商
  当前可用清单；运行时覆盖校验放宽为"静态命中或实例仍在启用"，动态
  模型的会话覆盖不再每轮被静默打回默认。提示词补行动指引：不确定
  模型名时直接把用户口述原样放进标记让网关校验，不要自行查配置或
  源码。
- 前端版本号对齐发布 tag：package.json 0.0.0 → 0.0.11，设置页"运行版本"
  与嵌入包清单随之显示 v0.0.11（本批发布 tag）。
- 修复引用+裸媒体组合绕过"先问用途"：引用图片/文件发送时消息带
  [引用回复] 提示行，而 media_only 判定要求所有行都是媒体行 → 判定
  失败直接起模型分析。现提示行不计入用户文字，引用裸媒体同样先问用途。
- 机器人能力自认知补全（中文）：基础系统提示词与前端默认提示词原来
  并不包含渠道控制标记与新入站格式的说明（只在微信会话首条消息的
  《渠道能力提示》里），模型被问到自身能力或见到 [引用 …]/
  [未支持的消息类型 N] 行时一无所知。现在自认知块覆盖：入站媒体/
  引用/未支持占位标记的含义、出站 [SEND_FILE:]/[SEND_VOICE:] 投递、
  [SET_MODEL:]/[SET_THINK:]/[SET_PERMISSION:] 会话切换、半角书写硬性
  要求；语音链路（[语音] 为微信自带 ASR 转写、[音频转写] 与语音合成
  走管理员配置的语音服务默认硅基流动 SenseVoice/CosyVoice、不可用时
  自动降级文字）；聊天渠道命令面（/new、/stop、/status、/models、
  /model、/think、/sessions、/switch、/del、"接下来都允许"/"恢复确认"，
  由网关处理不会到达模型，但被问到时要答得上来；微信渠道仅私聊）；
  长会话压缩恢复点 [post-compaction-recovery] 的含义与"细节可能被
  摘要，缺失时确认而非断言"。渠道能力前言同步附命令清单。同步修复
  两处过期测试：Rust default_base_prompt 断言仍为英文短语（v0.0.9
  中文化起已红）、前端 defaultSystemPrompts 断言仍为 Bodhi/英文
  （改名起已红），均更新为中文等价表述并补标记/语音/命令回归。
- 模型/智能度/审批模式切换实时推送到网页端：新增可重放会话事件
  `SessionExecutionProfileUpdated`（与标题/置顶同类的元数据事件，走
  cache→账本→广播的统一顺序发布），微信 /model、/think、SET 标记
  （含 [SET_PERMISSION]）、"接下来都允许"与网页 PATCH 落盘后都会广播
  （no-op 切换不广播）。前端会话流与账户流两路处理：正停在该会话页时
  输入框的模型/智能度选择器与审批模式指示即刻更新，其他标签页/后台
  会话经账户流同步，无需刷新。replayable 模块新增 parts 版发布函数供
  connect 桥接使用（无 AgentSessionContext 的写入方），顺序不变式仍
  集中一处。
- 微信侧切换模型/智能度对网页前端可见（反之亦然）：connect 桥接的会话
  覆盖此前写私有 metadata 键（connect.model_override / connect.think_
  override），网页 PATCH 与前端选择器读的是会话类型化字段（model_ref /
  reasoning_effort），两套真相互不可见——微信 /model、/think 或
  [SET_MODEL]/[SET_THINK] 标记切换后网页会话页不更新。现统一到类型化
  字段（与网页 PATCH 同源、同 bump metadata_version 语义），读取时对
  旧 metadata 键兜底兼容，写入时顺带清除旧键；网页切换同样对微信侧
  下一条消息生效。
- 微信纯媒体入站（裸文件/图片/视频，无任何文字说明）不再直接起模型分析
  ——裸文件意图无从猜测，直接读取只会白烧 token。适配器在 reply_ctx 打
  `media_only` 标（附文件名），bridge 确定性回一句"已收到 X。要我怎么处
  理？"，零 LLM 调用；文件路径行已随用户消息进会话，用户文字回复后带着
  上下文真正处理。带文字的消息（"看看这张图"）、带转写的语音/音频、下载
  失败的媒体（需模型向用户解释）均不走此短路，行为不变。
- 微信网关入站引用/未支持消息类型不再静默丢弃：公开 iLink 协议只枚举
  item type 1–5（文本/图/语音/文件/视频），引用回复等新类型此前落入
  `_ => {}` 整条吞掉（表现为"机器人看不到引用消息"）。现在未知类型
  生成 `[未支持的消息类型 N，内容暂无法读取]` 占位行照常转发（模型可
  据此告知用户），并 WARN 记录原始线格式载荷（item 反序列化保留未识别
  字段）供协议跟进；落在 text 里的 appmsg(type=57) refermsg XML 渲染为
  `[引用 <名字>: <被引摘要>] <用户输入>` 可读行（残缺 XML 退回原文）。
- 审计修复同类"本地化输入漂移"缺口：编号审批回复里的全角数字（"１"）
  此前 `parse::<usize>` 直接解析失败、选不中选项；现与 `/switch` 序号
  参数共用同一全角数字归一化（`approvals::normalize_fullwidth_digits`）。
  其余同类面（入站命令全角斜杠/空格、/think 中文档位别名、审批中英文
  关键词、持续授权短语 contains 匹配）已有归一化与测试覆盖，确认无恙。
- 修复微信网关出站媒体投递在全中文提示词下整类失效：模型中文语境里常把
  `[SEND_FILE: …]`/`[SEND_VOICE: …]`/`[SET_MODEL: …]` 等控制标记写成全角
  标点（［］【】：）、漏掉冒号后空格或闭括号后跟句读，网关的严格半角
  前缀匹配识别不到，标记行被当普通文本发给用户、文件/语音/命令全部静默
  失效。新增 `connect/marker_line` 统一的行级宽容解析（全角括号/冒号/
  空格、缺空格、大小写、尾句读均认，标记内容原样保留不改写），微信附件/
  语音出口、语音适念门槛与 bridge 的 SET_* 出口共用；渠道能力前言同步
  增加"标记必须半角、独占一行、勿入代码块"的硬性指令，从源头防漂移。
- 修复 Web 前端权限确认 409（Permission interactions require the typed
  decision endpoint）：实时 need_clarification SSE 事件不携带权限契约（引擎
  事件无权限字段），前端误走旧版 respond 提交被服务端拒绝。现事件到达时
  若无 typed 权限信号，自动经 GET pending-question 补水权限契约并改走
  /permission-decisions typed 端点（会话在事件发射前已持久化，无竞态）；
  微信网关审批路径不受影响（走桥接自有流）。
- 修复 Web 前端 typed 权限提交 400（missing field `request_generation`）：
  前端契约层从未读取/回传服务端签发的 request_generation（防重放代数，
  TUI 客户端早已携带），补水后的提交体缺字段被服务端反序列化拒绝。
  现在 normalizePermissionRequest 透传该字段、提交体必带；补权限契约与
  补水链路两处测试断言，嵌入前端包随之重建。
- 默认系统提示词全面中文化 + 能力自认知：服务端 DEFAULT_BASE_PROMPT 与前端默认提示词
  改为中文（nana 身份、运行时能力说明：持久记忆/Dream 整理/隐私脱敏/渠道，
  [REDACTED] 处理约定），并要求内部推理（reasoning）也使用简体中文；
  环境变量注入说明、默认提示词预设名同步中文化/nana 品牌。派生语言包
  （fr/ja/hi/zh-TW）品牌统一为 nana。
- 文档对齐当前版本：docs/config-reference.md 新增 dream_privacy_mode /
  dream_privacy_off_keep_durable_gate 字段说明与微信平台 connect 文档
  （dream 计划 off/daily/idle、auto_project、typing_indicator）；README
  记忆系统能力表补充隐私策略；嵌入前端包随之重建。
- 梦境脱敏机制改造（`dream_privacy_mode`）：`memory.json` 新增
  `dream_privacy_mode: "strict" | "redact" | "off"`（默认 strict，现状行为）
  与 `dream_privacy_off_keep_durable_gate`（仅 off 模式生效，默认 true）。
  strict 保持"命中敏感即整体剥离"不变；redact 仅把命中片段替换为
  `[REDACTED]`（赋值类只打码值、整行兜底、无法定位时整字段兜底，输出保证
  不含敏感值），持久化候选门禁在 strict/redact 下恒启用；off 不做提取源
  剥离、门禁可独立关断。设置页（系统 → 配置 → Memory 与 Auto Dream）新增
  下拉与开关，走 memory 分节 API 热加载（每次 Dream 运行刷新，无需重启）；
  非法配置值读取时回退 strict 并 WARN，写入时被 `validate_memory` 拒绝。
  `bamboo.auto_dream` 的 run_start 事件新增 `dream_privacy_mode` 字段。
- 项目品牌更名为 nana：前端展示名（页面标题、欢迎页、助手名、
  桌面通知文案、各设置页描述）与 README 更名；代码层标识（crate 名
  bamboo-*、二进制名、`BAMBOO_*` 环境变量、`/data` 目录、日志 target、
  bodhi provider 名称）保持不变以兼容既有部署。
- 重建嵌入前端包 `lotus-frontend.zip`：补齐 v0.0.7 的微信梦境计划控件
  （关闭/每天定时/闲置触发 + 时刻与分钟数）在嵌入包中的缺失，并携带
  dream_privacy_mode 与品牌更名；新增 `scripts/frontend-package-node.cjs`
  供无 7z/zip 的 Windows 工作站打包（与 cjs 版同 hash/清单规则）。
- 手动"仅本会话"梦境整合：POST project-dream/run 新增可选 body
  {"scope":"session"}（默认 project），只把触发会话增量整合进项目记忆、
  只消耗它的提取水位，同项目其他会话留给下一次整项目 Dream；前端会话
  菜单新增"运行梦境（仅本会话）"。
- 微信「正在输入中」指示（iLink sendtyping，picoclaw/openclaw 同款）：
  机器人后台思考期间每 8 秒发一次 typing 状态，发出首条回复即停；
  connect.platforms[type=wechat].typing_indicator 可关（默认开启），设置
  页微信区块有开关。
### Added

- 微信网关会话管理补全：聊天内 `/sessions`、`/switch <序号>`、
  `/del <序号>` 命令（模型切换另有显式 `/switch_model` 别名）与 Web 端（设置 → 连接 → 微信）会话面板（列表 /
  新建 / 切换 / 删除；`GET/POST /api/v1/bamboo/connect/sessions` 等）。
- 项目工作区支持嵌套：创建校验只拒绝与现有项目完全相同的根，归属
  解析按最长匹配选最内层项目；自动项目工作区冲突时在数据目录内
  递增编号（work_wechat、work_wechat_1、…），`auto_project.workspace`
  支持绝对路径显式指定。
- 微信网关首条消息自动创建「微信工作」项目（工作区为数据目录下
  `work_wechat`，不存在则创建）并把网关会话归属该项目；
  `auto_project` 段可调（enabled/name/workspace）。
- 微信网关 Project Dream 自动运行计划：`dream` 段支持 `daily`（每天
  `daily_at` 时刻，默认 03:00）与 `idle`（闲置 `idle_minutes` 分钟后触发，
  默认 30）两种模式，设置页微信区块可配；运行状态持久化防重启重跑。

- 手动 Project Dream 接口异步化：POST /sessions/{id}/project-dream/run
  立即返回 job_id（202），新增 GET .../project-dream/run/{job_id} 轮询
  状态与结果；同项目同时只跑一场，完结结果保留 30 分钟。前端"运行
 梦境"改为轮询展示，不再受 frp 等反向代理 30 秒网关超时影响。
- 聊天命令识别归一化：全角斜杠／、全角空格、全角数字序号均按半角
  识别（中文输入法下命令不再漏进模型）；新增 /switch_think 与
  /switch_model 对称的智能度切换别名。
### Changed

- 将最低支持的 Rust 版本从 1.84 提升到 1.95，以匹配锁定的依赖图，
  并新增显式 MSRV CI 车道与依赖图审计。

## [0.3.0] - 2026-02-26

### Security
- provider API 密钥现在在 `config.json` 中静态加密，且绝不通过 API 返回（仅掩码占位符）。
- MCP SSE 头值和 MCP stdio 环境变量值现在在 `config.json` 中静态加密，且绝不通过 API 返回（仅掩码占位符）。

## [0.2.12] - 2026-02-26

### Changed
- 配置现在完全统一到 `config.json`（单一入口/出口）：关键词掩码、模型映射、MCP 服务器配置和权限都通过统一配置持久化。

### Fixed
- MCP 服务器管理端点现在会把变更持久化到统一配置（运行时与磁盘之间不再漂移）。

## [0.2.11] - 2026-02-26

### Fixed
- Copilot 认证端点现在遵循已配置的 HTTP/HTTPS 代理设置（支持企业网络）。
- 内置 `http_request` 工具现在遵循相同的已配置代理设置（出站网络行为一致）。

## [0.2.10] - 2026-02-26

### Added
- 工具执行流式输出：工具现在可以在运行期间发出增量输出事件（SSE `tool_token`）。
- Claude Code CLI 集成：可选内置 `claude_code` 工具（发现 `claude` 可执行时自动启用），以 `--output-format stream-json` 运行并流式输出。

### Changed
- 流式输出：工具发出的 `token` 事件被当作工具范围的输出（`tool_token`），不再混入助手文本流。
- 模型限制：为 `gpt-4.1` 新增内置上下文窗口条目（默认 128k；如有需要可通过用户配置覆盖）。

### Fixed
- Claude Code CLI：使用 `-p/--print` 加 `--output-format=stream-json` 时始终传 `--verbose`（Claude Code 的要求）。

## [0.2.9] - 2026-02-25

### Added
- Copilot：支持通过 `providers.copilot.model` 选择并持久化 provider 特定的默认模型。
- Copilot：OpenAI 兼容的 `/v1/chat/completions` 现在会把解析后的模型转发到 Copilot 上游请求。

### Changed
- 设置：澄清 `POST /bamboo/settings/provider` 本身就会保存并重载 provider 配置（因此通常无需再单独调用重载）。

## [0.2.8] - 2026-02-25

### Removed (Breaking)
- 移除遗留的 `bamboo_agent::agent::server::state::AppState` 类型/模块；Bamboo 现在只有一个统一的 `AppState`。
  - **迁移**：一律改用 `bamboo_agent::server::app_state::AppState`（或 `bamboo_agent::server::AppState`）。
- 移除遗留的服务器实现/模块：
  - `bamboo_agent::agent::server`（遗留 Actix 服务器）
  - `bamboo_agent::web_service`（代理服务器）
- 移除遗留的 `/api/v1/stream/{session_id}` 端点。请改用 `POST /api/v1/execute/{session_id}` + `GET /api/v1/events/{session_id}`。

### Fixed
- 修复了当 handler 需要错误的 `AppState` 类型时可能导致运行时故障（如 `/anthropic/v1/messages`）的 Actix `Data<T>` 提取器不匹配问题。

## [0.2.6] - 2025-02-25

### Fixed - 关键生产阻塞问题

#### Security
- **安全**：修复 `bamboo config` 命令的 API 密钥泄露——密钥现在默认打码
  - 新增 `--show-secrets` 标志，在需要时显式显示 API 密钥
  - 防止 API 密钥出现在 shell 历史、CI 日志或屏幕共享中

#### 配置系统
- **严重**：修复运行中的服务器不遵守 `--data-dir` 标志的问题
  - 服务器现在会从指定的数据目录正确加载配置
  - `AppState::new()` 和 `reload_config()` 使用 `Config::from_data_dir()`
  - 修复潜在的数据损坏与安全问题

#### 文档
- **高**：修复配置模块的文档漂移
  - 配置文件位置更新为 `${BAMBOO_DATA_DIR}/config.json`（默认 `${HOME}/.bamboo/config.json`；此前错误地显示为 XDG 路径）
  - 移除 TOML 格式引用（实际格式只有 JSON）
  - 修正环境变量名：`BAMBOO_HEADLESS`（此前错误地写为 `BAMBOO_HEADLESS_AUTH`）
  - 移除对 `HTTP_PROXY`/`HTTPS_PROXY` 的提及（实现显式忽略它们）
  - 记录正确的优先级顺序：CLI > 环境变量 > 文件 > 默认值
  - 所有 provider 配置示例从 TOML 转为 JSON

### Changed

#### 默认 Provider
- **变更**：默认 provider 从 "copilot" 回退为 "anthropic"
  - **原因**：Copilot OAuth2 认证在 CI/CD 环境中难以测试与模拟
  - **影响**：新安装将默认使用 Anthropic（需要 API 密钥）
  - **迁移**：想用 Copilot 的用户应在配置中显式设置 `provider: "copilot"`
  - **未来**：待测试基础设施就绪后，Copilot 将在 v0.4.0 重新成为默认

- 配置文档现在准确反映实现行为
- 所有 provider 配置示例统一使用 JSON 格式

### 架构
- 统一配置系统，单一 `Config` 结构体
- 正确的优先级顺序：CLI 参数 > 环境变量 > 配置文件 > 代码默认值
- 服务器配置（端口、绑定、worker 数、static_dir）并入统一 Config

### 已知问题（优先级较低）
- `--workers` CLI 标志已解析但未接入服务器（使用默认 worker 数）
- `--static-dir` CLI 标志已解析但未接入服务器
- 这些问题已记录在案，可在未来版本处理

### 修改的文件
- `src/core/config.rs` - 默认 provider 回退为 "anthropic"
- `src/server/app_state.rs` - 修复配置加载中的 data_dir 使用
- `src/server/handlers/agent_api.rs` - 修复 `get_claude_dir()`，目录缺失时创建
- `src/bin/bamboo.rs` - 新增 `--show-secrets` 标志与密钥打码
- `tests/e2e/copilot_auth.rs` - 针对新默认 provider 更新测试
- `Cargo.toml` - 版本号升至 0.2.6

### 迁移说明
- **默认 provider 变更**：如果你依赖隐式的 "copilot" 默认值，请在配置中显式设置：
  ```json
  {
    "provider": "copilot"
  }
  ```
- **无其他破坏性变更** - 对既有配置 100% 向后兼容
- 用户可选择在 config.json 中添加 `server` 段（省略时使用默认值）
- 环境变量 `BAMBOO_HEADLESS_AUTH` 已弃用，请改用 `BAMBOO_HEADLESS`

### 部署状态
✅ **可用于生产部署**

Codex 评审第 3 轮指出的所有关键生产阻塞问题均已修复。

## [0.2.0] - 2026-02-24

### 🎉 重大重构：统一服务器架构

本版本将 `web_service` 和 `agent::server` 合并为统一的 `server/` 模块，
并以显式路由统一了所有 HTTP handler。

### Added

- **统一服务器模块**（`src/server/`）
  - 单一 `AppState`，直接访问 provider（消除代理模式）
  - 统一的指标基础设施
  - README.md 与 MIGRATION.md 中的完整迁移指南

- **显式路由系统**
  - 所有路由现在使用显式 `web::route()` 注册
  - handler 中不再有 `#[get]`、`#[post]` 宏
  - `src/server/routes.rs` 中的单一事实源（约 120 条路由）

- **统一 handler 术语**
  - 所有 HTTP handler 归并到 `src/server/handlers/` 之下
  - Agent handler：`handlers/agent/`（chat、execute、events 等）
  - Provider handler：`handlers/*.rs`（openai、anthropic、gemini 等）

- **服务器模式**
  - `run()` - 桌面模式（仅 localhost，不限流）
  - `run_with_bind()` - Docker 模式（自定义绑定，限流）
  - `run_with_bind_and_static()` - 带前端服务的生产模式

- **模块组织**
  - 新增 `server::routes` 模块，承载路由配置
  - 新增 `server::server` 模块，承载入口点
  - 新增 `server::config` 模块，承载 CORS/安全头
  - 新增 `server::metrics` 模块，承载统一基础设施

### Changed

- **BREAKING**（保留向后兼容）：
  - 弃用 `agent::server` 模块 → 改用 `server`
  - 弃用 `web_service` 模块 → 改用 `server`
  - 旧导入仍可使用，但会收到弃用警告

- **Handler 结构**：
  - `src/server/handlers/*.rs` → `src/server/handlers/agent/*.rs`（核心 handler）
  - `src/server/controllers/*.rs` → `src/server/handlers/*.rs`（provider handler）
  - 所有 handler 使用一致的术语统一

- **路由注册**：
  - 从：基于宏（`#[get("/path")]`）
  - 到：显式（`.route("/path", web::get().to(handler))`）
  - 全部约 120 条路由现在显式注册

- **状态管理**：
  - 单一统一 `AppState` 取代双重状态
  - 直接访问 provider 取代对自身的 HTTP 回调

- **代码组织**：
  - 消除 24 条重复路由注册（54 → 30，减少 44%）
  - 移除代理模式（`build_agent_state()` 函数）
  - 更清晰的模块结构

### Removed

- 重复路由定义（消除 24 条路由）
- 对自身进行 HTTP 回调的代理模式
- 路由宏，改为显式注册
- 约 430 行冗余代码

### Fixed

- `app_state::tests` 中的异步测试阻塞问题
- 配置保存/加载测试改用临时路径，以兼容 CI
- HTTP 请求测试通过 `BAMBOO_TEST_NETWORK=1` 环境变量选择性启用

### 迁移指南

#### 对库用户

旧写法（已弃用但仍可用）：
```rust
// 注意：该遗留导入路径已在 v0.2.8 中移除。
// use bamboo_agent::agent::server::state::AppState;
use bamboo_agent::web_service::WebService;
use bamboo_agent::agent::server::handlers;
```

新写法（推荐）：
```rust
use bamboo_agent::server::AppState;
use bamboo_agent::server::WebService;
use bamboo_agent::server::handlers;
```

#### 对贡献者

- 所有 HTTP handler 都在 `src/server/handlers/`
- 在 `src/server/routes.rs` 中使用显式路由注册
- 不再使用 `#[get]`、`#[post]` 等宏

### 统计

- **变更文件数**：63
- **新增行数**：+599
- **删除行数**：-1029
- **净变化**：-430 行（更干净的代码库！）
- **测试**：867/867 通过（100%）
- **消除的重复路由**：24（减少 44%）
- **提交**：2-3 天内 8 个主要提交
- 消除代理模式（`build_agent_state`）
- 统一状态管理（单一 AppState）
- 全部 866 个测试更新并通过

### Fixed
- `app_state::tests::test_app_state_creation` 中的异步测试阻塞问题

### 迁移指南

#### v0.2.0 之前
```rust
use bamboo_agent::agent::server::AppState;
use bamboo_agent::web_service::WebService;
```

#### v0.2.0 之后
```rust
use bamboo_agent::server::AppState;
use bamboo_agent::server::WebService;
```

所有旧导入仍可使用，但会收到弃用警告。详情见 MIGRATION.md。

## [0.1.2] - 2026-02-23

### Added
- 首次发布到 crates.io
- 多 LLM provider 支持（OpenAI、Anthropic、Gemini、Copilot）
- 基于 Actix-web 的内置 HTTP 服务器
- 带工具执行的 Agent 循环
- 会话管理
- 工作流系统
- MCP（Model Context Protocol）集成
- 863 个测试通过

[0.2.0]: https://github.com/bigduu/Bamboo-agent/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/bigduu/Bamboo-agent/releases/tag/v0.1.2
