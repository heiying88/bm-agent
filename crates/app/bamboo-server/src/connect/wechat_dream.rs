//! 微信网关的 Project Dream 自动运行计划。
//!
//! 两种触发模式（`[[connect.platforms]]` type=wechat 条目的 `dream` 段）：
//!
//! - `daily`（默认）：每天在 `daily_at`（HH:MM，默认 03:00，本地时区）对
//!   微信网关所属项目跑一次 [`run_project_auto_dream_once_for_project`]。
//! - `idle`（"项目休息时间"）：网关闲置 `idle_minutes`（默认 30）后自动
//!   跑一次；新入站消息会重置闲置计时（dream 之后必须再次出现活动并
//!   重新闲置才会再跑）。
//!
//! 上次运行状态（每日模式已跑日期 / idle 模式上次 dream 时间）持久化在
//! `<data_dir>/connect_wechat_dream_state.json`，重启后不重复跑当日份额。
//! 任务常驻但完全惰性：没有配置 wechat 条目或 mode=off 时每轮直接返回。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use bamboo_engine::auto_dream::{run_project_auto_dream_once_for_project, AutoDreamContext};

use super::ConnectBridge;

/// 轮询间隔：daily 模式的分钟级精度 + idle 模式的分钟级阈值都足够。
const TICK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Default, Serialize, Deserialize)]
struct WechatDreamState {
    /// daily 模式：本地日期字符串（YYYY-MM-DD）——该日已跑过。
    last_daily_date: Option<String>,
    /// idle 模式：上次 dream 完成时间（RFC3339 UTC）。仅当上次 dream 早于
    /// 最近一次活动时才允许再跑。
    last_dream_at: Option<String>,
}

fn state_path(data_dir: Option<&PathBuf>) -> Option<PathBuf> {
    data_dir.map(|dir| dir.join("connect_wechat_dream_state.json"))
}

async fn load_state(path: Option<&PathBuf>) -> WechatDreamState {
    let Some(path) = path else {
        return WechatDreamState::default();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(
                "connect: wechat dream state at {path:?} is corrupt, starting empty: {error}"
            );
            WechatDreamState::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => WechatDreamState::default(),
        Err(error) => {
            tracing::warn!("connect: failed to read wechat dream state at {path:?}: {error}");
            WechatDreamState::default()
        }
    }
}

/// 与 bridge 会话映射同款的原子写（临时文件 + rename）。
async fn persist_state(path: Option<&PathBuf>, state: &WechatDreamState) {
    let Some(path) = path else { return };
    let json = match serde_json::to_vec_pretty(state) {
        Ok(json) => json,
        Err(error) => {
            tracing::warn!("connect: failed to serialize wechat dream state: {error}");
            return;
        }
    };
    let temp = path.with_extension("json.tmp");
    let write = async {
        let mut file = tokio::fs::File::create(&temp).await?;
        file.write_all(&json).await?;
        file.sync_all().await
    };
    if let Err(error) = write.await {
        tracing::warn!("connect: failed to persist wechat dream state at {path:?}: {error}");
        return;
    }
    if let Err(error) = tokio::fs::rename(&temp, path).await {
        tracing::warn!("connect: failed to swap wechat dream state at {path:?}: {error}");
    }
}

/// `HH:MM`（24h）-> 当天的本地时间点；非法输入回退 None。
fn parse_daily_at(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.trim().splitn(2, ':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    if hour < 24 && minute < 60 {
        Some((hour, minute))
    } else {
        None
    }
}

/// 启动微信 Project Dream 计划任务。常驻但惰性：没有 wechat 条目或
/// `dream.mode = "off"` 时每轮 tick 空转直接返回。
pub fn spawn_wechat_dream_task(
    bridge: Arc<ConnectBridge>,
    dream_ctx: AutoDreamContext,
    project_store: Arc<bamboo_projects::ProjectStore>,
    data_dir: Option<PathBuf>,
) {
    tokio::spawn(async move {
        let state_file = state_path(data_dir.as_ref());
        let mut state = load_state(state_file.as_ref()).await;
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // interval 的第一次 tick 立即完成：跳过它，避免启动瞬间抢跑。
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Some(updated) =
                run_one_tick(&bridge, &dream_ctx, &project_store, &mut state).await
            {
                if updated {
                    persist_state(state_file.as_ref(), &state).await;
                }
            }
        }
    });
}

/// 单轮判定。返回 `Some(true)` = 状态有变化需要落盘；`Some(false)` = 无
/// 动作；`None` = 配置关闭（本轮无事可做）。
async fn run_one_tick(
    bridge: &Arc<ConnectBridge>,
    dream_ctx: &AutoDreamContext,
    project_store: &Arc<bamboo_projects::ProjectStore>,
    state: &mut WechatDreamState,
) -> Option<bool> {
    let (dream_cfg, explicit_project_id) = {
        let config = dream_ctx.config.read().await;
        let wechat = config
            .connect
            .platforms
            .iter()
            .find(|platform| platform.platform_type == "wechat")?;
        (
            wechat.dream.clone().unwrap_or_default(),
            wechat.project_id.clone(),
        )
    };
    let dream = dream_cfg;
    let mode = dream.effective_mode();
    if mode == "off" {
        return None;
    }

    // 项目解析：显式配置优先；否则按自动项目名找活跃项目（不创建）。
    let auto_name = {
        let config = dream_ctx.config.read().await;
        config
            .connect
            .platforms
            .iter()
            .find(|platform| platform.platform_type == "wechat")
            .and_then(|platform| platform.auto_project.clone())
            .unwrap_or_default()
            .effective_name()
            .to_string()
    };
    let project_id = explicit_project_id.clone().or_else(|| {
        project_store
            .list()
            .ok()
            .and_then(|projects| {
                projects.into_iter().find(|project| {
                    project.name == auto_name
                        && project.status == bamboo_domain::ProjectStatus::Active
                })
            })
            .map(|project| project.id)
    });
    let Some(project_id) = project_id else {
        return Some(false); // 项目还没被首条消息路径创建——无事可做
    };

    let should_run = match mode {
        "idle" => {
            let idle_minutes = dream.effective_idle_minutes();
            let last_dream = state
                .last_dream_at
                .as_deref()
                .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
                .map(|parsed| parsed.with_timezone(&Utc));
            match bridge.platform_last_activity("wechat") {
                // 还没有任何入站活动：没有可 dream 的新素材。
                None => false,
                Some(last_activity) => {
                    // 上次 dream 必须早于最近一次活动（否则已为这轮活动
                    // dream 过），且现在已越过闲置阈值。
                    let dreamed_before_this_activity = match last_dream {
                        None => true,
                        Some(dream_at) => dream_at < last_activity,
                    };
                    dreamed_before_this_activity
                        && Utc::now()
                            >= last_activity + chrono::Duration::minutes(idle_minutes as i64)
                }
            }
        }
        _ => {
            // daily：本地时间越过当天目标时刻，且今天还没跑过。
            let (hour, minute) = parse_daily_at(dream.effective_daily_at()).unwrap_or((3, 0));
            let now_local = Local::now();
            let today = now_local.format("%Y-%m-%d").to_string();
            let due = now_local.hour() > hour
                || (now_local.hour() == hour && now_local.minute() >= minute);
            due && state.last_daily_date.as_deref() != Some(today.as_str())
        }
    };
    if !should_run {
        return Some(false);
    }

    tracing::info!(
        target: "bamboo.memory",
        project_id = %project_id,
        mode,
        "connect: running scheduled wechat Project Dream"
    );
    match run_project_auto_dream_once_for_project(dream_ctx, &project_id).await {
        Ok(result) => {
            if let Some(result) = result {
                tracing::info!(
                    target: "bamboo.memory",
                    project_id = %project_id,
                    used_model = %result.used_model,
                    session_count = result.session_count,
                    "connect: scheduled wechat Project Dream completed"
                );
            } else {
                tracing::info!(
                    target: "bamboo.memory",
                    project_id = %project_id,
                    "connect: scheduled wechat Project Dream skipped (no update needed)"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                target: "bamboo.memory",
                project_id = %project_id,
                "connect: scheduled wechat Project Dream failed: {error}"
            );
        }
    }
    // 无论成功失败都记账：失败的项目 dream 每天重试一次（daily）或等下
    // 一轮活动+闲置（idle），不无限打转。
    match mode {
        "idle" => state.last_dream_at = Some(Utc::now().to_rfc3339()),
        _ => state.last_daily_date = Some(Local::now().format("%Y-%m-%d").to_string()),
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_daily_at_accepts_hh_mm_and_rejects_garbage() {
        assert_eq!(parse_daily_at("03:00"), Some((3, 0)));
        assert_eq!(parse_daily_at("23:59"), Some((23, 59)));
        // 运行时解析宽松：单位数小时也接受（"3:00" 等价 03:00）；严格的
        // 两位 HH:MM 由配置写入路径的 validate() 把关。
        assert_eq!(parse_daily_at("3:00"), Some((3, 0)));
        assert_eq!(parse_daily_at("24:00"), None);
        assert_eq!(parse_daily_at(""), None);
        assert_eq!(parse_daily_at("03:60"), None);
    }

    #[test]
    fn dream_defaults_are_daily_at_three() {
        let dream = bamboo_config::WechatDreamConfig::default();
        assert_eq!(dream.effective_mode(), "daily");
        assert_eq!(dream.effective_daily_at(), "03:00");
        assert_eq!(dream.effective_idle_minutes(), 30);
    }

    #[test]
    fn dream_validation_rejects_bad_mode_time_and_idle() {
        let mut dream = bamboo_config::WechatDreamConfig::default();
        assert!(dream.validate().is_ok());

        dream.mode = Some("sometimes".to_string());
        assert!(dream.validate().is_err());

        dream.mode = Some("idle".to_string());
        dream.daily_at = Some("3am".to_string());
        assert!(dream.validate().is_err());

        dream.daily_at = Some("03:00".to_string());
        dream.idle_minutes = Some(0);
        assert!(dream.validate().is_err());

        dream.idle_minutes = Some(1441);
        assert!(dream.validate().is_err());

        dream.idle_minutes = Some(45);
        assert!(dream.validate().is_ok());
    }

    #[test]
    fn auto_project_defaults_and_validation() {
        let auto = bamboo_config::WechatAutoProjectConfig::default();
        assert!(auto.effective_enabled());
        assert_eq!(auto.effective_name(), "微信工作");
        assert_eq!(auto.effective_workspace(), "work_wechat");
        assert!(auto.validate().is_ok());

        let mut auto = bamboo_config::WechatAutoProjectConfig {
            workspace: Some("nested/dir".to_string()),
            ..Default::default()
        };
        assert!(auto.validate().is_err());
        auto.workspace = Some("..\\escape".to_string());
        assert!(auto.validate().is_err());
        auto.workspace = Some("work_wechat".to_string());
        assert!(auto.validate().is_ok());
        // 绝对路径合法：默认位置被现有项目占住时的显式逃生口。
        auto.workspace = Some("D:\\work_wechat".to_string());
        assert!(auto.validate().is_ok());
        auto.workspace = Some("/srv/work_wechat".to_string());
        assert!(auto.validate().is_ok());
        // 带分隔符的相对路径仍然非法。
        auto.workspace = Some("some/relative/path".to_string());
        assert!(auto.validate().is_err());
    }
}
