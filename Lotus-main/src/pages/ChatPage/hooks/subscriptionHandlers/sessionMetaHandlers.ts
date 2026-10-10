import type { AgentEventHandlers } from "@services/chat/AgentService";
import { useAppStore } from "@shared/store/appStore";
import { applyReplayableSessionEvent } from "@shared/store/appStore/slices/sessionMetadataSlice";
import { planModeStateFromEvent } from "../useAgentEventSubscription.helpers";
import type { RunContext } from "../subscriptionContext";

/** Session metadata (title/pinned) + plan-mode handlers. */
export function createSessionMetaHandlers(run: RunContext): Partial<AgentEventHandlers> {
  const { sessionId } = run;
  const { refreshChatsNow, updateSession } = run.ctx;
  return {
    onSessionTitleUpdated: (event) => {
      applyReplayableSessionEvent(event, useAppStore.getState());
    },

    onSessionPinnedUpdated: (event) => {
      applyReplayableSessionEvent(event, useAppStore.getState());
    },

    // 模型/智能度/审批模式切换（网页 PATCH 或微信 /model、/think、SET 标记）
    // 实时推送：直接改当前会话的 config，输入框选择器即刻生效，无需刷新。
    onSessionExecutionProfileUpdated: (event) => {
      const targetSessionId = event.session_id || sessionId;
      const currentSession = useAppStore
        .getState()
        .chats.find((chat) => chat.id === targetSessionId);
      if (!currentSession) {
        void refreshChatsNow();
        return;
      }
      updateSession(targetSessionId, {
        config: {
          ...(currentSession.config || {}),
          model_ref: event.model_ref ?? undefined,
          reasoningEffort: event.reasoning_effort ?? undefined,
          ...(event.permission_mode
            ? { permissionMode: event.permission_mode }
            : {}),
        },
      });
    },

    onPlanModeEntered: (event) => {
      const targetSessionId = event.session_id || sessionId;
      const planMode = planModeStateFromEvent(event);
      if (!planMode) {
        void refreshChatsNow();
        return;
      }
      updateSession(targetSessionId, {
        planMode,
      });
    },

    onPlanModeExited: (event) => {
      const targetSessionId = event.session_id || sessionId;
      updateSession(targetSessionId, {
        planMode: null,
      });
      void refreshChatsNow();
    },

    onGoalStatusChanged: (event) => {
      const targetSessionId = event.session_id || sessionId;
      const goalState = event.goal_state ?? null;
      const currentSession = useAppStore
        .getState()
        .chats.find((chat) => chat.id === targetSessionId);
      if (!currentSession) {
        void refreshChatsNow();
        return;
      }
      updateSession(targetSessionId, {
        config: {
          ...(currentSession.config || {}),
          goalState,
        },
      });
    },

    onPlanFileUpdated: (event) => {
      const targetSessionId = event.session_id || sessionId;
      const currentSession = useAppStore
        .getState()
        .chats.find((chat) => chat.id === targetSessionId);
      const currentPlanMode = currentSession?.planMode;
      if (currentPlanMode) {
        updateSession(targetSessionId, {
          planMode: {
            ...currentPlanMode,
            plan_file_path: event.plan_file_path ?? currentPlanMode.plan_file_path ?? null,
            status:
              event.status === "exploring" ||
              event.status === "designing" ||
              event.status === "reviewing" ||
              event.status === "finalizing" ||
              event.status === "awaiting_approval"
                ? event.status
                : currentPlanMode.status,
          },
        });
      } else {
        void refreshChatsNow();
      }
    },
  };
}
