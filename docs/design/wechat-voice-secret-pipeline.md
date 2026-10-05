# 微信语音密钥的存储管道接入：问题记录与决策

> 状态：**已实施**（2026-10-03）。本文记录语音功能落地过程中发现的两个
> 存储层问题、最终解法与接入清单，供后续给 connect 平台新增 secret 字段时
> 照抄。姊妹文档：[`wechat-voice-tts-asr-plan.md`](wechat-voice-tts-asr-plan.md)
> （语音功能总体设计，其 §2 含本次修订说明）。

## 1. 发现的两个问题

### 问题 A（code review 发现）：PUT /connect 端点会抹掉语音配置

专用端点 `PUT /v1/bamboo/config/connect`
（`handlers/settings/bamboo_config/connect.rs`）在重建平台条目时
把 `voice` **硬编码为 `None`**，且请求结构体 `ConnectPlatformMutation`
带 `#[serde(deny_unknown_fields)]` 又**没有 voice 字段**——后果：

- 提交体里带 `voice` → 反序列化直接 400（未知字段被拒）；
- 即使通过其他途径配好了语音，任何一次经该端点的整单保存（设置 UI
  保存就是这条路径）都会**静默清空语音配置**。

### 问题 B（修复测试时暴露）：存储层架构性禁止明文密钥

最初的语音设计把 API 密钥作为明文字段 `siliconflow_api_key` 放在
`voice` 段里（"按部署方明文配置偏好"）。端到端测试立刻被拒：

```
400 Bad request: ordinary section contains forbidden credential material in siliconflow_api_key
```

守卫在 `bamboo-config/src/section_facade.rs` 的
`validate_ordinary_section_json` → `key_contains_literal_credential_material`：
**按字段名模式匹配**（`*_encrypted` / `*token` / `*password` / `*secret` /
`apikey` 词元等）拒绝普通分区里的字符串/数字值。这是安全架构的硬边界，
不是可绕过的校验——命名回避（比如改叫别的名字）既脆弱又自欺。

**结论：任何新密钥字段必须走既有凭据管道，没有"明文例外"。**

## 2. 决策：平台级 secret 字段 `voice_api_key`（token 同款管道）

密钥从 `voice` 段移出，成为 `ConnectPlatformConfig` 的平台级 secret，
与 `token` / `app_secret` 完全同构的四件套：

| 字段 | serde | 作用 |
|---|---|---|
| `voice_api_key` | `skip_serializing` | 内存中的明文（加载/水合时填充，绝不落盘） |
| `voice_api_key_encrypted` | 普通可选 | 落盘密文（legacy 路径；凭据库路径下为 None） |
| `voice_api_key_credential_ref` | 普通可选 | 凭据库稳定引用 `connect.<platform_id>.voice_api_key` |
| `voice_api_key_configured` | 默认 false | 供掩码客户端判断"已配置"而不暴露值 |

`voice` 段（`WechatVoiceConfig`）只保留**非秘密参数**：base_url、TTS
模型/音色/采样率/语速/比特率、`reply_mode`、ASR 模型、`file_asr`。
网关运行时用 `voice_api_key`（水合后的明文）+ `voice` 段共同构造
`VoiceConfig`（`wechat_voice.rs::VoiceConfig::from_config`）。

## 3. API 契约（PUT /v1/bamboo/config/connect）

```jsonc
{
  "expected_revision": 3,
  "data": {
    "platforms": [{
      "id": "<服务端分配的平台 id>",
      "type": "wechat",
      "allow_from": ["wxid_xxx@im.wechat"],
      "token_change": {"action": "keep"},
      // 密钥：与 token_change 同一形态（CredentialAction）
      "voice_api_key_change": {"action": "replace", "value": "sk-…"},
      //   keep    = 保留现有（缺省语义）
      //   replace = 替换
      //   clear   = 清除
      // voice 段：非秘密参数；缺省/null = 保留现有；提交 = 整体替换
      //（不与旧值做字段级合并；要"关闭"提交空对象即可）
      "voice": {"reply_mode": "mirror", "file_asr": "on"}
    }]
  }
}
```

- **缺省保留语义**：不认识这些字段的旧客户端整单保存，不会抹掉语音
  配置（voice 与密钥都保留）；显式提交才覆盖；
- GET 响应中每个平台带 `credential_status.<id>.voice_api_key` 状态视图
  （configured/credential_ref/state，无明文）；顶层 `credentials` 数组
  同样包含该引用；
- 密钥值**绝不**出现在任何响应、日志或 connect.json/credentials.json
  的明文里（测试断言覆盖）。

## 4. 接入点清单（给后续新增 connect secret 字段照抄）

以 `app_secret` 为模板，全部接入点（共 6 个文件）：

| 文件 | 位置 | 改动 |
|---|---|---|
| `bamboo-config/src/config.rs` | `ConnectPlatformConfig` | 四件套字段（voice_api_key 明文字段 + `_encrypted` + `_credential_ref` + `_configured`） |
| `bamboo-config/src/config_crypto.rs` | `hydrate_connect_platform_tokens_from_encrypted` | 密文 → 明文水合分支 |
| 同上 | `hydrate_connect_credentials_from_resolver` | 凭据库水合 + 清空 `_encrypted` |
| 同上 | `refresh_connect_platform_tokens_encrypted` | 明文 → 密文（落盘前） |
| 同上 | `sanitize_connect_credentials_for_disk` | 清运行时明文与 legacy 密文 |
| `bamboo-config/src/patch.rs` | `ConnectSecretIntents` + `connect_secret_intents` | intent 集合与提取 |
| 同上 | `clear_connect_ciphertext_for_explicit_clears` | 显式清除时同步清密文 |
| 同上 | `preserve_masked_connect_secrets` | 掩码占位符回填 |
| `bamboo-config/src/credential_store.rs` | `PersistedConnectCredentialRefs`（**二元组→三元组**）+ `prepare_connect_intents` 的 candidate/fields 表 | 持久化引用、意图应用 |
| `bamboo-config/src/credential_migration.rs` | `connect_refs_from_document`、reset-scope 链、独占检查循环、staged 合并、legacy 迁移表 | 三元组解构 + 表驱动字段行 |
| `bamboo-server/.../bamboo_config/connect.rs` | mutation 结构体、intent 校验、`apply_action` 重建、`connect_response` 的 active_refs 与 per-platform 状态 | `voice_api_key_change` + 引用视图 |
| `bamboo-server/src/connect/mod.rs` | 平台构造 | 用平台级明文密钥 + voice 段构造 `VoiceConfig` |

## 5. 写入途径的事实核实

- **`bamboo config set` 对 `connect.platforms` 不可用**（实测）：
  平台 `token` 是加密管道字段、序列化时被剥离，dot-path 的 round-trip
  守卫直接拒绝（`value did not survive a config round-trip`）；数组
  下标路径在中间段缺失时也无法创建。**connect 的唯一正规写入途径是
  `PUT /v1/bamboo/config/connect`**（设置 UI / curl 均走它）；
- 手改 `connect.json` 依然被分区哈希账本（attestation）拒绝启动。

## 6. 验证记录（2026-10-03）

- `dedicated_connect_mutation_supports_and_preserves_voice`（新增，端点级）：
  写入 → 凭据库引用生成、明文不落盘、voice 参数持久化；缺省提交保留；
  显式替换生效；凭据文件无明文；
- 网关套件 91 通过（含语音链路全部既有测试）；语音单元 7 通过；
  bamboo-config 683 通过（2 个失败为既有 Windows 环境问题：
  `~` 展开路径断言与 no-follow 文件 helper，与本次无关）；
- 全链路 e2e：PUT 带 `voice_api_key_change` → 响应含
  `connect.<id>.voice_api_key` 引用与 `voice` 段 → 磁盘 connect.json
  只有 ref/configured、无明文。

## 7. 给前端（Lotus）的注意事项

后续若在设置页加语音配置卡片：照 token 卡片的交互做——
`voice_api_key` 只显示状态（已配置/未配置），输入框提交
`voice_api_key_change: replace`，永远不回显明文；`voice` 段是普通
表单字段，整段提交（缺省不回传即保留）。
