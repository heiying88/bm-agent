use actix_web::{web, HttpResponse, Result};

use crate::app_state::AppState;
use bamboo_agent_core::{Session, Storage};
use bamboo_engine::auto_dream::{
    run_project_auto_dream_once_for_project, run_project_auto_dream_once_for_session,
    AutoDreamContext,
};
use bamboo_engine::project_context::ProjectContextResolver;
use bamboo_memory::memory_store::MemoryStore;
use bamboo_storage::{CleanupMode, CleanupResult};

use super::super::types::CleanupRequest;

/// `POST /api/v1/sessions/{session_id}/clear`
pub async fn clear_session(
    state: web::Data<AppState>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let _persistence_guard = state.persistence.acquire_lock(&session_id).await;
    let cleared = state
        .session_store
        .clear_session(&session_id)
        .await
        .map_err(|error| {
            crate::error::json_internal_server_error(format!("Failed to clear session: {error}"))
        })?;

    if !cleared {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": crate::error::error_value("Session not found"),
            "session_id": session_id
        })));
    }
    let _ = state.browser.close(&session_id).await;

    // History/chat treat the process cache as authoritative while no runner is
    // active. Replace it with the just-cleared durable snapshot before
    // publishing SessionCleared; otherwise the old transcript can remain
    // visible and a later cache-based save can write it back to disk. The
    // persistence guard stays held across clear, reload, cache publication, and
    // the account-feed event, preserving one process-local mutation boundary.
    // Evict first so a rare post-clear reload failure cannot leave the stale
    // pre-clear transcript authoritative in memory. A successful reload below
    // immediately installs the replacement.
    state.sessions.remove(&session_id);
    let cleared_session = state
        .session_store
        .load_session(&session_id)
        .await
        .map_err(|error| {
            crate::error::json_internal_server_error(format!(
                "Failed to reload cleared session: {error}"
            ))
        })?
        .ok_or_else(|| {
            crate::error::json_internal_server_error(
                "Cleared session disappeared before cache publication",
            )
        })?;
    state.sessions.insert(
        session_id.clone(),
        std::sync::Arc::new(bamboo_engine::SessionSnapshot::new(cleared_session)),
    );

    // Publish onto the account change feed so other clients drop their cached
    // messages for this session and refetch lazily.
    state.account_sink.record(
        Some(&session_id),
        &bamboo_agent_core::AgentEvent::SessionCleared {
            session_id: session_id.clone(),
        },
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "session_id": session_id
    })))
}

async fn load_session_from_state_or_storage(
    state: &AppState,
    session_id: &str,
) -> Result<Option<Session>> {
    let in_memory = { bamboo_engine::read_cached_session(&state.sessions, session_id) };
    if in_memory.is_some() {
        return Ok(in_memory);
    }

    state
        .storage
        .load_session(session_id)
        .await
        .map_err(|error| {
            crate::error::json_internal_server_error(format!("Failed to load session: {error}"))
        })
}

fn dream_memory_store(state: &AppState) -> MemoryStore {
    state.memory_store.clone()
}

// ---------------------------------------------------------------------------
// 手动 Project Dream 的任务注册表：进程内存态（与扫码登录的
// `QR_CONFIRMED_TOKENS` 同款取舍）。服务重启后未完成的 job 查询返回
// 404，前端据此提示；同一项目同时只允许一场在跑（防手动+定时叠加）。
// 已完结的 job 保留 30 分钟供查询，总量封顶防泄漏。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
enum ProjectDreamJobStatus {
    Running,
    Succeeded {
        dream_generated: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        used_model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        generated_at: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_generation: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notebook_chars: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProjectDreamJob {
    job_id: String,
    session_id: String,
    /// `project`（整合项目全部候选会话）| `session`（仅触发会话本身）。
    scope: String,
    project_id: bamboo_domain::ProjectId,
    started_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    state: ProjectDreamJobStatus,
}

/// 已完结 job 的保留窗口；过期条目在下一次插入/查询时清扫。
const PROJECT_DREAM_JOB_RETENTION: chrono::TimeDelta = chrono::TimeDelta::minutes(30);
/// 注册表总量上限（到达后优先丢弃最旧的已完结条目）。
const PROJECT_DREAM_JOB_CAP: usize = 200;

fn project_dream_jobs(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, ProjectDreamJob>> {
    static JOBS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, ProjectDreamJob>>,
    > = std::sync::OnceLock::new();
    JOBS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 清扫过期/超量的已完结 job（调用方持有锁时调用）。
fn prune_project_dream_jobs(jobs: &mut std::collections::HashMap<String, ProjectDreamJob>) {
    let now = chrono::Utc::now();
    jobs.retain(|_, job| match job.finished_at {
        None => true,
        Some(finished_at) => now.signed_duration_since(finished_at) < PROJECT_DREAM_JOB_RETENTION,
    });
    while jobs.len() > PROJECT_DREAM_JOB_CAP {
        let oldest = jobs
            .iter()
            .filter(|(_, job)| job.finished_at.is_some())
            .min_by_key(|(_, job)| job.finished_at)
            .map(|(id, _)| id.clone());
        match oldest {
            Some(id) => {
                jobs.remove(&id);
            }
            None => break,
        }
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectDreamRunRequest {
    /// `project`（默认）整合项目全部候选会话；`session` 仅整合触发会话
    /// 本身（含其子会话），其余会话留给下一次整项目 Dream。
    #[serde(default)]
    scope: String,
}

/// `POST /api/v1/sessions/{session_id}/project-dream/run` — 启动一场
/// Project Dream 并立即返回（202 + job_id）；结果通过
/// `GET .../project-dream/run/{job_id}` 轮询。一场 Dream 要加载项目会话、
/// 拼提示词、流式跑完整 LLM 生成再写快照——模型慢或会话多时轻松超过
/// 反向代理（frp 等）的网关超时，所以不在请求内 await。
pub async fn run_project_dream(
    state: web::Data<AppState>,
    path: web::Path<String>,
    payload: Option<web::Json<ProjectDreamRunRequest>>,
) -> Result<HttpResponse> {
    let session_id = path.into_inner();
    let Some(session) = load_session_from_state_or_storage(&state, &session_id).await? else {
        return Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": crate::error::error_value("Session not found"),
            "session_id": session_id
        })));
    };

    let scope = payload
        .map(|body| body.into_inner().scope)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !matches!(scope.as_str(), "" | "project" | "session") {
        return Ok(HttpResponse::BadRequest().json(serde_json::json!({
            "error": crate::error::error_value(
                "scope must be \"project\" or \"session\"",
            ),
            "session_id": session_id
        })));
    }
    let scope = if scope.is_empty() {
        "project".to_string()
    } else {
        scope
    };

    let project_id = match ProjectContextResolver::session_project_identity(&session) {
        bamboo_engine::project_context::SessionProjectIdentity::Assigned(project_id) => project_id,
        bamboo_engine::project_context::SessionProjectIdentity::Unassigned => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": crate::error::error_value(
                    "Project Dream writes require an assigned Project"
                ),
                "session_id": session_id
            })));
        }
        bamboo_engine::project_context::SessionProjectIdentity::Invalid { raw, message } => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": crate::error::error_value(format!(
                    "Session carries an invalid Project identity '{raw}': {message}"
                )),
                "session_id": session_id
            })));
        }
    };

    let ctx = AutoDreamContext {
        session_store: state.session_store.clone(),
        storage: state.storage.clone(),
        memory: dream_memory_store(&state),
        provider: state.get_provider().await,
        config: state.config.clone(),
        provider_registry: state.provider_registry.clone(),
    };

    let job_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now();
    {
        let mut jobs = project_dream_jobs()
            .lock()
            .expect("project dream jobs lock");
        prune_project_dream_jobs(&mut jobs);
        // 同项目单飞：已有在跑的场次（手动或定时触发）就直接复用它。
        if let Some(running) = jobs.values().find(|job| {
            job.project_id == project_id && matches!(job.state, ProjectDreamJobStatus::Running)
        }) {
            return Ok(HttpResponse::Accepted().json(serde_json::json!({
                "success": true,
                "status": "already_running",
                "job_id": running.job_id,
                "session_id": running.session_id,
                "project_id": running.project_id,
                "started_at": running.started_at,
            })));
        }
        jobs.insert(
            job_id.clone(),
            ProjectDreamJob {
                job_id: job_id.clone(),
                session_id: session_id.clone(),
                scope: scope.clone(),
                project_id: project_id.clone(),
                started_at,
                finished_at: None,
                state: ProjectDreamJobStatus::Running,
            },
        );
    }

    tracing::info!(
        target: "bamboo.memory",
        job_id = %job_id,
        project_id = %project_id,
        "project dream run accepted (async)"
    );
    let task_job_id = job_id.clone();
    let task_project_id = project_id.clone();
    let task_scope = scope.clone();
    let task_session_id = session_id.clone();
    tokio::spawn(async move {
        let outcome = if task_scope == "session" {
            run_project_auto_dream_once_for_session(&ctx, &task_project_id, &task_session_id).await
        } else {
            run_project_auto_dream_once_for_project(&ctx, &task_project_id).await
        };
        let finished_at = chrono::Utc::now();
        let state = match outcome {
            Ok(Some(result)) => ProjectDreamJobStatus::Succeeded {
                dream_generated: true,
                used_model: Some(result.used_model),
                session_count: Some(result.session_count),
                generated_at: Some(result.generated_at),
                source_generation: Some(result.source_generation),
                notebook_chars: Some(result.notebook_chars),
                message: None,
            },
            Ok(None) => ProjectDreamJobStatus::Succeeded {
                dream_generated: false,
                used_model: None,
                session_count: None,
                generated_at: None,
                source_generation: None,
                notebook_chars: None,
                message: Some("No project Dream update was needed".to_string()),
            },
            Err(error) => {
                tracing::warn!(
                    target: "bamboo.memory",
                    job_id = %task_job_id,
                    project_id = %task_project_id,
                    "project dream run failed: {error}"
                );
                ProjectDreamJobStatus::Failed {
                    error: error.to_string(),
                }
            }
        };
        let mut jobs = project_dream_jobs()
            .lock()
            .expect("project dream jobs lock");
        prune_project_dream_jobs(&mut jobs);
        if let Some(job) = jobs.get_mut(&task_job_id) {
            job.finished_at = Some(finished_at);
            job.state = state;
        }
    });

    Ok(HttpResponse::Accepted().json(serde_json::json!({
        "success": true,
        "status": "running",
        "job_id": job_id,
        "session_id": session_id,
        "project_id": project_id,
    })))
}

/// `GET /api/v1/sessions/{session_id}/project-dream/run/{job_id}` — 查询
/// 一场 Project Dream 的执行状态（`running` / `succeeded` / `failed`），
/// 完成时带出与旧同步响应同名的结果字段。
pub async fn get_project_dream_status(
    _state: web::Data<AppState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse> {
    let (_session_id, job_id) = path.into_inner();
    let jobs = project_dream_jobs()
        .lock()
        .expect("project dream jobs lock");
    match jobs.get(&job_id) {
        Some(job) => {
            let mut body = serde_json::json!({
                "success": true,
                "job_id": job.job_id,
                "session_id": job.session_id,
                "project_id": job.project_id,
                "scope": job.scope,
                "started_at": job.started_at,
            });
            if let Some(finished_at) = job.finished_at {
                body["finished_at"] = serde_json::json!(finished_at);
            }
            // tagged 状态体（"status" + 各分支字段）摊平进响应顶层。
            if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(&job.state) {
                for (key, value) in fields {
                    body[key] = value;
                }
            }
            Ok(HttpResponse::Ok().json(body))
        }
        None => Ok(HttpResponse::NotFound().json(serde_json::json!({
            "error": crate::error::error_value("Project Dream job not found"),
            "job_id": job_id
        }))),
    }
}

/// `POST /api/v1/sessions/cleanup`
pub async fn cleanup_sessions(
    state: web::Data<AppState>,
    req: web::Json<CleanupRequest>,
) -> Result<HttpResponse> {
    let mode = match req.mode.trim().to_ascii_lowercase().as_str() {
        "all" => CleanupMode::All,
        "empty" => CleanupMode::Empty,
        "children" => CleanupMode::Children,
        other => {
            return Ok(HttpResponse::BadRequest().json(serde_json::json!({
                "error": crate::error::error_value("Invalid cleanup mode"),
                "mode": other
            })));
        }
    };

    let result: CleanupResult = state
        .session_store
        .cleanup(mode, req.keep_pinned)
        .await
        .map_err(|error| {
            crate::error::json_internal_server_error(format!("Cleanup failed: {error}"))
        })?;

    if !result.deleted_session_ids.is_empty() {
        for session_id in &result.deleted_session_ids {
            let _ = state.browser.retire(session_id).await;
        }
        // Best-effort cancel any in-flight executions.
        {
            let mut runners = state.agent_runners.write().await;
            for session_id in &result.deleted_session_ids {
                if let Some(runner) =
                    bamboo_engine::runtime::execution::runner_lifecycle::remove_runner_entry(
                        &mut runners,
                        session_id,
                    )
                    .await
                {
                    runner.cancel_token.cancel();
                }
            }
        }
        {
            let mut tokens = state.cancel_tokens.write().await;
            for session_id in &result.deleted_session_ids {
                if let Some(token) = tokens.remove(session_id) {
                    token.cancel();
                }
            }
        }
        for session_id in &result.deleted_session_ids {
            state.sessions.remove(session_id);
        }
        {
            let mut senders = state.session_event_senders.write().await;
            for session_id in &result.deleted_session_ids {
                senders.remove(session_id);
            }
        }
        // Publish onto the account change feed so other clients drop the
        // cleaned-up sessions from their list.
        for session_id in &result.deleted_session_ids {
            state.account_sink.record(
                Some(session_id),
                &bamboo_agent_core::AgentEvent::SessionDeleted {
                    session_id: session_id.clone(),
                },
            );
        }
    }

    Ok(HttpResponse::Ok().json(result))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use actix_web::{http::StatusCode, test, web, App};
    use async_trait::async_trait;
    use futures::stream;
    use serde_json::Value;
    use tempfile::tempdir;

    use super::*;
    use crate::routes::configure_routes;
    use crate::AppState;
    use bamboo_agent_core::{ConversationSummary, Message};
    use bamboo_llm::Config;
    use bamboo_llm::{LLMChunk, LLMError, LLMProvider, LLMStream};

    #[derive(Clone)]
    struct SequenceProvider {
        responses: Arc<Mutex<Vec<String>>>,
    }

    impl SequenceProvider {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses)),
            }
        }
    }

    #[async_trait]
    impl LLMProvider for SequenceProvider {
        async fn chat_stream(
            &self,
            _messages: &[Message],
            _tools: &[bamboo_agent_core::tools::ToolSchema],
            _max_output_tokens: Option<u32>,
            _model: &str,
        ) -> Result<LLMStream, LLMError> {
            let next = self.responses.lock().expect("lock poisoned").remove(0);
            Ok(Box::pin(stream::iter(vec![
                Ok(LLMChunk::Token(next)),
                Ok(LLMChunk::Done),
            ])))
        }
    }

    async fn build_test_app_state(
        data_dir: std::path::PathBuf,
        provider: Arc<dyn LLMProvider>,
    ) -> web::Data<AppState> {
        let mut config = Config::default();
        *config.memory_mut() = Some(bamboo_config::MemoryConfig {
            background_model: Some("fast-model".to_string()),
            // Disable the background maintenance tickers in tests. L4 made them
            // default-ON, so with a background_model set the AppState builder
            // spawns real auto_dream/gardener tasks whose first tick fires
            // immediately and calls the mock provider — racing the endpoint's
            // own dream call and draining the finite `SequenceProvider` queue,
            // which flakily panics on `remove(0)`. These endpoint tests drive
            // dream generation explicitly (the endpoint runs regardless of the
            // flag), so the background tickers must stay quiet.
            auto_dream_enabled: false,
            gardener_enabled: false,
            dedup_gardener_enabled: false,
            ..bamboo_config::MemoryConfig::default()
        });
        web::Data::new(
            AppState::new_with_provider(data_dir, config, provider)
                .await
                .expect("app state should initialize"),
        )
    }

    #[actix_web::test]
    async fn run_project_dream_endpoint_generates_project_dream_for_session_project() {
        let temp_dir = tempdir().expect("tempdir");
        bamboo_config::paths::init_bamboo_dir(temp_dir.path().to_path_buf());

        let workspace = temp_dir.path().join("workspace-http-project-dream");
        std::fs::create_dir_all(&workspace).expect("workspace dir");

        let provider: Arc<dyn LLMProvider> = Arc::new(SequenceProvider::new(vec![
            "{\"candidates\":[]}".to_string(),
            "## Current durable context\n- HTTP project dream generated\n\n## Cross-session patterns\n- None\n\n## Active threads to remember\n- None\n\n## Stable constraints and preferences\n- None\n\n## Open risks or questions\n- None".to_string(),
        ]));
        let app_state = build_test_app_state(temp_dir.path().to_path_buf(), provider).await;
        let project = app_state
            .project_store
            .create_with_bindings(
                "HTTP Project Dream",
                None,
                vec![bamboo_domain::WorkspaceBinding {
                    path: workspace.to_string_lossy().into_owned(),
                    label: None,
                    git_common_dir: None,
                }],
            )
            .expect("create Project");

        let mut session = bamboo_agent_core::Session::new("session-http-project-dream", "model");
        session.title = "HTTP Project Dream".to_string();
        session.set_project_id_meta(project.id.to_string());
        session.set_workspace_path_meta(workspace.to_string_lossy().into_owned());
        session.conversation_summary = Some(ConversationSummary::new(
            "Project-scoped session for manual dream generation.",
            3,
            120,
        ));
        session.add_message(Message::user("Generate project dream now."));
        app_state
            .storage
            .save_session(&session)
            .await
            .expect("save session");

        // 同项目的第二个会话：scope=session 只应整合触发会话一个来源。
        let mut second = bamboo_agent_core::Session::new("session-http-project-dream-2", "model");
        second.title = "HTTP Project Dream 2".to_string();
        second.set_project_id_meta(project.id.to_string());
        second.set_workspace_path_meta(workspace.to_string_lossy().into_owned());
        second.conversation_summary = Some(ConversationSummary::new(
            "Second project-scoped session that scope=session must skip.",
            3,
            120,
        ));
        second.add_message(Message::user("Second session content."));
        app_state
            .storage
            .save_session(&second)
            .await
            .expect("save second session");
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .configure(configure_routes),
        )
        .await;
        // 新语义：POST 立即 202 + job_id，后台跑完后由状态端点给出结果。
        let req = test::TestRequest::post()
            .uri("/api/v1/sessions/session-http-project-dream/project-dream/run")
            .set_json(serde_json::json!({ "scope": "session" }))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body.get("success").and_then(Value::as_bool), Some(true));
        assert_eq!(body.get("status").and_then(Value::as_str), Some("running"));
        let job_id = body
            .get("job_id")
            .and_then(Value::as_str)
            .expect("job_id")
            .to_string();
        assert_eq!(
            body.get("session_id").and_then(Value::as_str),
            Some("session-http-project-dream")
        );
        assert_eq!(
            body.get("project_id").and_then(Value::as_str),
            Some(project.id.as_str())
        );

        // 轮询状态端点直到终态（后台任务很快：SequenceProvider 即时返回）。
        let status_uri =
            format!("/api/v1/sessions/session-http-project-dream/project-dream/run/{job_id}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let body = loop {
            let poll =
                test::call_service(&app, test::TestRequest::get().uri(&status_uri).to_request())
                    .await;
            assert_eq!(poll.status(), StatusCode::OK);
            let body: Value = test::read_body_json(poll).await;
            match body.get("status").and_then(Value::as_str) {
                Some("succeeded") | Some("failed") => break body,
                _ => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "project dream job never settled"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        };
        assert_eq!(
            body.get("status").and_then(Value::as_str),
            Some("succeeded")
        );
        assert_eq!(
            body.get("dream_generated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.get("used_model").and_then(Value::as_str),
            Some("fast-model")
        );
        assert!(body.get("generated_at").and_then(Value::as_str).is_some());
        assert_eq!(
            body.get("source_generation")
                .and_then(Value::as_str)
                .map(str::len),
            Some(64)
        );
        assert!(body.get("note_path").is_none());

        let memory = app_state.memory_store.for_project(&project.id);
        let project_dream = memory
            .read_dream_snapshot(
                bamboo_memory::memory_store::MemoryScope::Project,
                Some(project.id.as_str()),
            )
            .await
            .expect("read project Dream snapshot")
            .snapshot
            .expect("project dream should exist");
        assert!(project_dream
            .content
            .contains("HTTP project dream generated"));
    }

    #[actix_web::test]
    async fn lifecycle_mutations_publish_change_feed_events() {
        use bamboo_engine::events::journal;

        let temp_dir = tempdir().expect("tempdir");
        bamboo_config::paths::init_bamboo_dir(temp_dir.path().to_path_buf());
        let provider: Arc<dyn LLMProvider> = Arc::new(SequenceProvider::new(vec![]));
        let app_state = build_test_app_state(temp_dir.path().to_path_buf(), provider).await;

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .configure(configure_routes),
        )
        .await;

        // Create a session via the route.
        let create = test::TestRequest::post()
            .uri("/api/v1/sessions")
            .set_json(serde_json::json!({ "title": "Feed test" }))
            .to_request();
        let resp = test::call_service(&app, create).await;
        // POST /sessions now returns 201 Created (#251 finding 3).
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body: Value = test::read_body_json(resp).await;
        let session_id = body["session"]["id"]
            .as_str()
            .expect("session id")
            .to_string();

        // Clear it.
        let clear = test::TestRequest::post()
            .uri(&format!("/api/v1/sessions/{session_id}/clear"))
            .to_request();
        assert_eq!(
            test::call_service(&app, clear).await.status(),
            StatusCode::OK
        );

        // Delete it.
        let delete = test::TestRequest::delete()
            .uri(&format!("/api/v1/sessions/{session_id}"))
            .to_request();
        assert_eq!(
            test::call_service(&app, delete).await.status(),
            StatusCode::OK
        );

        // Let the single writer task drain.
        for _ in 0..100 {
            if app_state.account_sink.latest_seq() >= 3 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        let events = journal::read_since(app_state.account_sink.events_dir(), 0).unwrap();
        let kinds: Vec<&str> = events
            .iter()
            .map(|ce| match &ce.event {
                bamboo_agent_core::AgentEvent::SessionCreated { .. } => "created",
                bamboo_agent_core::AgentEvent::SessionCleared { .. } => "cleared",
                bamboo_agent_core::AgentEvent::SessionDeleted { .. } => "deleted",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["created", "cleared", "deleted"],
            "events: {kinds:?}"
        );
        // All carry the right session id and monotonic seq.
        assert!(events
            .iter()
            .all(|ce| ce.session_id.as_deref() == Some(session_id.as_str())));
        assert_eq!(
            events.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[actix_web::test]
    async fn run_project_dream_endpoint_returns_bad_request_when_project_scope_is_unavailable() {
        let temp_dir = tempdir().expect("tempdir");
        bamboo_config::paths::init_bamboo_dir(temp_dir.path().to_path_buf());

        let provider: Arc<dyn LLMProvider> = Arc::new(SequenceProvider::new(vec![]));
        let app_state = build_test_app_state(temp_dir.path().to_path_buf(), provider).await;

        let mut session = bamboo_agent_core::Session::new("session-no-project-scope", "model");
        session.title = "No Project Scope".to_string();
        session.conversation_summary =
            Some(ConversationSummary::new("No workspace metadata.", 1, 40));
        session.add_message(Message::user(
            "Try to generate project dream without workspace.",
        ));
        app_state
            .storage
            .save_session(&session)
            .await
            .expect("save session");

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .configure(configure_routes),
        )
        .await;
        let req = test::TestRequest::post()
            .uri("/api/v1/sessions/session-no-project-scope/project-dream/run")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body: Value = test::read_body_json(resp).await;
        // Canonical nested error envelope (#251 finding 2), not the old flat
        // `{"error": "<string>"}` shape.
        assert_eq!(body["error"]["type"], "api_error");
        assert_eq!(
            body["error"]["message"].as_str(),
            Some("Project Dream writes require an assigned Project")
        );
        assert_eq!(
            body.get("session_id").and_then(Value::as_str),
            Some("session-no-project-scope")
        );
    }
}
