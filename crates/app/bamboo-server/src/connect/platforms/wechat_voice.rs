//! 微信网关语音能力（**仅被 `wechat.rs` 引用**，网关层内部模块）：
//!
//! - SILK v3 编码（vendored Skype SILK SDK，`build.rs` 经 cc 编译，无
//!   libclang/bindgen 依赖），产出微信语音条的腾讯变体字节流；
//! - 硅基流动（SiliconFlow）TTS 合成与音频文件转写；
//! - 出站语音/文字决策（`reply_mode` 三态 + 内容硬门槛）与 TTS 文本
//!   规范化/分段。
//!
//! 行为规范：docs/design/wechat-voice-tts-asr-plan.md。

use std::ffi::c_int;
use std::ffi::c_short;
use std::ffi::c_void;

// ---------------------------------------------------------------------------
// 配置（connect.json `platforms[type=wechat].voice` 的运行时形态）
// ---------------------------------------------------------------------------

/// 出站语音/文字决策模式（TTS 总开关，默认 [`VoiceReplyMode::Off`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceReplyMode {
    /// 完全关闭出站语音，`[SEND_VOICE]` 标记也被忽略。
    Off,
    /// 用户这条是语音 → 回语音；是文字 → 回文字。
    Mirror,
    /// 普通回复总是语音（内容硬门槛仍然生效）。
    Always,
}

/// `voice` 段解析后的运行时配置（默认值已应用）。无 `siliconflow_api_key`
/// 时整段视为未启用（`VoiceConfig::from_config` 返回 `None`）。
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub api_key: String,
    pub base_url: String,
    pub tts_model: String,
    pub tts_voice: String,
    /// SILK 支持档位（16000/24000）；也是 TTS 请求的 pcm 采样率。
    pub sample_rate: u32,
    pub speed: f64,
    /// SILK 目标比特率（bps）。
    pub bitrate: i32,
    pub reply_mode: VoiceReplyMode,
    pub asr_model: String,
    pub file_asr: bool,
}

/// 硅基流动官方接入点。
const DEFAULT_SILICONFLOW_BASE_URL: &str = "https://api.siliconflow.cn";
const DEFAULT_TTS_MODEL: &str = "FunAudioLLM/CosyVoice2-0.5B";
const DEFAULT_TTS_VOICE: &str = "FunAudioLLM/CosyVoice2-0.5B:anna";
const DEFAULT_ASR_MODEL: &str = "FunAudioLLM/SenseVoiceSmall";
/// SILK 编码默认比特率（对齐 rust-silk CLI 默认值，微信语音条常规档）。
const DEFAULT_SILK_BITRATE: i32 = 25_000;

impl VoiceConfig {
    /// 从 connect 配置解析；`api_key` 缺失/为空 → `None`（语音功能整体
    /// 禁用，行为与未配置时完全一致）。非法枚举值回退默认并告警。
    pub fn from_config(config: &bamboo_config::WechatVoiceConfig) -> Option<Self> {
        let api_key = config
            .siliconflow_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())?
        .to_string();

        let reply_mode = match config.reply_mode.as_deref().map(str::trim) {
            None | Some("") | Some("off") => VoiceReplyMode::Off,
            Some("mirror") => VoiceReplyMode::Mirror,
            Some("always") => VoiceReplyMode::Always,
            Some(other) => {
                tracing::warn!(
                    "connect: wechat voice.reply_mode={other:?} 不认识，回退 off（合法值：off/mirror/always）"
                );
                VoiceReplyMode::Off
            }
        };
        let sample_rate = config
            .tts_sample_rate
            .filter(|rate| matches!(rate, 16_000 | 24_000))
            // 默认对齐官方 openclaw-weixin 包的微信语音采样率（24kHz，
            // 见其 silk-transcode 的 SILK_SAMPLE_RATE）；16000 同样合法。
            .unwrap_or(24_000);

        Some(Self {
            api_key,
            base_url: config
                .siliconflow_base_url
                .as_deref()
                .map(|url| url.trim().trim_end_matches('/').to_string())
                .filter(|url| !url.is_empty())
                .unwrap_or(DEFAULT_SILICONFLOW_BASE_URL.to_string()),
            tts_model: config
                .tts_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_TTS_MODEL)
                .to_string(),
            tts_voice: config
                .tts_voice
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_TTS_VOICE)
                .to_string(),
            sample_rate,
            speed: config.tts_speed.unwrap_or(1.0).clamp(0.25, 4.0),
            bitrate: config.tts_bitrate.unwrap_or(DEFAULT_SILK_BITRATE),
            reply_mode,
            asr_model: config
                .asr_model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_ASR_MODEL)
                .to_string(),
            file_asr: config.file_asr.as_deref().map(str::trim) == Some("on"),
        })
    }
}

// ---------------------------------------------------------------------------
// SILK v3 编码（vendored SDK 的手写绑定，仅编码方向）
// ---------------------------------------------------------------------------

/// 与 vendor/silk/interface/SKP_Silk_control.h 的
/// `SKP_SILK_SDK_EncControlStruct` 逐字段对齐（8 × 32 位整数）。
#[repr(C)]
struct SilkEncControl {
    api_sample_rate: c_int,
    max_internal_sample_rate: c_int,
    packet_size: c_int,
    bit_rate: c_int,
    packet_loss_percentage: c_int,
    complexity: c_int,
    use_inband_fec: c_int,
    use_dtx: c_int,
}

extern "C" {
    fn SKP_Silk_SDK_Get_Encoder_Size(enc_size_bytes: *mut c_int) -> c_int;
    fn SKP_Silk_SDK_InitEncoder(
        enc_state: *mut c_void,
        enc_status: *mut SilkEncControl,
    ) -> c_int;
    fn SKP_Silk_SDK_Encode(
        enc_state: *const c_void,
        enc_control: *const SilkEncControl,
        samples_in: *const i16,
        n_samples_in: c_int,
        out_data: *mut u8,
        n_bytes_out: *mut c_short,
    ) -> c_int;
}

/// 微信语音条腾讯变体的头两段：`\x02` 前缀 + 标准 `#!SILK_V3` 魔数。
const TENCENT_SILK_PREFIX: &[u8] = b"\x02#!SILK_V3";
/// SILK 每帧 20ms；`MAX_INPUT_FRAMES` 对齐 SDK 单包最多 5 帧。
const SILK_FRAME_MS: i32 = 20;
const SILK_MAX_INPUT_FRAMES: usize = 5;
/// 对齐 SDK 的单帧最大编码字节数。
const SILK_MAX_BYTES_PER_FRAME: usize = 250;

/// 把 s16le 单声道 PCM 编码为微信语音条字节流（腾讯变体：
/// `\x02#!SILK_V3` + 每包 `u16le 长度 + 载荷`，无标准变体的结尾标记）。
/// `sample_rate` 须为 8000/12000/16000/24000 之一。
pub fn encode_silk_tencent(
    pcm: &[u8],
    sample_rate: u32,
    bitrate: i32,
) -> Result<Vec<u8>, String> {
    if !matches!(sample_rate, 8_000 | 12_000 | 16_000 | 24_000) {
        return Err(format!("不支持的 SILK 采样率：{sample_rate}"));
    }
    debug_assert_eq!(std::mem::size_of::<SilkEncControl>(), 32);
    let sample_rate = sample_rate as i32;

    // 对齐 rust-silk CLI 的默认编码参数（微信语音常规档）。
    let control = SilkEncControl {
        api_sample_rate: sample_rate,
        max_internal_sample_rate: sample_rate,
        packet_size: SILK_FRAME_MS * sample_rate / 1000,
        bit_rate: bitrate,
        packet_loss_percentage: 0,
        complexity: 2,
        use_inband_fec: 0,
        use_dtx: 0,
    };

    let frame_samples = (SILK_FRAME_MS * sample_rate / 1000) as usize;
    let mut output = Vec::with_capacity(pcm.len() / 4 + 16);
    output.extend_from_slice(TENCENT_SILK_PREFIX);

    unsafe {
        let mut enc_size: c_int = 0;
        if SKP_Silk_SDK_Get_Encoder_Size(&mut enc_size) != 0 || enc_size <= 0 {
            return Err("SILK 编码器初始化失败（Get_Encoder_Size）".to_string());
        }
        let mut enc_state = vec![0u8; enc_size as usize];
        let mut enc_status = SilkEncControl {
            api_sample_rate: 0,
            max_internal_sample_rate: 0,
            packet_size: 0,
            bit_rate: 0,
            packet_loss_percentage: 0,
            complexity: 0,
            use_inband_fec: 0,
            use_dtx: 0,
        };
        if SKP_Silk_SDK_InitEncoder(enc_state.as_mut_ptr() as *mut c_void, &mut enc_status) != 0 {
            return Err("SILK 编码器初始化失败（InitEncoder）".to_string());
        }

        let mut payload = vec![0u8; SILK_MAX_BYTES_PER_FRAME * SILK_MAX_INPUT_FRAMES];
        let mut samples = vec![0i16; frame_samples];
        // 每次消费一包（20ms）的 PCM 字节；包计数与 SDK 的
        // smpls_since_last_packet 语义一致（packet_size == frame_samples）。
        for chunk in pcm.chunks(frame_samples * 2) {
            if chunk.len() < frame_samples * 2 {
                // 末尾不足一帧：补零到整帧（对齐 CLI 行为）。
                let mut padded = chunk.to_vec();
                padded.resize(frame_samples * 2, 0);
                for (index, pair) in padded.chunks_exact(2).enumerate() {
                    samples[index] = i16::from_le_bytes([pair[0], pair[1]]);
                }
            } else {
                for (index, pair) in chunk.chunks_exact(2).enumerate() {
                    samples[index] = i16::from_le_bytes([pair[0], pair[1]]);
                }
            }
            let mut n_bytes: c_short = payload.len() as c_short;
            let status = SKP_Silk_SDK_Encode(
                enc_state.as_ptr() as *const c_void,
                &control,
                samples.as_ptr(),
                samples.len() as c_int,
                payload.as_mut_ptr(),
                &mut n_bytes,
            );
            if status != 0 {
                return Err(format!("SILK 编码失败：{status}"));
            }
            if n_bytes > 0 {
                output.extend_from_slice(&(n_bytes as u16).to_le_bytes());
                output.extend_from_slice(&payload[..n_bytes as usize]);
            }
        }
    }
    Ok(output)
}

/// PCM 时长（毫秒）：`字节数 / (采样率 × 2) × 1000`（s16le 单声道）。
pub fn pcm_duration_ms(pcm_len: usize, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    (pcm_len as u64 * 1000) / (sample_rate as u64 * 2)
}

// ---------------------------------------------------------------------------
// 硅基流动 client（TTS 合成 + 音频文件转写）
// ---------------------------------------------------------------------------

/// TTS 合成：`POST {base}/v1/audio/speech`，直接返回 s16le 单声道 PCM
/// （`response_format=pcm`），与 SILK 编码输入完全一致——全程无 ffmpeg。
pub async fn synthesize_pcm(http: &reqwest::Client, cfg: &VoiceConfig, text: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}/v1/audio/speech", cfg.base_url);
    let body = serde_json::json!({
        "model": cfg.tts_model,
        "input": text,
        "voice": cfg.tts_voice,
        "response_format": "pcm",
        "sample_rate": cfg.sample_rate,
        "speed": cfg.speed,
    });
    let response = http
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("TTS 请求失败：{error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("TTS 返回 HTTP {status}：{}", truncate_error(&detail)));
    }
    // 错误时接口返回 JSON；成功返回音频二进制。
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("TTS 响应读取失败：{error}"))?;
    if content_type.contains("json") {
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .or_else(|| value.get("error"))
                    .and_then(|message| message.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| truncate_error(&String::from_utf8_lossy(&bytes)));
        return Err(format!("TTS 接口错误：{message}"));
    }
    if bytes.is_empty() {
        return Err("TTS 返回空音频".to_string());
    }
    Ok(bytes.to_vec())
}

/// 音频文件转写：`POST {base}/v1/audio/transcriptions`（multipart，手工
/// 拼装——workspace 的 reqwest 未启用 multipart feature，不值得为一个
/// 字段全仓库加 feature）。返回识别文本。
pub async fn transcribe_file(
    http: &reqwest::Client,
    cfg: &VoiceConfig,
    file_bytes: &[u8],
    file_name: &str,
) -> Result<String, String> {
    let boundary = format!("bamboo-wechat-voice-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(file_bytes.len() + 512);
    let mut push_part = |body: &mut Vec<u8>, name: &str, value: &[u8], file_name: Option<&str>| {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match file_name {
            Some(file_name) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                );
            }
            None => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                );
            }
        }
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    };
    push_part(&mut body, "file", file_bytes, Some(file_name));
    push_part(&mut body, "model", cfg.asr_model.as_bytes(), None);
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let url = format!("{}/v1/audio/transcriptions", cfg.base_url);
    let response = http
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(|error| format!("转写请求失败：{error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!("转写返回 HTTP {status}：{}", truncate_error(&detail)));
    }
    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("转写响应解析失败：{error}"))?;
    Ok(parsed
        .get("text")
        .and_then(|text| text.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

fn truncate_error(text: &str) -> String {
    let mut summary: String = text.chars().take(200).collect();
    if summary.len() < text.len() {
        summary.push('…');
    }
    summary
}

// ---------------------------------------------------------------------------
// 出站语音/文字决策 + 文本处理
// ---------------------------------------------------------------------------

/// 单条语音条的文本上限（约 250 字 ≈ 55 秒语速，留足 60s 裕量）。
pub const MAX_VOICE_SEGMENT_CHARS: usize = 250;
/// 每条回复的语音条数上限（显式标记与自动模式共用）。
pub const MAX_VOICE_SEGMENTS: usize = 3;
/// 自动（非显式标记）模式下走语音的可见文本长度上限，超过降级文字。
pub const AUTO_VOICE_MAX_CHARS: usize = 400;
/// 转写上传的音频文件大小上限（对齐 OpenAI 兼容接口惯例）。
pub const MAX_ASR_FILE_BYTES: usize = 25 * 1024 * 1024;
/// 微信语音条时长上限（毫秒）。
pub const MAX_VOICE_DURATION_MS: u64 = 60_000;

/// 出站内容硬门槛：判定一段可见文本是否适合以语音念出。
/// 代码/表格/URL/文件标记/超长一律降级文字（见设计文档 §3a）。
pub fn is_voice_suitable(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() >= AUTO_VOICE_MAX_CHARS {
        return false;
    }
    if trimmed.contains("```") || trimmed.contains('`') {
        return false;
    }
    // 表格行（行内含竖线成对）。
    if trimmed.lines().any(|line| line.matches('|').count() >= 2) {
        return false;
    }
    if trimmed.contains("[SEND_FILE:") || trimmed.contains("[SEND_VOICE:") {
        return false;
    }
    if trimmed.contains("http://") || trimmed.contains("https://") {
        return false;
    }
    true
}

/// TTS 文本规范化：剔除 markdown 强调符/标题/列表标记/行内代码残留、
/// 链接文本保留但裸 URL 替换为“链接”、剔除常见表情符号，压缩连续空白。
pub fn normalize_for_tts(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for line in text.lines() {
        let mut line = line.trim().to_string();
        // 标题与列表前缀。
        while line.starts_with('#') {
            line.remove(0);
        }
        for prefix in ["- ", "* ", "+ ", "> "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                line = rest.to_string();
                break;
            }
        }
        if let Some(rest) = line.strip_prefix("1. ") {
            line = rest.to_string();
        }
        normalized.push_str(line.trim());
        normalized.push('\n');
    }
    let mut normalized = normalized.trim().to_string();
    // 链接 [文本](url) → 文本（链接）；裸 URL → 链接。
    let mut replaced = String::with_capacity(normalized.len());
    let mut rest = normalized.as_str();
    while let Some(start) = rest.find('[') {
        replaced.push_str(&rest[..start]);
        if let Some((link_text, after)) = extract_markdown_link(&rest[start..]) {
            replaced.push_str(&link_text);
            replaced.push_str("（链接）");
            rest = after;
        } else {
            replaced.push('[');
            rest = &rest[start + 1..];
        }
    }
    replaced.push_str(rest);
    normalized = replaced;
    normalized = normalize_urls(&normalized);
    // 强调符与行内代码残留。
    normalized = normalized
        .replace("**", "")
        .replace("__", "")
        .replace('`', "");
    // 表情符号（常见区段；够用即可，不做完整 emoji 数据库）。
    normalized = normalized
        .chars()
        .filter(|&ch| {
            !matches!(ch as u32,
                0x1F000..=0x1FAFF      // 表意表情与扩展
                | 0x2600..=0x27BF      // 杂项符号与装饰
                | 0xFE00..=0xFE0F      // 变体选择符
                | 0x200D               // 零宽连接符
                | 0x2190..=0x21FF      // 箭头
            )
        })
        .collect();
    // 连续空白压一行内一个空格、连续空行压一行。
    let mut collapsed = String::with_capacity(normalized.len());
    let mut blank = false;
    for line in normalized.lines() {
        let line: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank {
                collapsed.push('\n');
            }
            blank = true;
        } else {
            blank = false;
            collapsed.push_str(&line);
            collapsed.push('\n');
        }
    }
    collapsed.trim().to_string()
}

/// `[文本](url)` → 返回 `(文本, 其后剩余)`；不是该形态返回 `None`。
fn extract_markdown_link(input: &str) -> Option<(String, &str)> {
    let close = input.find(']')?;
    let after_close = &input[close + 1..];
    let rest = after_close.strip_prefix('(')?;
    let end = rest.find(')')?;
    Some((input[1..close].to_string(), &rest[end + 1..]))
}

/// 裸 URL（含 www. 开头）替换为“链接”。
fn normalize_urls(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    'outer: while !rest.is_empty() {
        for prefix in ["https://", "http://", "www."] {
            if let Some(start) = rest.find(prefix) {
                output.push_str(&rest[..start]);
                let tail = &rest[start..];
                let end = tail
                    .find(|ch: char| ch.is_whitespace() || "，。；！？、）)]\"".contains(ch))
                    .unwrap_or(tail.len());
                output.push_str("链接");
                rest = &tail[end..];
                continue 'outer;
            }
        }
        output.push_str(rest);
        break;
    }
    output
}

/// 按句读（。！？!?；;\n）分段，段长不超过 `max_chars`；超长无句读的
/// 连续文本按 `max_chars` 硬切。空段被丢弃。
pub fn segment_for_tts(text: &str, max_chars: usize) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    for line in text.split('\n') {
        let mut sentence = String::new();
        for ch in line.chars() {
            sentence.push(ch);
            if "。！？!?；;".contains(ch) {
                try_append(&mut segments, &mut current, &sentence, max_chars);
                sentence.clear();
            }
        }
        try_append(&mut segments, &mut current, &sentence, max_chars);
        if !current.is_empty() {
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        hard_split_into(&mut segments, current.trim(), max_chars);
    }
    segments
}

fn try_append(segments: &mut Vec<String>, current: &mut String, sentence: &str, max_chars: usize) {
    if sentence.trim().is_empty() {
        return;
    }
    if current.chars().count() + sentence.chars().count() > max_chars {
        if !current.trim().is_empty() {
            hard_split_into(segments, current.trim(), max_chars);
            current.clear();
        }
        hard_split_into(segments, sentence.trim(), max_chars);
    } else {
        current.push_str(sentence);
    }
}

fn hard_split_into(segments: &mut Vec<String>, text: &str, max_chars: usize) {
    if text.trim().is_empty() {
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(max_chars) {
        let segment: String = chunk.iter().collect();
        if !segment.trim().is_empty() {
            segments.push(segment);
        }
    }
}

/// 判断文件名是否为音频（用于入站文件转写触发）。
pub fn is_audio_file_name(name: &str) -> bool {
    const AUDIO_EXTENSIONS: &[&str] = &[
        "mp3", "wav", "m4a", "amr", "aac", "ogg", "oga", "flac", "webm", "wma", "opus", "silk",
        "aud", "slk",
    ];
    let extension = name
        .rsplit('.')
        .next()
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default();
    AUDIO_EXTENSIONS.contains(&extension.as_str())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silk_encode_produces_tencent_prefix_and_packets() {
        // 1 秒 16kHz 静音 PCM。
        let pcm = vec![0u8; 16_000 * 2];
        let silk = encode_silk_tencent(&pcm, 16_000, 25_000).expect("encode");
        assert!(silk.starts_with(TENCENT_SILK_PREFIX));
        // 每包 u16le 长度前缀 + 载荷，长度必须自洽。
        let mut offset = TENCENT_SILK_PREFIX.len();
        let mut packets = 0;
        while offset < silk.len() {
            assert!(
                offset + 2 <= silk.len(),
                "truncated packet length prefix at {offset}"
            );
            let length = u16::from_le_bytes([silk[offset], silk[offset + 1]]) as usize;
            assert!(length > 0, "empty packet");
            offset += 2 + length;
            packets += 1;
        }
        assert_eq!(offset, silk.len(), "trailing bytes after last packet");
        // 1 秒 / 20ms = 50 包（静音下 DTX 关闭仍逐包产出）。
        assert_eq!(packets, 50);
        // 时长换算。
        assert_eq!(pcm_duration_ms(pcm.len(), 16_000), 1000);
    }

    #[test]
    fn silk_encode_rejects_unsupported_rate() {
        assert!(encode_silk_tencent(&[0; 4], 44_100, 25_000).is_err());
    }

    #[test]
    fn voice_config_defaults_and_gating() {
        let none = bamboo_config::WechatVoiceConfig::default();
        assert!(VoiceConfig::from_config(&none).is_none());

        let mut configured = bamboo_config::WechatVoiceConfig::default();
        configured.siliconflow_api_key = Some("sk-test".to_string());
        let parsed = VoiceConfig::from_config(&configured).expect("enabled");
        assert_eq!(parsed.reply_mode, VoiceReplyMode::Off);
        assert_eq!(parsed.sample_rate, 24_000);
        assert!(!parsed.file_asr);
        assert_eq!(parsed.base_url, DEFAULT_SILICONFLOW_BASE_URL);

        configured.reply_mode = Some("mirror".to_string());
        configured.file_asr = Some("on".to_string());
        configured.tts_sample_rate = Some(16_000);
        let parsed = VoiceConfig::from_config(&configured).expect("enabled");
        assert_eq!(parsed.reply_mode, VoiceReplyMode::Mirror);
        assert!(parsed.file_asr);
        assert_eq!(parsed.sample_rate, 16_000);

        configured.reply_mode = Some("bogus".to_string());
        assert_eq!(
            VoiceConfig::from_config(&configured).unwrap().reply_mode,
            VoiceReplyMode::Off
        );
    }

    #[test]
    fn voice_suitability_gate() {
        assert!(is_voice_suitable("今天下午三点开会，记得带笔记本"));
        assert!(!is_voice_suitable("运行 `cargo build` 一下"));
        assert!(!is_voice_suitable("```rust\nfn main() {}\n```"));
        assert!(!is_voice_suitable("看这个 | 表格 | 例子"));
        assert!(!is_voice_suitable("详见 https://example.com"));
        assert!(!is_voice_suitable("[SEND_FILE: C:\\a.png]"));
        assert!(!is_voice_suitable(&"很长".repeat(300)));
        assert!(!is_voice_suitable("   "));
    }

    #[test]
    fn tts_normalization_strips_markdown_and_emoji() {
        let normalized = normalize_for_tts(
            "# 标题\n- **重点**是 `速度`\n详见 [文档](https://example.com/a) 或 https://b.cn/x\n好的😀",
        );
        assert!(normalized.contains("重点") && normalized.contains("速度"), "got: {normalized}");
        assert!(!normalized.contains("**"));
        assert!(!normalized.contains('`'));
        assert!(!normalized.contains('#'));
        assert!(!normalized.contains("https://"));
        assert!(!normalized.contains("example.com"));
        assert!(!normalized.contains("b.cn"));
        assert!(normalized.contains("文档（链接）"));
        assert!(normalized.contains("链接"));
        assert!(!normalized.contains('😀'));
    }

    #[test]
    fn tts_segmentation_respects_boundaries_and_caps() {
        let segments = segment_for_tts("第一句话。第二句话！这是第三句；还有第四句呢。", 10);
        assert!(!segments.is_empty());
        assert!(segments.iter().all(|segment| segment.chars().count() <= 10));
        assert_eq!(segments.join(""), "第一句话。第二句话！这是第三句；还有第四句呢。");

        // 无句读超长文本硬切。
        let long = "长".repeat(60);
        let segments = segment_for_tts(&long, 25);
        assert!(segments.iter().all(|segment| segment.chars().count() <= 25));
        assert_eq!(segments.concat(), long);
    }

    #[test]
    fn audio_file_name_detection() {
        assert!(is_audio_file_name("meeting.MP3"));
        assert!(is_audio_file_name("录音.m4a"));
        assert!(is_audio_file_name("voice.amr"));
        assert!(!is_audio_file_name("报告.docx"));
        assert!(!is_audio_file_name("noextension"));
    }
}
