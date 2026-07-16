import { useState } from "react";
import { useTranslation } from "react-i18next";
import { AppSettings, DownloadProgress, ModelInfo, updateSettings } from "../lib/ipc";
import HotkeyRecorder from "./HotkeyRecorder";
import { ModelList, PermissionsList } from "./SettingsView";
import chrome from "./settingsChrome.module.css";
import styles from "./OnboardingView.module.css";

const STEP_KEYS = ["stepPermissions", "stepModel", "stepHotkey"] as const;
const MODEL_STEP = 1;
const HOTKEY_STEP = 2;

type Props = {
  settings: AppSettings;
  models: ModelInfo[];
  progress: Record<string, DownloadProgress>;
  isMac: boolean;
  micGranted: boolean | null;
  axGranted: boolean | null;
  hotkeyError: string | null;
  onRequestMic: () => void;
  onRequestAx: () => void;
  onDownloadModel: (modelId: string) => void;
  onSelectModel: (modelId: string) => void;
  onDeleteModel: (modelId: string) => void;
  onHotkeyChange: (hotkey: string) => void;
  onCompleted: (next: AppSettings) => void;
};

export default function OnboardingView({
  settings,
  models,
  progress,
  isMac,
  micGranted,
  axGranted,
  hotkeyError,
  onRequestMic,
  onRequestAx,
  onDownloadModel,
  onSelectModel,
  onDeleteModel,
  onHotkeyChange,
  onCompleted,
}: Props) {
  const { t } = useTranslation("common");
  const [step, setStep] = useState(0);
  const hasModel = models.some((model) => model.downloaded);
  const canContinue = step !== MODEL_STEP || hasModel;

  const finish = async () => {
    const next = { ...settings, onboardingCompleted: true };
    await updateSettings(next);
    onCompleted(next);
  };

  return (
    <div className={styles.shell}>
      <div className={styles.wizard}>
        <h1 className={styles.title}>{t("onboarding.title")}</h1>

        <ol className={styles.steps}>
          {STEP_KEYS.map((key, index) => (
            <li
              key={key}
              className={`${styles.stepDot} ${index === step ? styles.stepDotActive : ""} ${
                index < step ? styles.stepDotDone : ""
              }`}
            >
              {t(`onboarding.${key}`)}
            </li>
          ))}
        </ol>

        <div className={styles.body}>
          {step === 0 &&
            (isMac ? (
              <>
                <p className={chrome.paneNote}>{t("onboarding.permissionsIntro")}</p>
                <PermissionsList
                  micGranted={micGranted}
                  axGranted={axGranted}
                  onRequestMic={onRequestMic}
                  onRequestAx={onRequestAx}
                />
              </>
            ) : (
              <p className={chrome.paneNote}>{t("onboarding.permissionsSkippable")}</p>
            ))}

          {step === MODEL_STEP && (
            <>
              <p className={chrome.paneNote}>{t("onboarding.modelIntro")}</p>
              <ModelList
                models={models}
                activeModelId={settings.modelId}
                progress={progress}
                onSelect={onSelectModel}
                onDownload={onDownloadModel}
                onDelete={onDeleteModel}
              />
              {!hasModel && <p className={styles.hint}>{t("onboarding.needModel")}</p>}
            </>
          )}

          {step === HOTKEY_STEP && (
            <>
              <p className={chrome.paneNote}>{t("onboarding.hotkeyIntro")}</p>
              <div className={chrome.group}>
                <div className={chrome.row}>
                  <div className={chrome.rowLabel}>{t("dictation.hotkey")}</div>
                  <HotkeyRecorder value={settings.hotkey} onChange={onHotkeyChange} />
                </div>
              </div>
              {hotkeyError && <div className={chrome.rowError}>{hotkeyError}</div>}
            </>
          )}
        </div>

        <div className={styles.footer}>
          <div className={styles.footerSide}>
            {step > 0 && (
              <button
                type="button"
                className={chrome.buttonGhost}
                onClick={() => setStep(step - 1)}
              >
                {t("onboarding.back")}
              </button>
            )}
          </div>
          <div className={styles.footerSide}>
            {step < HOTKEY_STEP && hasModel && (
              <button type="button" className={chrome.buttonGhost} onClick={finish}>
                {t("onboarding.skipReady")}
              </button>
            )}
            {step < HOTKEY_STEP ? (
              <button
                type="button"
                className={chrome.buttonPrimary}
                disabled={!canContinue}
                onClick={() => setStep(step + 1)}
              >
                {t("onboarding.continue")}
              </button>
            ) : (
              <button type="button" className={chrome.buttonPrimary} onClick={finish}>
                {t("onboarding.finish")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
