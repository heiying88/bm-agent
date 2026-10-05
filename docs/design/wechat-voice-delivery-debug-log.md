# 微信语音出站投递调试记录（已结案：文件兜底）

> **终审结论（2026-10-05 12:08）**：iLink 机器人通道**不渲染 bot 发出的
> `voice_item`**。判决实验：把微信自己的语音条（凭据全新——回显模式探针
> 不再预下载媒体、media 引用/字段原样、无 null 脏字段）回显，sendmessage
> `ret=0 errcode=0` 但客户端不显示。至此载荷、字段、密钥形态、上传通道
> 全部排除——连微信自己生成的语音引用由 bot 发出都不渲染，是通道限制。
> 旁证：官方 `@tencent-weixin/openclaw-weixin` 只做语音接收无出站实现；
> Hermes 同症状 issue（#9971）长期未解；社区文档（lankerens/WeChat-iLinkBot）
> 只演示语音接收。**处置**：默认 `delivery: "file"`——TTS 直出 mp3 走
> 文件通道（真机可用，点开即播）；`delivery: "bubble"` 保留 SILK 语音条
> 路径供 iLink 未来支持时切换。
>
> 原始症状：语音条永远到不了微信（模型回复正常、TTS 正常、SILK 编码
> 正常、CDN 上传正常、sendmessage `ret=0 errcode=0`，但微信客户端
> 什么都不显示；同会话的纯文字/文件消息正常到达）。
> 关联设计文档：`wechat-voice-tts-asr-plan.md`（R1 风险项即本问题）。

## 1. 已确认的事实（日志 + 真机）

serve.log（UTF-16LE，`iconv -f UTF-16LE -t UTF-8` 转换）关键行：

```
connect: wechat voice enabled reply_mode=Mirror file_asr=false   ← 配置已生效
connect: wechat reply to=... visible_chars=0 voice_markers=1 auto_voice=true
connect: wechat voice to=... chars=58 silk_bytes=38151 playtime_ms=12640
connect: wechat sendmessage to=... ret=0 errcode=0 errmsg=None   ← 网关"接受"
```

- TTS（硅基流动）、SILK 编码（`\x02#!SILK_V3` 腾讯变体、24kHz）、
  getuploadurl、CDN 密文上传、x-encrypted-param 回读全链路正常。
- `ret=0 errcode=0` 但客户端不显示 = **网关容忍了 voice_item，客户端
  侧解码/下载失败后静默丢弃**。不是网络、不是密钥、不是配置。
- 纯文字消息同一信封正常到达 → sendmessage 信封（base_info/from_user_id=
  ""/client_id/message_state=2/context_token）没问题。
- 上传请求与官方包逐字段一致（filekey hex32/media_type/to_user_id/
  rawsize/rawfilemd5/filesize/no_need_thumb:true/aeskey hex）→ 上传构造没问题。
- media JSON（encrypt_type:1 + encrypt_query_param + aes_key=base64(hex)）
  与官方 send.ts 完全一致。

## 2. 真机试验矩阵（全部 sendmessage ret=0，均不显示）

| # | encode_type | 上传 media_type | voice_item 附加字段 | 载荷 | 结果 |
|---|---|---|---|---|---|
| 1 | 4（社区逆向原值） | 4 (VOICE) | playtime | SILK | ❌ 不显示 |
| 2 | 6（官方包注释 6=silk） | 4 (VOICE) | playtime/sample_rate/bits_per_sample | SILK | ❌ 不显示 |
| 3 | 三级解析（见 §4） | 3 (FILE) | 无（极简） | SILK | ⏳ 待真机验证 |

## 3. 两份权威资料的矛盾（核心未解之谜）

- **官方 `@tencent-weixin/openclaw-weixin@2.4.9`**（`src/api/types.ts` 注释）：
  `encode_type：1=pcm 2=adpcm 3=feature 4=speex 5=amr 6=silk 7=mp3 8=ogg-speex`。
  但官方包**没有出站语音实现**（send-media.ts 只有 image/video/file；
  silk-transcode.ts 是入站 SILK→WAV 解码），该枚举从未被官方用于出站。
- **cc-connect**（`platform/weixin/media_outbound.go`，README 语音 ✅，
  2026-04-12 fix #587 "TTS audio silently dropped" 与我们症状相同）：
  注释 `EncodeType: 0, // 0 = AMR format, 1 = SILK format`。其真机可用
  实现发的是 **AMR（ffmpeg 转）+ encode_type=0**，从未发过 SILK。
- 真机已排除：4（官方说 speex）与 6（官方说 silk）都不行。
  剩余候选：**入站观测值**（微信自己语音条带的值，最可信）、1（cc-connect
  说这是 SILK）、0（需要 AMR 载荷，不匹配我们的 SILK）。
- Hermes 同症状 issue（NousResearch/hermes-agent#9971）仍 open 未解决，
  无可抄答案。它声称 playtime 单位是秒（官方说毫秒）——未证实，
  现已按 cc-connect 配方整个去掉 playtime，绕开此问题。

## 4. 已写入代码但【未编译未测试】的改动（断点状态）

`crates/app/bamboo-server/src/connect/platforms/wechat.rs` 已完成以下编辑
（cargo test 被中断，**尚未跑过编译**）：

1. **上传通道**：`deliver_voice` 改用 `UPLOAD_MEDIA_FILE`(3) 上传
   （原来 VOICE=4；`UPLOAD_MEDIA_VOICE` 常量保留仅作文档）。
   依据：官方包定义了 VOICE=4 但从未使用；cc-connect 真机可用实现
   明确用 FILE 上传语音（"voice uses same CDN upload mechanism"）。
2. **voice_item 极简化**：只发 `{media, encode_type}`，去掉
   playtime/sample_rate/bits_per_sample（对齐 cc-connect 真机可用配方；
   它一个附加字段都不发，客户端自行解码取时长）。
3. **encode_type 三级解析** `resolve_outbound_voice_encode_type()`：
   - 环境变量 `BAMBOO_WECHAT_VOICE_ENCODE_TYPE`（强制覆盖，排查用）
   - > 入站观测值 `observed_voice_encode_type: AtomicI64`
     （poll_once 里观测微信自己语音条的 encode_type 并 INFO 日志
     `observed inbound voice encode_type=N`，自动校准——微信语音条就是
     SILK 载荷，它带的值即本线格式下 SILK 的真实取值）
   - > 兜底 `VOICE_ENCODE_TYPE_SILK = 6`
4. `IlinkVoiceItem` 增加 `encode_type: Option<i64>` 反序列化。
5. 投递日志行增加 `encode_type=` 字段；`resolve_*` 环境变量覆盖时 WARN。
6. 测试更新：主链路测试断言 FILE 上传 + playtime 缺失 + 兜底 6；
   新增 `outbound_voice_encode_type_adopts_observed_inbound_value`
   （入站带 encode_type=1 → 出站采用 1）。

## 5. 结案处置（替代原"下次继续"清单）

原排查矩阵全部执行完毕（encode_type 4/6/观测值、VOICE/FILE 上传、
24k/16k、极简/全字段、去 null 回显），最终由干净回显实验定案。处置：

1. **默认 `voice.delivery = "file"`**（WechatVoiceConfig 新字段，缺省即
   file）：TTS 直出 mp3（44100，mp3 档位与 pcm 不同）→ `upload_media`
   FILE 通道 → `file_item {media, file_name: 语音回复-<unix秒>.mp3, len}`。
   全链路无 SILK/无 ffmpeg，失败照旧回退文字。
2. `delivery = "bubble"` 保留原 SILK 语音条路径（16kHz/16kbps、
   encode_type 三级解析、入站校准全保留）——iLink 哪天支持了切回去。
3. 诊断工具保留：探针（每进程首条入站语音 dump 字段+字节）、
   `BAMBOO_WECHAT_VOICE_ECHO=1` 回显模式（回显时探针自动跳过下载、
   保住一次性凭据）、`BAMBOO_WECHAT_VOICE_ENCODE_TYPE` /
   `BAMBOO_WECHAT_VOICE_NO_SILK_PREFIX` 载荷实验开关。
4. 测试：32 个 wechat 测试全过（新增 `default_delivery_sends_voice_as_
   mp3_file`：mp3 请求参数/FILE 上传/file_item 形状/不构造 voice_item）；
   bamboo-config 2 个失败为已知预存在问题（~ 展开、no-follow，与本次无关）。
5. 文档：CONNECT.md §4.3 与能力清单已更新；本文件结案。

**若要重启 bubble 实验**（iLink 支持后）：`voice.delivery = "bubble"`
+ 真机发语音，看 `wechat voice to=... encode_type=` 后微信是否渲染。

## 6. 环境/工具备忘

- serve 由用户在 PowerShell 启动（输出重定向 UTF-16LE；**启动流程自带
  cargo build**，服务器停着时代码改动会被自动编入）；
  重启 serve 是用户操作，不要代杀进程。
- **系统代理坑（R7，已两次触发）**：Windows 系统代理开启而代理本体已死
  （127.0.0.1:12450 无监听）时，reqwest 的 system-proxy 会把网关所有
  外联（iLink/CDN/LLM）路由进死端口——症状是 getupdates/openai 全部
  `error sending request`。跑测试用 `NO_PROXY=127.0.0.1,localhost`。
- serve.log 在仓库根 `serve.log`；先 `iconv` 再 grep。
- 日志关键行：`voice enabled`（启动配置，含 delivery/sample_rate）、
  `observed inbound voice
  encode_type=`（入站校准）、`wechat voice to=... encode_type=`（出站）、
  `sendmessage ... ret= errcode=`（网关应答）、`voice delivery failed`。
- 官方 npm 包已解包在 `/tmp/ocw/package`（v2.4.9；Windows 侧无 node，
  是 curl 直接下 tgz 解的）。cc-connect 关键文件已存
  `/tmp/cc_media_outbound.go`、`/tmp/cc_types.go`、`/tmp/cc_client.go`。

## 7. 参考

- cc-connect 语音实现引入提交：`feat(weixin): implement SendAudio for TTS
  voice messages`（2026-04-12，fixes #587）；
  <https://github.com/chenhg5/cc-connect/blob/main/platform/weixin/media_outbound.go>
- 官方包类型定义（encode_type 枚举注释出处）：
  `@tencent-weixin/openclaw-weixin` `src/api/types.ts`（VoiceItem）
- Hermes 同症状未解 issue：
  <https://github.com/NousResearch/hermes-agent/issues/9971>
