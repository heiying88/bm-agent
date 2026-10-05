# 微信网关语音能力方案：TTS 语音条回复 + 音频文件 ASR（硅基流动）

> 状态：**已实施**（2026-10-02：M0-M3 落地——vendor SILK SDK（cc 编译，无 libclang 依赖）+ 硅基流动 TTS/ASR + 网关化决策管线 + 双开关默认关；实现见 wechat_voice.rs 与 wechat.rs，配置示例见 CONNECT.md §4.3）。
> 语音条只用微信转写、ASR 仅识别音频文件附件）。
> 前置阅读：[`wechat-ilink-adapter-plan.md`](wechat-ilink-adapter-plan.md)（iLink 协议与网关分层）、
> [`wechat-ilink-architecture.md`](wechat-ilink-architecture.md)。

## 0. 目标与分层红线

给微信（iLink）适配器补上语音的双向能力：

| 方向 | 现状 | 目标 |
|---|---|---|
| 出站（机器人 → 用户） | 只能发文本/媒体文件，**语音条未实现**（v1 明确推迟，卡在 SILK 编码） | 网关按确定性规则决定语音/文字（见 §3a）：`reply_mode` 三态（**默认 `off`，TTS 总开关**）+ 内容硬门槛降级；模型可用 `[SEND_VOICE: 文本]` 显式请求语音 |
| 入站（用户 → 机器人） | 语音条只用微信自带转写（`voice_item.text`），音频文件附件只存盘标记路径，模型读不出内容 | **语音条维持现状**（不做 SILK 下载解码）；音频文件（mp3/wav/m4a 等）在 `file_asr` 开启时经 SenseVoice 转写，追加 `[音频转写] 文本` 行（默认关） |

**分层红线（与 SEND_FILE 同一约束）**：

- 全部代码只落在 `bamboo-server` 的 connect 网关层（`src/connect/platforms/` 下），
  **不改 engine / tools / domain / 其他平台适配器**；
- 唯一的跨文件例外是 `bridge.rs` 的渠道能力前言追加一句 `[SEND_VOICE]` 标记说明
  （前言本身就在网关层，与 `[SEND_FILE]` 同一位置同一性质）；
- `bamboo-server/Cargo.toml` 新增 SILK 编解码依赖（见 §6，纯 Rust，无构建期系统依赖）；
- 配置与密钥全部进 `connect.json` 的微信平台条目，走既有加密/掩码管道，
  **不动 bamboo-config 的 schema 定义之外的任何东西**。

## 1. 依赖的外部事实

### 1a. 硅基流动 API

| 项 | 事实 |
|---|---|
| 接入点 | `https://api.siliconflow.cn`（可自定义 base URL），认证 `Authorization: Bearer <api_key>` |
| TTS | `POST /v1/audio/speech`，JSON：`model`、`input`（文本）、`voice`（`模型名:音色名`）、`response_format`（**mp3 / opus / wav / pcm**）、`sample_rate`、`speed`（0.25–4.0，默认 1.0）、`gain`（−10~10 dB）。**响应体即音频二进制**（非 JSON） |
| 采样率 | `pcm`/`wav` 支持 8000–44100（含 **16000**）；`opus` 仅 48000；`mp3` 仅 32000/44100 |
| 推荐模型 | `FunAudioLLM/CosyVoice2-0.5B`：中英日韩+方言（粤/川/沪），情感/口音可用 `<|endofprompt|>` 前缀指令控制；备选 `fishaudio/fish-speech-1.5`（音色克隆）、`fnlp/MOSS-TTSD-v0.5`（双人对话，需参考音频，本期不用） |
| 预置音色 | 男：alex/benjamin/charles/david；女：anna/bella/claire/diana。完整写法如 `FunAudioLLM/CosyVoice2-0.5B:anna` |
| ASR | `POST /v1/audio/transcriptions`，**multipart/form-data**：`file`（音频文件）+ `model`；模型 `FunAudioLLM/SenseVoiceSmall`（免费档）；响应 JSON `{"text": "..."}` |
| 计费 | TTS 按输入文本 UTF-8 字节数计费；SenseVoiceSmall 免费 |

### 1b. 微信语音条格式（SILK v3）

| 项 | 事实 |
|---|---|
| 编码 | 微信客户端**只认 Tencent SILK v3**，否则不显示为语音气泡 |
| 文件形态 | 微信变体 = **`\x02` 前缀字节** + `#!SILK_V3` 头的标准 SILK 流（且**无**结尾 `\xFF\xFF`）；入站反向：去掉首字节即标准 SILK |
| 参数 | 单声道（SILK 硬性限制）；采样率 16000 或 24000（微信语音条常规值），PCM 输入为 s16le |
| Rust 编解码 | [foyoux/silk-codec](https://github.com/foyoux/silk-codec)：纯 Rust 实现（Python 库 pilk 的底层），PCM↔SILK 双向；**实施前核对 crates.io 上的发布名与版本**（备选见 §8-R2） |
| 时长 | 语音条上限 60 秒；时长可由 PCM 字节数精确推出：`ms = pcm_bytes / (sample_rate × 2) × 1000` |

### 1c. 关键落点：TTS 直接产出 SILK 前置物，**全程无 ffmpeg**

硅基流动 `response_format: "pcm"` + `sample_rate: 16000` 返回的正是 s16le 单声道 PCM
（以 CosyVoice2 真机返回核对声道数，见 §8-R3）——与 SILK 编码输入完全一致。
因此出站链路是纯内存变换：**PCM →（silk-codec 编码）→ SILK →（加 `\x02`）→ 上传**，
不引入 ffmpeg/外部进程，跨 Windows/Linux 部署零差异。

## 2. 配置设计（connect.json，微信平台条目内）

> **2026-10-03 修订**：存储层的凭据材料守卫禁止普通分区出现明文密钥（最初设计的
> `siliconflow_api_key` 明文字段被 400 拒绝）。密钥改为**平台级 secret 字段
> `voice_api_key`**，与 `token` 走同一套凭据管道（加密落盘/凭据库引用/GET
> 状态视图/掩码回填）；设置 API 提交形态为
> `voice_api_key_change: {"action": "replace|keep|clear", "value": …}`，
> `voice` 段只保留非秘密参数。完整问题记录、接入清单与 API 契约见
> [`wechat-voice-secret-pipeline.md`](wechat-voice-secret-pipeline.md)。

```jsonc
{
  "type": "wechat",
  "token": "ilink bot_token（既有加密管道）",
  "allow_from": ["wxid_xxx@im.wechat"],
  "voice": {
    // 密钥不在 voice 段——它是平台级 secret 字段 voice_api_key（token 同款加密管道）
    "siliconflow_base_url": "https://api.siliconflow.cn",  // 可选，默认即此
    "tts_model": "FunAudioLLM/CosyVoice2-0.5B",
    "tts_voice": "FunAudioLLM/CosyVoice2-0.5B:anna",
    "tts_sample_rate": 16000,                  // 固定档 16000/24000，默认 16000
    "tts_speed": 1.0,
    "asr_model": "FunAudioLLM/SenseVoiceSmall",
    "file_asr": "off"                          // 音频文件转写开关：off | on，默认 off
  }
}
```

- **整个 `voice` 段可选**：缺省（无 `voice` 或无 `api_key`）时行为与现状完全一致
  （出站无语音条、入站只用微信转写）——向后兼容，零迁移；
  **不要手改文件**——`connect.json` 受分区哈希账本（attestation）保护，手改会导致 serve 拒绝启动
  （前车之鉴，见 CONNECT.md 故障排查）；
- `file_asr`（**音频文件转写开关，默认关**，见 §4）：微信语音条永远走微信自带转写，
  此开关只控制"用户发来的音频文件要不要额外转写文本"。SenseVoiceSmall 免费，
  开启无费用负担；识别失败/音乐/超限一律静默跳过，文件标记不受影响；
- `reply_mode`（出站语音/文字决策，详见 §3a）——**TTS 总开关，默认关**：
  - `off`（默认）：出站语音完全关闭，`[SEND_VOICE]` 标记也被忽略——
    **配好 api_key 也不会发语音**，必须显式改为此外的值才算开启（费用/行为默认保守）；
  - `mirror`：用户这条是语音 → 回语音；是文字 → 回文字；
  - `always`：普通回复总是语音（内容门槛仍然生效）。

## 3. 出站数据流（TTS 语音条）

### 3a. 语音/文字决策（网关确定性规则，不依赖模型自觉）

```
一条回复到达网关 reply()
  ① 网关自身的消息（审批提示、错误、指令回执、没看懂提示）→ 永远文本
     （审批选项需要看得清、回数字，不合成语音）
  ② 模型显式写了 [SEND_VOICE: 文本] → 语音候选（念标记内文本）
  ③ 模型普通回复 → 按 reply_mode（**默认 off，即 TTS 总开关默认关**）：
       off     → 文本（未显式开启即完全关闭，标记也忽略）
       mirror  → 本条入站消息含语音 → 语音候选；否则文本
       always  → 语音候选
  ④ 语音候选过【内容硬门槛】，任一命中即降级为纯文本：
       - 含代码块 / 行内代码 / 表格 / 文件路径
       - 含 URL（念出来没法用；文本规范化只救得了"顺带提到链接"的场景，
         主体是链接的直接降级）
       - 含 [SEND_FILE:] 标记（文件投递配文字）
       - 可见文本 ≥ 400 字（语音是短对话形态；长文分段念体验差且费 TTS）
  ⑤ 后续任一步失败（合成/编码/上传）→ 降级文本 + 一行"（语音合成失败，已改用文字）"
```

要点：

- **决策权在网关规则**，模型标记只是"用户明确要求朗读/语音回复"时的显式例外——
  行为可预测、TTS 费用可预测，不随模型心情漂移；
- **门槛是硬规则**：`always` 模式也不会出现念代码/念长文的荒谬场景；
- 语音回复**只发语音条，不重复发文字**（门槛已保证内容适合念）；
  模型需要"语音一句 + 详细文字"时用标记混排：标记行念出去，其余文字照发；
- 每条回复语音条数上限（默认 3，随分段策略生效）。

### 3b. 合成与投递管线

```
模型回复（§3a 判定为语音，或含 [SEND_VOICE] 标记行）
  "[SEND_VOICE: 今天下午三点开会，记得带笔记本]"
        │  wechat.rs reply()：解析标记（复用 SEND_FILE 的解析/剥离框架）
        ├─ 可见文本 → 照常分条发送（2000 字符切块不变）
        └─ 每个标记一次（自动模式则取整条可见文本）↓
  ⓪ TTS 文本规范化：剔除 markdown 符号（**、`、标题井号、列表符）与表情符号；
     URL → "链接"或只念域名；保留中文标点（否则星号井号会被念出来）
  ① 文本预处理：超长分段（按 TTS 计费与 60s 上限，~每段 ≤ 280 字；段落间独立合成）
  ② TTS：POST /v1/audio/speech  {model, input, voice, response_format:"pcm", sample_rate:16000}
        → s16le PCM（内存缓冲，不落盘）
  ③ SILK：silk-codec 编码 PCM→SILK v3；头部插入 \x02；算 voice_length（毫秒）
  ④ 上传：复用既有 getuploadurl（语音 media type，§8-R1 待核对）+ AES-128-ECB 加密 + CDN POST
  ⑤ 发送：sendmessage 携带 voice_item（voice_size=加密后大小, voice_length；字段名 §8-R1 核对）
  ⑥ 失败回退（任何一步）：日志记因，向用户发一条普通文本（原文本 + "（语音合成失败，已改用文字）"）
```

与 `[SEND_FILE]` 一致的三个原则：标记行不展示给用户；网关侧解析、引擎无感知；
失败绝不静默（发文字兜底，不卡会话）。

**前言扩展**（`bridge.rs`，一句话追加到既有渠道能力提示）：
"仅当用户明确要求语音回复或朗读时，才为对应文本单独一行附加
`[SEND_VOICE: 要念的文本]`；日常语音/文字的形态选择由网关自动处理，无需你决定。"

## 4. 入站数据流（ASR，仅音频文件）

**职责划分（2026-10-02 定稿）**：微信**语音条**继续用微信自带转写（免费、中文效果好、零延迟），
**不做** SILK 下载/解码/重新识别；硅基流动 SenseVoice **只识别用户发来的音频文件附件**
（mp3/wav/m4a/amr/aac/ogg/flac 等——现状只存盘标记路径，模型"听"不了内容）。
因此**入站不再需要 SILK 解码**，silk-codec 只服务出站编码，风险面减半。

现状（`extract_content_lines`，wechat.rs:1290 附近）：
- 语音条（`type == 3`）：只读 `voice_item.text`，有则 `[语音] 文本`、无则占位行——**维持不变**；
- 音频文件（`type == 4`）：下载解密存 `state_dir/media/`，标记 `[文件] 路径`——模型拿得到文件但读不出内容。

新增（`file_asr` 开启时，对音频扩展名的文件条目追加转写）：

```
文件条目到达（现有下载+解密+存盘流程不变）
  ├─ file_asr == off，或扩展名不在音频清单 → 现状路径（只标 [文件] 路径）
  └─ file_asr 开启 && 是音频文件：
       ① POST /v1/audio/transcriptions（multipart，file=已解密的原始文件，直接上传无需转码）
       ② 成功且非空：在 "[文件] 路径" 行外追加一行 "[音频转写] {text}"
          （模型既能读到说了什么，也保留文件路径可用工具处理）
       ③ 失败/空结果（音乐、无语音内容、超限）：静默跳过转写，文件标记照旧——不打扰、不报错
```

边界：单文件上传大小上限对齐接口限制（~25MB，超限直接跳过转写）；
长音频按接口实际支持时长在 M0 核对，超限同样静默跳过。

## 5. 文件改动清单（全部在网关层）

| 文件 | 改动 | 性质 |
|---|---|---|
| `connect/platforms/wechat_voice.rs` **（新）** | 硅基流动 client（TTS/ASR 两个方法）、SILK **编码**封装（PCM→`\x02`+SILK，仅需出站方向）、时长计算、文本分段与规范化 | 纯新增，仅被 wechat.rs 引用 |
| `connect/platforms/wechat.rs` | `VoiceConfig` 解析（含默认值）；`reply()` 增加 `[SEND_VOICE]` 标记解析与投递 + **§3a 语音/文字决策管线**（reply_mode 三态、内容门槛、文本规范化入口）；`extract_content_lines()` 的媒体下载循环增加音频文件转写分支；CDN 上传的语音 media type 分支 | 既有函数内扩展 |
| `connect/platforms/mod.rs` | `pub(crate) mod wechat_voice;` 一行 | 声明 |
| `connect/bridge.rs` | 前言追加一句 `[SEND_VOICE]` 说明 | 一行文案 |
| `connect` 平台配置结构（connect.json 对应的 Rust 侧 `ConnectPlatformConfig` 所在文件） | `voice: Option<VoicePlatformConfig>` 字段 + `siliconflow_api_key` 注册进 secret 管道 | connect 范围内 |
| `bamboo-server/Cargo.toml` | `silk-codec`（纯 Rust，无系统依赖） | 依赖 |
| `docs/guides/CONNECT.md` | 微信节补语音双向说明与配置示例 | 文档 |

明确**不改**：engine / bamboo-tools / bamboo-domain / 前端 / 其他平台适配器 /
providers.json（语音密钥与模型商无关，只在 connect）。

## 6. 测试计划

- 单元（wechat_voice.rs 内）：
  - SILK 编码产物头校验（`#!SILK_V3` 头 + `\x02` 前缀）；编码→外部解码器抽检（M0 定）；
    时长计算边界（0、恰好 60s、超限分段）；
  - 文本分段与规范化（中文标点不撕裂、markdown/表情剔除、URL 替换）；配置默认值与非法值回退。
- 网关（wechat.rs 既有测试风格，mock HTTP）：
  - `[SEND_VOICE]` 标记解析/剥离/多条；与 `[SEND_FILE]` 混用；
  - **§3a 决策规则逐条**：mirror（语音入站→语音候选/文本入站→文本）、always、off（标记忽略）、
    内容门槛各命中项（代码块/表格/URL/SEND_FILE/超长）降级文本、网关自身消息永远文本；
  - TTS 假服务返回固定 PCM → 断言 CDN 上传体与 sendmessage 的 voice_item 构造；
  - TTS 失败 → 回退文本消息（不卡、有提示）；
  - **文件转写两态**：`file_asr=on` 时音频文件（mp3/wav/m4a…）追加上传断言 + `[音频转写]` 行、
    非音频文件不触发、识别失败/空文本静默跳过、超限跳过、`off` 完全不调用；
  - 密钥 GET 掩码不回显。
- 真机验收清单：中文/混合文本语音条可播放、时长显示正确、60s 上限、
  发一段 mp3 得到 `[音频转写]`、发音乐文件安静跳过、语音条行为与旧版一致、密钥走正规写入路径。

## 7. 里程碑

1. **M0 spike（半天）**：核对 §8 的 R1/R3（抓包或 cc-connect 源码确认 voice_item 出站字段与 CDN 语音 media type；真机验证 CosyVoice2 `pcm@16000` 的声道数）——这两点落定前不动手写主逻辑；
2. **M1 出站**：silk-codec 接入 + TTS 合成 + 语音条投递 + 失败回退（含单测）；
3. **M2 入站**：音频文件 SenseVoice 转写 + `file_asr` 开关（含单测；无需 SILK 解码，较原方案大幅简化）；
4. **M3 文档与验收**：CONNECT.md、真机清单过一遍。

## 8. 风险与待核对清单

| # | 风险/未知 | 等级 | 对策 |
|---|---|---|---|
| R1 | iLink **出站** `voice_item` 的确切字段名与 `getuploadurl` 的语音 media type | ~~P0~~ **已结案（2026-10-05）：通道不支持，转文件投递** | 排查矩阵穷尽（encode_type 4/6/观测值、VOICE/FILE 上传、24k/16k、极简/全字段）后由**干净回显实验**定案：微信自己的语音条凭据全新、原样回显，`ret=0` 仍不渲染——bot 发出的 voice_item 被 iLink 通道丢弃，与字段/载荷无关。处置：`voice.delivery` 默认 `file`（TTS 直出 mp3 走文件通道，真机可用），`bubble` 保留实验开关。全程记录见 `wechat-voice-delivery-debug-log.md` |
| R2 | `silk-codec` 在 crates.io 的发布名/版本/维护状态未核实（现仅需**编码**方向，无解码需求） | P0 | M0 核对；备选： vendored C（kn007 silk-v3）FFI，或带 `ffmpeg` 子进程转码（打破"零外部依赖"，仅作最后手段） |
| R3 | CosyVoice2 `pcm` 输出的实际采样率/声道与文档声明不符 | P1 | M0 真机打印返回头核对；不符则请求 `wav` 并解析头重采样/抽取（纯 Rust，几十行） |
| R4 | SenseVoice 对长音频/大文件的支持边界（音乐文件返回空或报错） | P2 | ~25MB 上限 + 失败/空结果一律静默跳过（文件标记照旧）；长音频上限 M2 真机校准 |
| R5 | 微信对语音条的大小/时长风控（未公布） | P1 | 16k 单声道 SILK 约 1–2 KB/s，60s ≈ 60–120KB，远小于文件通道；保守起步，超限分段 |
| R6 | TTS 计费（按 UTF-8 字节）与滥用 | P2 | 分段上限 + 每条回复语音条数上限（默认 3，可配）；失败熔断当日计数 |
| R7 | 代理干扰（前车之鉴：系统代理指向死端口时全部外联失败） | P2 | 部署文档提示；错误信息明确区分网络层失败与鉴权失败 |
| R8 | 密钥写入渠道被手改破坏 attestation | P2 | 文档显著位置写明只能 `config set` / 设置 API 写入 |

## 9. 参考资料

- 硅基流动 TTS 指南：<https://api-docs.siliconflow.cn/docs/userguide/capabilities/text-to-speech>
- 硅基流动 ASR 接口：<https://api-docs.siliconflow.cn/docs/api/audio-transcriptions-post>
- SILK Rust 实现（pilk 底层）：<https://github.com/foyoux/silk-codec>；Python 绑定：<https://github.com/foyoux/pilk>
- 微信语音变体格式（`\x02` 前缀）：<https://github.com/kn007/silk-v3-decoder>、
  <https://qiita.com/shooter/items/21b01019886b29aeba6d>
- "微信只认 SILK v3 才显示语音气泡"的旁证：<https://github.com/NousResearch/hermes-agent/issues/9971>
- 经典 MP3→PCM→SILK 流程：<https://juejin.cn/post/7102030151721943070>
