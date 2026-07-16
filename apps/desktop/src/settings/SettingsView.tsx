import { useCallback, useEffect, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import {
  checkAccessibilityPermission,
  checkMicrophonePermission,
  requestAccessibilityPermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import {
  AppSettings,
  DownloadProgress,
  ModelInfo,
  deleteModel,
  downloadModel,
  getSettings,
  listModels,
  updateSettings,
} from "../lib/ipc";
import { UI_LANGUAGE_EVENT, applyUiLanguage } from "../i18n/i18n";
import DictionarySection from "./DictionarySection";
import HotkeyRecorder from "./HotkeyRecorder";
import OnboardingView from "./OnboardingView";
import SettingsShell, { type SettingsSection } from "./SettingsShell";
import chrome from "./settingsChrome.module.css";

const SPEECH_LANGUAGES: [string, string][] = [
  ["en", "English"],
  ["ru", "Русский"],
  ["uk", "Українська"],
  ["de", "Deutsch"],
  ["fr", "Français"],
  ["es", "Español"],
  ["it", "Italiano"],
  ["pt", "Português"],
  ["pl", "Polski"],
  ["tr", "Türkçe"],
  ["zh", "中文"],
  ["ja", "日本語"],
  ["ko", "한국어"],
];

const UI_LANGUAGES = ["auto", "en", "ru"] as const;

export type Progress = Record<string, DownloadProgress>;

export default function SettingsView() {
  const { t } = useTranslation("common");
  const [section, setSection] = useState<SettingsSection>("general");
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [progress, setProgress] = useState<Progress>({});
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [micGranted, setMicGranted] = useState<boolean | null>(null);
  const [axGranted, setAxGranted] = useState<boolean | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const isMac = navigator.userAgent.includes("Mac");

  const refreshModels = useCallback(() => {
    listModels().then(setModels).catch(console.error);
  }, []);

  const refreshPermissions = useCallback(() => {
    checkMicrophonePermission().then(setMicGranted).catch(console.error);
    checkAccessibilityPermission().then(setAxGranted).catch(console.error);
  }, []);

  useEffect(() => {
    getSettings().then(setSettings).catch(console.error);
    refreshModels();
    isEnabled().then(setAutostart).catch(console.error);
    if (isMac) refreshPermissions();
  }, [isMac, refreshModels, refreshPermissions]);

  useEffect(() => {
    if (!isMac) return;
    const id = setInterval(refreshPermissions, 3000);
    return () => clearInterval(id);
  }, [isMac, refreshPermissions]);

  useEffect(() => {
    const unlisten = listen<DownloadProgress>("model-download-progress", (event) => {
      const p = event.payload;
      setProgress((prev) => ({ ...prev, [p.modelId]: p }));
      if (p.done) refreshModels();
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refreshModels]);

  useEffect(() => {
    const unlisten = listen<SettingsSection>("settings-open-section", (event) => {
      setSection(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const save = useCallback(
    async (patch: Partial<AppSettings>) => {
      if (!settings) return;
      const next = { ...settings, ...patch };
      const prev = settings;
      setSettings(next);
      try {
        await updateSettings(next);
        setHotkeyError(null);
      } catch (e) {
        setSettings(prev);
        if (patch.hotkey) setHotkeyError(String(e));
        else console.error(e);
      }
    },
    [settings],
  );

  const changeUiLanguage = useCallback(
    (uiLanguage: string) => {
      applyUiLanguage(uiLanguage);
      void emit(UI_LANGUAGE_EVENT, uiLanguage);
      void save({ uiLanguage });
    },
    [save],
  );

  const startDownload = useCallback((modelId: string) => {
    setProgress((prev) => ({
      ...prev,
      [modelId]: {
        modelId,
        downloadedBytes: 0,
        totalBytes: null,
        percent: 0,
        done: false,
        error: null,
      },
    }));
    downloadModel(modelId).catch(console.error);
  }, []);

  const toggleAutostart = useCallback(async () => {
    try {
      if (autostart) {
        await disable();
        setAutostart(false);
      } else {
        await enable();
        setAutostart(true);
      }
    } catch (e) {
      console.error(e);
    }
  }, [autostart]);

  const requestMic = useCallback(
    () => requestMicrophonePermission().then(refreshPermissions),
    [refreshPermissions],
  );
  const requestAx = useCallback(
    () => requestAccessibilityPermission().then(refreshPermissions),
    [refreshPermissions],
  );
  const selectModel = useCallback((modelId: string) => void save({ modelId }), [save]);
  const removeModel = useCallback(
    (modelId: string) => deleteModel(modelId).then(refreshModels).catch(console.error),
    [refreshModels],
  );
  const changeHotkey = useCallback((hotkey: string) => void save({ hotkey }), [save]);

  if (!settings) {
    return <div className={chrome.shell} />;
  }

  if (!settings.onboardingCompleted) {
    return (
      <OnboardingView
        settings={settings}
        models={models}
        progress={progress}
        isMac={isMac}
        micGranted={micGranted}
        axGranted={axGranted}
        hotkeyError={hotkeyError}
        onRequestMic={requestMic}
        onRequestAx={requestAx}
        onDownloadModel={startDownload}
        onSelectModel={selectModel}
        onDeleteModel={removeModel}
        onHotkeyChange={changeHotkey}
        onCompleted={setSettings}
      />
    );
  }

  return (
    <SettingsShell section={section} onSectionChange={setSection}>
      {section === "general" && (
        <GeneralPane
          settings={settings}
          autostart={autostart}
          onUiLanguageChange={changeUiLanguage}
          onLanguageChange={(language) => save({ language })}
          onToggleAutostart={toggleAutostart}
        />
      )}

      {section === "model" && (
        <ModelPane
          models={models}
          activeModelId={settings.modelId}
          progress={progress}
          onSelect={selectModel}
          onDownload={startDownload}
          onDelete={removeModel}
        />
      )}

      {section === "hotkey" && (
        <HotkeyPane hotkey={settings.hotkey} error={hotkeyError} onChange={changeHotkey} />
      )}

      {section === "vocabulary" && <VocabularyPane />}

      {section === "permissions" && isMac && (
        <PermissionsPane
          micGranted={micGranted}
          axGranted={axGranted}
          onRequestMic={requestMic}
          onRequestAx={requestAx}
        />
      )}
    </SettingsShell>
  );
}

function GeneralPane({
  settings,
  autostart,
  onUiLanguageChange,
  onLanguageChange,
  onToggleAutostart,
}: {
  settings: AppSettings;
  autostart: boolean | null;
  onUiLanguageChange: (uiLanguage: string) => void;
  onLanguageChange: (language: string) => void;
  onToggleAutostart: () => void;
}) {
  const { t } = useTranslation("common");
  return (
    <>
      <h1 className={chrome.paneTitle}>{t("nav.general")}</h1>
      <div className={chrome.group}>
        <div className={chrome.row}>
          <div>
            <div className={chrome.rowLabel}>{t("general.interfaceLanguage")}</div>
            <div className={chrome.rowDetail}>{t("general.interfaceLanguageDetail")}</div>
          </div>
          <select
            className={chrome.select}
            value={settings.uiLanguage}
            onChange={(e) => onUiLanguageChange(e.target.value)}
          >
            {UI_LANGUAGES.map((code) => (
              <option key={code} value={code}>
                {t(`general.interfaceLanguageOption.${code}`)}
              </option>
            ))}
          </select>
        </div>

        <div className={chrome.row}>
          <div>
            <div className={chrome.rowLabel}>{t("dictation.language")}</div>
            <div className={chrome.rowDetail}>{t("dictation.languageDetail")}</div>
          </div>
          <select
            className={chrome.select}
            value={settings.language}
            onChange={(e) => onLanguageChange(e.target.value)}
          >
            <option value="auto">{t("dictation.languageAuto")}</option>
            {SPEECH_LANGUAGES.map(([code, label]) => (
              <option key={code} value={code}>
                {label}
              </option>
            ))}
          </select>
        </div>

        <div className={chrome.row}>
          <div>
            <div className={chrome.rowLabel}>{t("general.autostart")}</div>
            <div className={chrome.rowDetail}>{t("general.autostartDetail")}</div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={!!autostart}
            className={`${chrome.toggle} ${autostart ? chrome.toggleOn : ""}`}
            onClick={onToggleAutostart}
          >
            <span className={chrome.toggleKnob} />
          </button>
        </div>
      </div>
    </>
  );
}

function VocabularyPane() {
  const { t } = useTranslation("common");
  return (
    <>
      <h1 className={chrome.paneTitle}>{t("nav.vocabulary")}</h1>
      <DictionarySection />
    </>
  );
}

function HotkeyPane({
  hotkey,
  error,
  onChange,
}: {
  hotkey: string;
  error: string | null;
  onChange: (hotkey: string) => void;
}) {
  const { t } = useTranslation("common");
  return (
    <>
      <h1 className={chrome.paneTitle}>{t("nav.hotkey")}</h1>
      <div className={chrome.group}>
        <div className={chrome.row}>
          <div>
            <div className={chrome.rowLabel}>{t("dictation.hotkey")}</div>
            <div className={chrome.rowDetail}>{t("dictation.hotkeyDetail")}</div>
          </div>
          <HotkeyRecorder value={hotkey} onChange={onChange} />
        </div>
        {error && <div className={chrome.rowError}>{error}</div>}
      </div>
    </>
  );
}

export type ModelListProps = {
  models: ModelInfo[];
  activeModelId: string;
  progress: Progress;
  onSelect: (modelId: string) => void;
  onDownload: (modelId: string) => void;
  onDelete: (modelId: string) => void;
};

export function ModelList({
  models,
  activeModelId,
  progress,
  onSelect,
  onDownload,
  onDelete,
}: ModelListProps) {
  return (
    <div className={chrome.group}>
      {models.map((model) => (
        <ModelRow
          key={model.id}
          model={model}
          active={activeModelId === model.id}
          progress={progress[model.id]}
          onSelect={() => onSelect(model.id)}
          onDownload={() => onDownload(model.id)}
          onDelete={() => onDelete(model.id)}
        />
      ))}
    </div>
  );
}

function ModelPane(props: ModelListProps) {
  const { t } = useTranslation("common");
  return (
    <>
      <h1 className={chrome.paneTitle}>{t("nav.model")}</h1>
      <p className={chrome.paneNote}>{t("models.note")}</p>
      <ModelList {...props} />
    </>
  );
}

export type PermissionsListProps = {
  micGranted: boolean | null;
  axGranted: boolean | null;
  onRequestMic: () => void;
  onRequestAx: () => void;
};

export function PermissionsList({
  micGranted,
  axGranted,
  onRequestMic,
  onRequestAx,
}: PermissionsListProps) {
  const { t } = useTranslation("common");
  return (
    <div className={chrome.group}>
      <PermissionRow
        name={t("permissions.microphone")}
        detail={t("permissions.microphoneDetail")}
        granted={micGranted}
        onRequest={onRequestMic}
      />
      <PermissionRow
        name={t("permissions.accessibility")}
        detail={t("permissions.accessibilityDetail")}
        granted={axGranted}
        onRequest={onRequestAx}
      />
    </div>
  );
}

function PermissionsPane(props: PermissionsListProps) {
  const { t } = useTranslation("common");
  return (
    <>
      <h1 className={chrome.paneTitle}>{t("nav.permissions")}</h1>
      <PermissionsList {...props} />
    </>
  );
}

function PermissionRow({
  name,
  detail,
  granted,
  onRequest,
}: {
  name: string;
  detail: string;
  granted: boolean | null;
  onRequest: () => void;
}) {
  const { t } = useTranslation("common");
  return (
    <div className={chrome.row}>
      <div>
        <div className={chrome.rowLabel}>
          <span
            className={`${chrome.statusDot} ${
              granted === null ? "" : granted ? chrome.statusOk : chrome.statusBad
            }`}
          />
          {name}
        </div>
        <div className={chrome.rowDetail}>{detail}</div>
      </div>
      {granted === false && (
        <button type="button" className={chrome.buttonSecondary} onClick={onRequest}>
          {t("permissions.grant")}
        </button>
      )}
      {granted === true && <span className={chrome.grantedText}>{t("permissions.granted")}</span>}
    </div>
  );
}

function ModelRow({
  model,
  active,
  progress,
  onSelect,
  onDownload,
  onDelete,
}: {
  model: ModelInfo;
  active: boolean;
  progress?: DownloadProgress;
  onSelect: () => void;
  onDownload: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation("common");
  const downloading = !!progress && !progress.done;
  const languageBadge =
    model.languages === "multilingual"
      ? t("models.badge.multilingual")
      : t(`models.badge.${model.languages}`, {
          defaultValue: model.languages.toUpperCase().split(",").join(", "),
        });
  const description = t(`models.description.${model.id}`, {
    defaultValue: model.description,
  });

  return (
    <div className={chrome.row}>
      <label className={chrome.modelMain}>
        <input
          type="radio"
          name="model"
          checked={active}
          disabled={!model.downloaded}
          onChange={onSelect}
          className={chrome.radio}
        />
        <div>
          <div className={chrome.rowLabel}>
            {model.label} <span className={chrome.modelSize}>{model.sizeLabel}</span>{" "}
            <span
              className={`${chrome.badge} ${
                model.languages === "multilingual" ? "" : chrome.badgeRu
              }`}
            >
              {languageBadge}
            </span>
          </div>
          <div className={chrome.rowDetail}>{description}</div>
          {progress?.error && <div className={chrome.rowError}>{progress.error}</div>}
        </div>
      </label>

      <div className={chrome.modelActions}>
        {downloading ? (
          <div className={chrome.progressWrap}>
            <div className={chrome.progressTrack}>
              <div className={chrome.progressFill} style={{ width: `${progress.percent}%` }} />
            </div>
            <span className={chrome.progressText}>{progress.percent}%</span>
          </div>
        ) : model.downloaded ? (
          !active && (
            <button type="button" className={chrome.buttonGhost} onClick={onDelete}>
              {t("models.delete")}
            </button>
          )
        ) : (
          <button type="button" className={chrome.buttonPrimary} onClick={onDownload}>
            {t("models.download")}
          </button>
        )}
      </div>
    </div>
  );
}
