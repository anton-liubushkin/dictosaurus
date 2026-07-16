import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import type { DictationPhase, DictationState } from "../lib/ipc";
import styles from "./Overlay.module.css";

export default function Overlay() {
  const { t } = useTranslation("common");
  const [phase, setPhase] = useState<DictationPhase>("idle");
  const [previewText, setPreviewText] = useState("");
  const [level, setLevel] = useState(0);

  useEffect(() => {
    const unState = listen<DictationState>("dictation-state", (event) => {
      const { phase, text } = event.payload;
      setPhase(phase);
      // Live transcription arrives as `text` on the `recording` phase; any
      // other phase (or a null text) resets the preview to the status label.
      setPreviewText(phase === "recording" && text ? text : "");
    });
    const unLevel = listen<number>("audio-level", (event) => {
      setLevel(event.payload);
    });
    return () => {
      unState.then((fn) => fn());
      unLevel.then((fn) => fn());
    };
  }, []);

  const showingPreview = phase === "recording" && previewText.length > 0;
  const status =
    phase === "recording" && !showingPreview
      ? t("overlay.listening")
      : phase === "transcribing"
        ? t("overlay.transcribing")
        : phase === "inserted"
          ? t("overlay.inserted")
          : phase === "error"
            ? t("overlay.error")
            : phase === "canceled"
              ? t("overlay.canceled", { defaultValue: "" })
              : "";

  const pulse = Math.min(1, level * 4);

  return (
    <div className={styles.root}>
      <div className={styles.hud} data-phase={phase}>
        <span
          className={styles.pulse}
          style={{
            transform: `scale(${0.75 + pulse * 0.6})`,
            opacity: 0.5 + pulse * 0.5,
          }}
          aria-hidden
        />
        {showingPreview ? (
          <div className={styles.previewViewport}>
            <div className={styles.previewText}>{previewText}</div>
          </div>
        ) : (
          <div className={styles.status}>{status || "\u00a0"}</div>
        )}
      </div>
    </div>
  );
}
