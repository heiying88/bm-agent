import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Divider,
  Flex,
  Input,
  Select,
  Switch,
  Tag,
  Typography,
  theme,
} from "antd";
import { useTranslation } from "react-i18next";
import { getErrorMessage } from "@services/api";
import {
  type ConnectSection,
  type ConnectSectionDraftPlatform,
  type ConnectSectionPlatform,
  type ConfigSectionEnvelope,
  type CredentialStatusView,
} from "@services/config/configSections";
import { useConfigSectionStore } from "@shared/store/configSectionStore";
import { configSectionsService } from "@services/config/configSections";
import { reapplyConfigChanges } from "@shared/hooks/useConfigSectionDraft";
import { redactConfigError } from "@shared/utils/configErrors";

const { Text } = Typography;
const { useToken } = theme;

interface ConnectDraft {
  telegramEnabled: boolean;
  telegramToken: string;
  telegramAllowFrom: string[];
  feishuEnabled: boolean;
  feishuAppId: string;
  feishuAppSecret: string;
  feishuDomain: string;
  feishuAllowFrom: string[];
  wechatEnabled: boolean;
  wechatToken: string;
  wechatAllowFrom: string[];
  wechatVoiceApiKey: string;
  /** `off` | `mirror` | `always` — gates AUTO voice only (markers stay live). */
  wechatReplyMode: string;
  /** `file` (default, mp3) | `bubble` (native voice, experimental). */
  wechatDelivery: string;
  wechatFileAsr: boolean;
  wechatTtsModel: string;
  wechatTtsVoice: string;
}

const findPlatform = (
  platforms: ConnectSectionPlatform[] | undefined,
  type: string,
): ConnectSectionPlatform | undefined => platforms?.find((p) => p.type === type);

function draftFromConfig(connect: ConnectSection | undefined): ConnectDraft {
  const telegram = findPlatform(connect?.platforms, "telegram");
  const feishu = findPlatform(connect?.platforms, "feishu");
  const wechat = findPlatform(connect?.platforms, "wechat");
  const voice = wechat?.voice;
  return {
    telegramEnabled: Boolean(telegram),
    telegramToken: "",
    telegramAllowFrom: telegram?.allow_from ?? [],
    feishuEnabled: Boolean(feishu),
    feishuAppId: feishu?.app_id ?? "",
    feishuAppSecret: "",
    feishuDomain: feishu?.domain ?? "",
    feishuAllowFrom: feishu?.allow_from ?? [],
    wechatEnabled: Boolean(wechat),
    wechatToken: "",
    wechatAllowFrom: wechat?.allow_from ?? [],
    wechatVoiceApiKey: "",
    wechatReplyMode: voice?.reply_mode ?? "off",
    wechatDelivery: voice?.delivery ?? "file",
    wechatFileAsr: voice?.file_asr === "on",
    wechatTtsModel: voice?.tts_model ?? "",
    wechatTtsVoice: voice?.tts_voice ?? "",
  };
}

const secretFreeDraft = (draft: ConnectDraft) => ({
  ...draft,
  telegramToken: draft.telegramToken ? "[replace requested]" : "",
  feishuAppSecret: draft.feishuAppSecret ? "[replace requested]" : "",
  wechatToken: draft.wechatToken ? "[replace requested]" : "",
  wechatVoiceApiKey: draft.wechatVoiceApiKey ? "[replace requested]" : "",
});

/**
 * Connect / IM-bridge settings: drive Bamboo sessions from Telegram or
 * Feishu/Lark (bamboo epic #447, closes Lotus #49).
 *
 * Reads the typed `connect` section and performs revisioned credential-domain
 * writes. `connect.platforms` is at most one
 * entry per platform `type` in this UI (the backend only ever starts the
 * first configured entry per type — `multi_bot_guard`, bamboo #462) so the
 * form presents one fixed Telegram slot and one fixed Feishu slot rather
 * than a freeform list.
 *
 * Secret inputs are always blank. Unchanged values are omitted, replacement
 * sends plaintext exactly once, and clear sends explicit null. Stable platform
 * ids are preserved so credential ownership cannot drift when entries move.
 *
 * Enabling/disabling a platform in this UI adds/removes its entry from the
 * `connect.platforms` array — `ConnectPlatformConfig` has no `enabled`
 * field on the backend (unlike `notifications.ntfy`/`.bark`); presence in
 * the array is what starts the bridge.
 */
const SystemSettingsConnectTab: React.FC = () => {
  const { t } = useTranslation();
  const { token } = useToken();
  const [connect, setConnect] = useState<ConnectSection | undefined>(undefined);
  const [draft, setDraft] = useState<ConnectDraft>(() => draftFromConfig(undefined));
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [baseSectionRevision, setBaseSectionRevision] = useState<number | null>(null);
  const [dirty, setDirty] = useState(false);
  const [clearTelegramToken, setClearTelegramToken] = useState(false);
  const [clearFeishuSecret, setClearFeishuSecret] = useState(false);
  const [clearWechatToken, setClearWechatToken] = useState(false);
  const [clearWechatVoiceKey, setClearWechatVoiceKey] = useState(false);
  const [qrCode, setQrCode] = useState<{
    qrcodeId: string;
    imageKind: string;
    image: string;
  } | null>(null);
  const [qrPhase, setQrPhase] = useState<"idle" | "loading" | "waiting" | "applying" | "done">(
    "idle",
  );
  const [qrError, setQrError] = useState<string | null>(null);
  const qrPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [showComparison, setShowComparison] = useState(false);
  const baseDraftRef = useRef<ConnectDraft | null>(null);
  const snapshot = useConfigSectionStore((state) => state.sections.connect);
  const loadSection = useConfigSectionStore((state) => state.loadSection);
  const saveConnect = useConfigSectionStore((state) => state.saveConnect);

  const adoptSnapshot = useCallback((envelope: ConfigSectionEnvelope<ConnectSection>) => {
    setConnect(envelope.data);
    const nextDraft = draftFromConfig(envelope.data);
    setDraft(nextDraft);
    baseDraftRef.current = structuredClone(nextDraft);
    setBaseSectionRevision(envelope.revision);
    setDirty(false);
    setClearTelegramToken(false);
    setClearFeishuSecret(false);
    setClearWechatToken(false);
    setClearWechatVoiceKey(false);
    setShowComparison(false);
  }, []);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const envelope = await loadSection("connect", { force: true });
      adoptSnapshot(envelope);
    } catch (error) {
      setLoadError(redactConfigError(getErrorMessage(error)));
    } finally {
      setLoading(false);
    }
  }, [adoptSnapshot, loadSection]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const envelope = snapshot.envelope;
    if (
      !envelope ||
      baseSectionRevision === null ||
      dirty ||
      envelope.revision < baseSectionRevision ||
      envelope.revision === baseSectionRevision
    ) {
      return;
    }
    adoptSnapshot(envelope);
  }, [adoptSnapshot, baseSectionRevision, dirty, snapshot.envelope]);

  const patch = (p: Partial<ConnectDraft>) => {
    setDraft((d) => ({ ...d, ...p }));
    setDirty(true);
  };

  const storedTelegram = findPlatform(connect?.platforms, "telegram");
  const storedFeishu = findPlatform(connect?.platforms, "feishu");
  const storedWechat = findPlatform(connect?.platforms, "wechat");
  const hasStoredTelegramToken =
    storedTelegram?.token_configured ?? Boolean(storedTelegram?.token_credential_ref);
  const hasStoredFeishuSecret =
    storedFeishu?.app_secret_configured ?? Boolean(storedFeishu?.app_secret_credential_ref);
  const hasStoredWechatToken =
    storedWechat?.token_configured ?? Boolean(storedWechat?.token_credential_ref);
  const hasStoredWechatVoiceKey =
    storedWechat?.voice_api_key_configured ??
    Boolean(storedWechat?.voice_api_key_credential_ref);

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      const telegramToken = draft.telegramToken.trim();
      const feishuAppSecret = draft.feishuAppSecret.trim();

      if (baseSectionRevision === null) throw new Error("Connect configuration is not loaded.");

      const buildTelegramEntry = (): ConnectSectionDraftPlatform => ({
        ...(storedTelegram?.id ? { id: storedTelegram.id } : {}),
        ...(storedTelegram?.project_id !== undefined
          ? { project_id: storedTelegram.project_id }
          : {}),
        type: "telegram",
        ...(clearTelegramToken
          ? { token_change: { action: "clear" as const } }
          : telegramToken
            ? {
                token_change: {
                  action: "replace" as const,
                  value: telegramToken,
                },
              }
            : {}),
        allow_from: draft.telegramAllowFrom,
        admin_from: storedTelegram?.admin_from ?? [],
      });

      const buildFeishuEntry = (): ConnectSectionDraftPlatform => ({
        ...(storedFeishu?.id ? { id: storedFeishu.id } : {}),
        ...(storedFeishu?.project_id !== undefined ? { project_id: storedFeishu.project_id } : {}),
        type: "feishu",
        ...(draft.feishuAppId.trim() ? { app_id: draft.feishuAppId.trim() } : {}),
        ...(clearFeishuSecret
          ? { app_secret_change: { action: "clear" as const } }
          : feishuAppSecret
            ? {
                app_secret_change: {
                  action: "replace" as const,
                  value: feishuAppSecret,
                },
              }
            : {}),
        ...(draft.feishuDomain.trim() ? { domain: draft.feishuDomain.trim() } : {}),
        allow_from: draft.feishuAllowFrom,
        admin_from: storedFeishu?.admin_from ?? [],
      });

      const buildWechatEntry = (): ConnectSectionDraftPlatform => ({
        ...(storedWechat?.id ? { id: storedWechat.id } : {}),
        ...(storedWechat?.project_id !== undefined
          ? { project_id: storedWechat.project_id }
          : {}),
        type: "wechat",
        ...(clearWechatToken
          ? { token_change: { action: "clear" as const } }
          : draft.wechatToken.trim()
            ? {
                token_change: {
                  action: "replace" as const,
                  value: draft.wechatToken.trim(),
                },
              }
            : {}),
        ...(clearWechatVoiceKey
          ? { voice_api_key_change: { action: "clear" as const } }
          : draft.wechatVoiceApiKey.trim()
            ? {
                voice_api_key_change: {
                  action: "replace" as const,
                  value: draft.wechatVoiceApiKey.trim(),
                },
              }
            : {}),
        // Submitting `voice` replaces the whole section server-side; blank
        // optional fields fall back to server defaults (same behavior as an
        // absent section value there).
        voice: {
          reply_mode: draft.wechatReplyMode,
          delivery: draft.wechatDelivery,
          file_asr: draft.wechatFileAsr ? "on" : "off",
          ...(draft.wechatTtsModel.trim() ? { tts_model: draft.wechatTtsModel.trim() } : {}),
          ...(draft.wechatTtsVoice.trim() ? { tts_voice: draft.wechatTtsVoice.trim() } : {}),
        },
        allow_from: draft.wechatAllowFrom,
        admin_from: storedWechat?.admin_from ?? [],
      });

      // Preserve original order and stable ids. Untouched/unknown platform
      // types (future adapters) pass through without server-managed metadata.
      const original = connect?.platforms ?? [];
      const platforms: ConnectSectionDraftPlatform[] = [];
      let telegramSeen = false;
      let feishuSeen = false;
      let wechatSeen = false;
      for (const entry of original) {
        if (entry.type === "telegram") {
          telegramSeen = true;
          if (draft.telegramEnabled) platforms.push(buildTelegramEntry());
        } else if (entry.type === "feishu") {
          feishuSeen = true;
          if (draft.feishuEnabled) platforms.push(buildFeishuEntry());
        } else if (entry.type === "wechat") {
          wechatSeen = true;
          if (draft.wechatEnabled) platforms.push(buildWechatEntry());
        } else {
          const sanitized = { ...entry } as ConnectSectionPlatform;
          delete sanitized.token;
          delete sanitized.app_secret;
          delete sanitized.token_configured;
          delete sanitized.token_credential_ref;
          delete sanitized.token_credential;
          delete sanitized.app_secret_configured;
          delete sanitized.app_secret_credential_ref;
          delete sanitized.app_secret_credential;
          delete sanitized.voice_api_key_configured;
          delete sanitized.voice_api_key_credential_ref;
          delete sanitized.voice_api_key_credential;
          platforms.push(sanitized);
        }
      }
      if (draft.telegramEnabled && !telegramSeen) platforms.push(buildTelegramEntry());
      if (draft.feishuEnabled && !feishuSeen) platforms.push(buildFeishuEntry());
      if (draft.wechatEnabled && !wechatSeen) platforms.push(buildWechatEntry());

      const envelope = await saveConnect({ platforms }, baseSectionRevision);
      adoptSnapshot(envelope);
      setSaved(true);
      setTimeout(() => setSaved(false), 2500);
    } catch (error) {
      setSaveError(redactConfigError(getErrorMessage(error)));
    } finally {
      setSaving(false);
    }
  };

  const telegramDenyAll = draft.telegramEnabled && draft.telegramAllowFrom.length === 0;
  const feishuDenyAll = draft.feishuEnabled && draft.feishuAllowFrom.length === 0;
  const wechatDenyAll = draft.wechatEnabled && draft.wechatAllowFrom.length === 0;
  const externalSectionRevision =
    dirty &&
    snapshot.envelope &&
    baseSectionRevision !== null &&
    snapshot.envelope.revision > baseSectionRevision
      ? snapshot.envelope.revision
      : null;
  const hasExternalRevision = externalSectionRevision !== null;
  const comparison =
    showComparison && hasExternalRevision && snapshot.envelope && baseDraftRef.current
      ? JSON.stringify(
          {
            baseSectionRevision,
            latestSectionRevision: snapshot.envelope.revision,
            base: secretFreeDraft(baseDraftRef.current),
            draft: {
              ...secretFreeDraft(draft),
              clearTelegramToken,
              clearFeishuSecret,
              clearWechatToken,
              clearWechatVoiceKey,
            },
            latest: secretFreeDraft(draftFromConfig(snapshot.envelope.data)),
          },
          null,
          2,
        )
      : null;

  const credentialStatus = (status: CredentialStatusView | undefined, configured: boolean) => {
    const hasError = Boolean(snapshot.error) || status?.state === "error";
    const fromEnvironment =
      status?.state === "from_env" ||
      (status?.configured && (status.source === "environment" || status.source === "env"));
    const label = hasError
      ? "Error"
      : fromEnvironment
        ? "From env"
        : (status?.configured ?? configured)
          ? "Configured"
          : "Missing";
    return (
      <Flex align="center" gap={8} wrap="wrap">
        <Tag color={hasError ? "error" : status?.configured || configured ? "success" : "warning"}>
          {label}
        </Tag>
        {fromEnvironment ? (
          <Text type="secondary">
            The environment value is read-only; only an explicit replacement is persisted.
          </Text>
        ) : null}
      </Flex>
    );
  };

  const reapplyLatest = async () => {
    if (!baseDraftRef.current) return;
    setLoadError(null);
    try {
      const envelope = await loadSection("connect", { force: true });
      const latest = draftFromConfig(envelope.data);
      setDraft(reapplyConfigChanges(baseDraftRef.current, draft, latest));
      setConnect(envelope.data);
      baseDraftRef.current = structuredClone(latest);
      setBaseSectionRevision(envelope.revision);
      setDirty(true);
      setShowComparison(false);
    } catch (error) {
      setLoadError(redactConfigError(getErrorMessage(error)));
    }
  };

  const stopQrPolling = useCallback(() => {
    if (qrPollRef.current !== null) {
      clearInterval(qrPollRef.current);
      qrPollRef.current = null;
    }
  }, []);

  useEffect(() => stopQrPolling, [stopQrPolling]);

  const resetQrFlow = useCallback(() => {
    stopQrPolling();
    setQrCode(null);
    setQrPhase("idle");
    setQrError(null);
  }, [stopQrPolling]);

  const applyQrToken = useCallback(
    async (qrcodeId: string) => {
      if (baseSectionRevision === null) {
        setQrError("Connect configuration is not loaded.");
        resetQrFlow();
        return;
      }
      setQrPhase("applying");
      setQrError(null);
      try {
        const envelope = await configSectionsService.wechatQrApply({
          qrcode_id: qrcodeId,
          expected_revision: baseSectionRevision,
          ...(storedWechat?.id ? { id: storedWechat.id } : {}),
          allow_from: draft.wechatAllowFrom,
          admin_from: storedWechat?.admin_from ?? [],
          voice: {
            reply_mode: draft.wechatReplyMode,
            delivery: draft.wechatDelivery,
            file_asr: draft.wechatFileAsr ? "on" : "off",
            ...(draft.wechatTtsModel.trim()
              ? { tts_model: draft.wechatTtsModel.trim() }
              : {}),
            ...(draft.wechatTtsVoice.trim()
              ? { tts_voice: draft.wechatTtsVoice.trim() }
              : {}),
          },
          ...(draft.wechatVoiceApiKey.trim()
            ? {
                voice_api_key_change: {
                  action: "replace" as const,
                  value: draft.wechatVoiceApiKey.trim(),
                },
              }
            : {}),
        });
        stopQrPolling();
        adoptSnapshot(envelope);
        setQrPhase("done");
        setQrCode(null);
      } catch (error) {
        setQrError(redactConfigError(getErrorMessage(error)));
        resetQrFlow();
      }
    },
    [
      adoptSnapshot,
      baseSectionRevision,
      draft,
      resetQrFlow,
      stopQrPolling,
      storedWechat,
    ],
  );

  const startQrLogin = async () => {
    stopQrPolling();
    setQrError(null);
    setQrPhase("loading");
    setQrCode(null);
    try {
      const result = await configSectionsService.wechatQrStart();
      setQrCode({
        qrcodeId: result.qrcode_id,
        imageKind: result.image_kind,
        image: result.image,
      });
      setQrPhase("waiting");
      if (result.image_kind === "url") {
        window.open(result.image, "_blank", "noopener");
      }
      const qrcodeId = result.qrcode_id;
      qrPollRef.current = setInterval(() => {
        void (async () => {
          try {
            const status = await configSectionsService.wechatQrStatus(qrcodeId);
            if (status.status === "confirmed") {
              stopQrPolling();
              await applyQrToken(qrcodeId);
            } else if (["expired", "cancelled", "timeout"].includes(status.status)) {
              stopQrPolling();
              setQrPhase("idle");
              setQrCode(null);
              setQrError(
                `QR login ${status.status} — click the button to request a fresh code.`,
              );
            }
          } catch (error) {
            // 瞬时轮询失败不打断流程（与网关侧 qr_login 同款策略）。
            setQrError(redactConfigError(getErrorMessage(error)));
          }
        })();
      }, 2500);
    } catch (error) {
      setQrError(redactConfigError(getErrorMessage(error)));
      setQrPhase("idle");
    }
  };

  return (
    <Card size="small" className="lotus-settings-card" loading={loading}>
      <Flex vertical gap={token.marginMD}>
        <Text strong>{t("settings.connectTab.title")}</Text>
        <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
          {t("settings.connectTab.description")}
        </Text>

        {loadError ? (
          <Alert
            type="error"
            showIcon
            message={loadError}
            action={
              <Button size="small" onClick={() => void load()}>
                {t("settings.connectTab.retry")}
              </Button>
            }
          />
        ) : null}

        {hasExternalRevision ? (
          <Alert
            type="warning"
            showIcon
            message="Connect configuration changed externally"
            description={`Section r${baseSectionRevision} → r${snapshot.envelope?.revision ?? "?"}. Your draft was preserved.`}
            action={
              <Flex gap={8}>
                <Button size="small" onClick={() => void load()}>
                  Reload
                </Button>
                <Button size="small" onClick={() => setShowComparison((current) => !current)}>
                  Compare
                </Button>
                <Button size="small" onClick={() => void reapplyLatest()}>
                  Reapply
                </Button>
              </Flex>
            }
          />
        ) : null}

        {comparison ? (
          <pre
            data-testid="connect-revision-comparison"
            style={{ maxHeight: 240, overflow: "auto", whiteSpace: "pre-wrap" }}
          >
            {comparison}
          </pre>
        ) : null}

        {!loading && !loadError && (
          <>
            <Divider style={{ margin: `${token.marginXS}px 0` }} />

            <Flex vertical gap={token.marginXS}>
              <Flex align="center" justify="space-between" gap={token.marginSM}>
                <Text>{t("settings.connectTab.telegram.title")}</Text>
                <Switch
                  data-testid="connect-telegram-enabled"
                  checked={draft.telegramEnabled}
                  onChange={(checked) => patch({ telegramEnabled: checked })}
                  aria-label={t("settings.connectTab.telegram.enable")}
                />
              </Flex>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.telegram.token")}
                </Text>
                <Input.Password
                  data-testid="connect-telegram-token"
                  value={draft.telegramToken}
                  onChange={(e) => {
                    setClearTelegramToken(false);
                    patch({ telegramToken: e.target.value });
                  }}
                  placeholder={
                    hasStoredTelegramToken
                      ? t("settings.connectTab.telegram.tokenPlaceholderConfigured")
                      : t("settings.connectTab.telegram.tokenPlaceholderEmpty")
                  }
                />
              </label>
              {credentialStatus(storedTelegram?.token_credential, hasStoredTelegramToken)}
              {hasStoredTelegramToken ? (
                <Button
                  size="small"
                  danger={clearTelegramToken}
                  onClick={() => {
                    setClearTelegramToken((current) => !current);
                    setDraft((current) => ({ ...current, telegramToken: "" }));
                    setDirty(true);
                  }}
                >
                  {clearTelegramToken ? "Token will be cleared" : "Clear configured token"}
                </Button>
              ) : null}
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.telegram.allowFrom")}
                </Text>
                <Select<string[]>
                  data-testid="connect-telegram-allow-from"
                  mode="tags"
                  open={false}
                  tokenSeparators={[",", " "]}
                  value={draft.telegramAllowFrom}
                  onChange={(value) => patch({ telegramAllowFrom: value })}
                  placeholder={t("settings.connectTab.telegram.allowFromPlaceholder")}
                  style={{ width: "100%" }}
                />
              </label>
              {telegramDenyAll ? (
                <Text type="warning" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.denyAllWarning")}
                </Text>
              ) : null}
            </Flex>

            <Divider style={{ margin: `${token.marginXS}px 0` }} />

            <Flex vertical gap={token.marginXS}>
              <Flex align="center" justify="space-between" gap={token.marginSM}>
                <Text>{t("settings.connectTab.feishu.title")}</Text>
                <Switch
                  data-testid="connect-feishu-enabled"
                  checked={draft.feishuEnabled}
                  onChange={(checked) => patch({ feishuEnabled: checked })}
                  aria-label={t("settings.connectTab.feishu.enable")}
                />
              </Flex>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.feishu.appId")}
                </Text>
                <Input
                  data-testid="connect-feishu-app-id"
                  value={draft.feishuAppId}
                  onChange={(e) => patch({ feishuAppId: e.target.value })}
                  placeholder={t("settings.connectTab.feishu.appIdPlaceholder")}
                />
              </label>
              {hasStoredFeishuSecret ? (
                <Button
                  size="small"
                  danger={clearFeishuSecret}
                  onClick={() => {
                    setClearFeishuSecret((current) => !current);
                    setDraft((current) => ({ ...current, feishuAppSecret: "" }));
                    setDirty(true);
                  }}
                >
                  {clearFeishuSecret ? "Secret will be cleared" : "Clear configured secret"}
                </Button>
              ) : null}
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.feishu.appSecret")}
                </Text>
                <Input.Password
                  data-testid="connect-feishu-app-secret"
                  value={draft.feishuAppSecret}
                  onChange={(e) => {
                    setClearFeishuSecret(false);
                    patch({ feishuAppSecret: e.target.value });
                  }}
                  placeholder={
                    hasStoredFeishuSecret
                      ? t("settings.connectTab.feishu.appSecretPlaceholderConfigured")
                      : t("settings.connectTab.feishu.appSecretPlaceholderEmpty")
                  }
                />
              </label>
              {credentialStatus(storedFeishu?.app_secret_credential, hasStoredFeishuSecret)}
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.feishu.domain")}
                </Text>
                <Input
                  data-testid="connect-feishu-domain"
                  value={draft.feishuDomain}
                  onChange={(e) => patch({ feishuDomain: e.target.value })}
                  placeholder={t("settings.connectTab.feishu.domainPlaceholder")}
                />
              </label>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.feishu.allowFrom")}
                </Text>
                <Select<string[]>
                  data-testid="connect-feishu-allow-from"
                  mode="tags"
                  open={false}
                  tokenSeparators={[",", " "]}
                  value={draft.feishuAllowFrom}
                  onChange={(value) => patch({ feishuAllowFrom: value })}
                  placeholder={t("settings.connectTab.feishu.allowFromPlaceholder")}
                  style={{ width: "100%" }}
                />
              </label>
              {feishuDenyAll ? (
                <Text type="warning" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.denyAllWarning")}
                </Text>
              ) : null}
            </Flex>

            <Divider style={{ margin: `${token.marginXS}px 0` }} />

            <Flex vertical gap={token.marginXS}>
              <Flex align="center" justify="space-between" gap={token.marginSM}>
                <Text>{t("settings.connectTab.wechat.title")}</Text>
                <Switch
                  data-testid="connect-wechat-enabled"
                  checked={draft.wechatEnabled}
                  onChange={(checked) => patch({ wechatEnabled: checked })}
                  aria-label={t("settings.connectTab.wechat.enable")}
                />
              </Flex>

              <Divider style={{ margin: `${token.marginXS}px 0` }} />
              <Flex align="center" justify="space-between" gap={token.marginSM} wrap="wrap">
                <Text style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.qrTitle")}
                </Text>
                <Button
                  size="small"
                  data-testid="connect-wechat-qr-start"
                  loading={qrPhase === "loading"}
                  disabled={qrPhase === "applying"}
                  onClick={() => void startQrLogin()}
                >
                  {qrPhase === "waiting"
                    ? t("settings.connectTab.wechat.qrRestart")
                    : t("settings.connectTab.wechat.qrStart")}
                </Button>
              </Flex>
              <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                {t("settings.connectTab.wechat.qrHint")}
              </Text>
              {qrPhase === "waiting" && qrCode ? (
                qrCode.imageKind === "png_base64" ? (
                  <img
                    data-testid="connect-wechat-qr-image"
                    src={`data:image/png;base64,${qrCode.image}`}
                    alt="WeChat login QR"
                    style={{ width: 220, height: 220, imageRendering: "pixelated" }}
                  />
                ) : qrCode.imageKind === "url" ? (
                  <Button
                    size="small"
                    href={qrCode.image}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    {t("settings.connectTab.wechat.qrOpenPage")}
                  </Button>
                ) : null
              ) : null}
              {qrPhase === "waiting" ? (
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.qrWaiting")}
                </Text>
              ) : null}
              {qrPhase === "applying" ? (
                <Text type="warning" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.qrApplying")}
                </Text>
              ) : null}
              {qrPhase === "done" ? (
                <Text type="success" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.qrDone")}
                </Text>
              ) : null}
              {qrError ? (
                <Alert type="error" showIcon message={qrError} closable onClose={() => setQrError(null)} />
              ) : null}

              <Divider style={{ margin: `${token.marginXS}px 0` }} />
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.token")}
                </Text>
                <Input.Password
                  data-testid="connect-wechat-token"
                  value={draft.wechatToken}
                  onChange={(e) => {
                    setClearWechatToken(false);
                    patch({ wechatToken: e.target.value });
                  }}
                  placeholder={
                    hasStoredWechatToken
                      ? t("settings.connectTab.wechat.tokenPlaceholderConfigured")
                      : t("settings.connectTab.wechat.tokenPlaceholderEmpty")
                  }
                />
              </label>
              {credentialStatus(storedWechat?.token_credential, hasStoredWechatToken)}
              {hasStoredWechatToken ? (
                <Button
                  size="small"
                  danger={clearWechatToken}
                  onClick={() => {
                    setClearWechatToken((current) => !current);
                    setDraft((current) => ({ ...current, wechatToken: "" }));
                    setDirty(true);
                  }}
                >
                  {clearWechatToken
                    ? t("settings.connectTab.wechat.tokenClearArmed")
                    : t("settings.connectTab.wechat.tokenClear")}
                </Button>
              ) : null}
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.allowFrom")}
                </Text>
                <Select<string[]>
                  data-testid="connect-wechat-allow-from"
                  mode="tags"
                  open={false}
                  tokenSeparators={[",", " "]}
                  value={draft.wechatAllowFrom}
                  onChange={(value) => patch({ wechatAllowFrom: value })}
                  placeholder={t("settings.connectTab.wechat.allowFromPlaceholder")}
                  style={{ width: "100%" }}
                />
              </label>
              <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                {t("settings.connectTab.wechat.allowFromAutoTrustHint")}
              </Text>
              {wechatDenyAll ? (
                <Text type="warning" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.denyAllWarning")}
                </Text>
              ) : null}

              <Divider style={{ margin: `${token.marginXS}px 0` }} />
              <Text style={{ fontSize: token.fontSizeSM }}>
                {t("settings.connectTab.wechat.voiceTitle")}
              </Text>
              <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                {t("settings.connectTab.wechat.voiceDescription")}
              </Text>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.voiceApiKey")}
                </Text>
                <Input.Password
                  data-testid="connect-wechat-voice-key"
                  value={draft.wechatVoiceApiKey}
                  onChange={(e) => {
                    setClearWechatVoiceKey(false);
                    patch({ wechatVoiceApiKey: e.target.value });
                  }}
                  placeholder={
                    hasStoredWechatVoiceKey
                      ? t("settings.connectTab.wechat.voiceKeyPlaceholderConfigured")
                      : t("settings.connectTab.wechat.voiceKeyPlaceholderEmpty")
                  }
                />
              </label>
              {credentialStatus(storedWechat?.voice_api_key_credential, hasStoredWechatVoiceKey)}
              {hasStoredWechatVoiceKey ? (
                <Button
                  size="small"
                  danger={clearWechatVoiceKey}
                  onClick={() => {
                    setClearWechatVoiceKey((current) => !current);
                    setDraft((current) => ({ ...current, wechatVoiceApiKey: "" }));
                    setDirty(true);
                  }}
                >
                  {clearWechatVoiceKey
                    ? t("settings.connectTab.wechat.voiceKeyClearArmed")
                    : t("settings.connectTab.wechat.voiceKeyClear")}
                </Button>
              ) : null}
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.replyMode")}
                </Text>
                <Select<string>
                  data-testid="connect-wechat-reply-mode"
                  value={draft.wechatReplyMode}
                  onChange={(value) => patch({ wechatReplyMode: value })}
                  style={{ width: "100%" }}
                  options={[
                    { value: "off", label: t("settings.connectTab.wechat.replyModeOff") },
                    { value: "mirror", label: t("settings.connectTab.wechat.replyModeMirror") },
                    { value: "always", label: t("settings.connectTab.wechat.replyModeAlways") },
                  ]}
                />
              </label>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.delivery")}
                </Text>
                <Select<string>
                  data-testid="connect-wechat-delivery"
                  value={draft.wechatDelivery}
                  onChange={(value) => patch({ wechatDelivery: value })}
                  style={{ width: "100%" }}
                  options={[
                    { value: "file", label: t("settings.connectTab.wechat.deliveryFile") },
                    { value: "bubble", label: t("settings.connectTab.wechat.deliveryBubble") },
                  ]}
                />
              </label>
              <Flex align="center" justify="space-between" gap={token.marginSM}>
                <Flex vertical>
                  <Text style={{ fontSize: token.fontSizeSM }}>
                    {t("settings.connectTab.wechat.fileAsr")}
                  </Text>
                  <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                    {t("settings.connectTab.wechat.fileAsrDescription")}
                  </Text>
                </Flex>
                <Switch
                  data-testid="connect-wechat-file-asr"
                  checked={draft.wechatFileAsr}
                  onChange={(checked) => patch({ wechatFileAsr: checked })}
                  aria-label={t("settings.connectTab.wechat.fileAsr")}
                />
              </Flex>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.ttsModel")}
                </Text>
                <Input
                  data-testid="connect-wechat-tts-model"
                  value={draft.wechatTtsModel}
                  onChange={(e) => patch({ wechatTtsModel: e.target.value })}
                  placeholder={t("settings.connectTab.wechat.ttsModelPlaceholder")}
                />
              </label>
              <label>
                <Text type="secondary" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.wechat.ttsVoice")}
                </Text>
                <Input
                  data-testid="connect-wechat-tts-voice"
                  value={draft.wechatTtsVoice}
                  onChange={(e) => patch({ wechatTtsVoice: e.target.value })}
                  placeholder={t("settings.connectTab.wechat.ttsVoicePlaceholder")}
                />
              </label>
            </Flex>

            {saveError ? <Alert type="error" showIcon message={saveError} /> : null}

            <Divider style={{ margin: `${token.marginXS}px 0` }} />

            <Flex align="center" justify="end" gap={token.marginSM}>
              {saved ? (
                <Text type="success" style={{ fontSize: token.fontSizeSM }}>
                  {t("settings.connectTab.saved")}
                </Text>
              ) : null}
              <Button
                data-testid="connect-save-button"
                type="primary"
                size="small"
                onClick={() => void save()}
                disabled={saving}
              >
                {saving ? t("settings.connectTab.saving") : t("settings.connectTab.save")}
              </Button>
            </Flex>
          </>
        )}
      </Flex>
    </Card>
  );
};

export default SystemSettingsConnectTab;
