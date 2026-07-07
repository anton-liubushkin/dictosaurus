import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
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
import HotkeyRecorder, { formatHotkey } from "./HotkeyRecorder";
import styles from "./SettingsView.module.css";

const LANGUAGES: [string, string][] = [
  ["auto", "Auto-detect"],
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

type Progress = Record<string, DownloadProgress>;

export default function SettingsView() {
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

  if (!settings) {
    return <div className={styles.page} />;
  }

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className={styles.logoDot} />
        <div>
          <h1 className={styles.title}>Dictosaurus</h1>
          <p className={styles.subtitle}>Local AI dictation — your voice never leaves this Mac</p>
        </div>
      </header>

      <section className={styles.hint}>
        Hold <kbd className={styles.kbd}>{formatHotkey(settings.hotkey)}</kbd>, speak, release — the
        text is inserted into the active input and copied to the clipboard.
      </section>

      {isMac && (
        <Section title="Permissions">
          <PermissionRow
            name="Microphone"
            detail="Required to record your dictation"
            granted={micGranted}
            onRequest={() => requestMicrophonePermission().then(refreshPermissions)}
          />
          <PermissionRow
            name="Accessibility"
            detail="Required to insert text into the active app"
            granted={axGranted}
            onRequest={() => requestAccessibilityPermission().then(refreshPermissions)}
          />
        </Section>
      )}

      <Section title="Dictation">
        <div className={styles.row}>
          <div>
            <div className={styles.rowLabel}>Push-to-talk hotkey</div>
            <div className={styles.rowDetail}>Hold to record, release to insert</div>
          </div>
          <HotkeyRecorder value={settings.hotkey} onChange={(hotkey) => save({ hotkey })} />
        </div>
        {hotkeyError && <div className={styles.error}>{hotkeyError}</div>}

        <div className={styles.row}>
          <div>
            <div className={styles.rowLabel}>Language</div>
            <div className={styles.rowDetail}>Spoken language, or auto-detect</div>
          </div>
          <select
            className={styles.select}
            value={settings.language}
            onChange={(e) => save({ language: e.target.value })}
          >
            {LANGUAGES.map(([code, label]) => (
              <option key={code} value={code}>
                {label}
              </option>
            ))}
          </select>
        </div>
      </Section>

      <Section title="Models">
        <p className={styles.sectionNote}>
          Whisper models run fully on-device with GPU acceleration. Bigger models are more accurate
          but slower.
        </p>
        {models.map((model) => (
          <ModelRow
            key={model.id}
            model={model}
            active={settings.modelId === model.id}
            progress={progress[model.id]}
            onSelect={() => save({ modelId: model.id })}
            onDownload={() => {
              setProgress((prev) => ({
                ...prev,
                [model.id]: {
                  modelId: model.id,
                  downloadedBytes: 0,
                  totalBytes: null,
                  percent: 0,
                  done: false,
                  error: null,
                },
              }));
              downloadModel(model.id).catch(console.error);
            }}
            onDelete={() => deleteModel(model.id).then(refreshModels).catch(console.error)}
          />
        ))}
      </Section>

      <Section title="General">
        <div className={styles.row}>
          <div>
            <div className={styles.rowLabel}>Launch at login</div>
            <div className={styles.rowDetail}>Start Dictosaurus automatically</div>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={!!autostart}
            className={`${styles.toggle} ${autostart ? styles.toggleOn : ""}`}
            onClick={toggleAutostart}
          >
            <span className={styles.toggleKnob} />
          </button>
        </div>
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className={styles.section}>
      <h2 className={styles.sectionTitle}>{title}</h2>
      <div className={styles.card}>{children}</div>
    </section>
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
  return (
    <div className={styles.row}>
      <div>
        <div className={styles.rowLabel}>
          <span
            className={`${styles.statusDot} ${
              granted === null ? "" : granted ? styles.statusOk : styles.statusBad
            }`}
          />
          {name}
        </div>
        <div className={styles.rowDetail}>{detail}</div>
      </div>
      {granted === false && (
        <button type="button" className={styles.buttonSecondary} onClick={onRequest}>
          Grant
        </button>
      )}
      {granted === true && <span className={styles.grantedText}>Granted</span>}
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
  const downloading = !!progress && !progress.done;

  return (
    <div className={`${styles.modelRow} ${active ? styles.modelActive : ""}`}>
      <label className={styles.modelMain}>
        <input
          type="radio"
          name="model"
          checked={active}
          disabled={!model.downloaded}
          onChange={onSelect}
          className={styles.radio}
        />
        <div>
          <div className={styles.rowLabel}>
            {model.label} <span className={styles.modelSize}>{model.sizeLabel}</span>
          </div>
          <div className={styles.rowDetail}>{model.description}</div>
          {progress?.error && <div className={styles.error}>{progress.error}</div>}
        </div>
      </label>

      <div className={styles.modelActions}>
        {downloading ? (
          <div className={styles.progressWrap}>
            <div className={styles.progressTrack}>
              <div className={styles.progressFill} style={{ width: `${progress.percent}%` }} />
            </div>
            <span className={styles.progressText}>{progress.percent}%</span>
          </div>
        ) : model.downloaded ? (
          !active && (
            <button type="button" className={styles.buttonGhost} onClick={onDelete}>
              Delete
            </button>
          )
        ) : (
          <button type="button" className={styles.buttonPrimary} onClick={onDownload}>
            Download
          </button>
        )}
      </div>
    </div>
  );
}
