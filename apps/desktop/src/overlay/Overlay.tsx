import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import type { DictationPhase, DictationState } from "../lib/ipc";
import Mascot from "./Mascot";
import styles from "./Overlay.module.css";

const LABEL_KEYS: Partial<Record<DictationPhase, string>> = {
  recording: "overlay.listening",
  transcribing: "overlay.transcribing",
  inserted: "overlay.inserted",
  error: "overlay.error",
};

export default function Overlay() {
  const { t } = useTranslation("common");
  const [phase, setPhase] = useState<DictationPhase>("idle");
  const [previewText, setPreviewText] = useState("");
  const levelRef = useRef(0);

  useEffect(() => {
    const unState = listen<DictationState>("dictation-state", (event) => {
      const { phase, text } = event.payload;
      setPhase(phase);
      // Live transcription arrives as `text` on the `recording` phase; any
      // other phase (or a null text) resets the preview to the status label.
      setPreviewText(phase === "recording" && text ? text : "");
    });
    const unLevel = listen<number>("audio-level", (event) => {
      levelRef.current = event.payload;
    });
    return () => {
      unState.then((fn) => fn());
      unLevel.then((fn) => fn());
    };
  }, []);

  const labelKey = LABEL_KEYS[phase];
  const statusLabel = labelKey ? t(labelKey) : "";
  const showingPreview = phase === "recording" && previewText.length > 0;
  const caption = showingPreview ? previewText : statusLabel;

  return (
    <div className={styles.root}>
      {showingPreview ? (
        // Newest text is pinned to the bottom (next to the mascot); older lines
        // scroll up and fade out, so you always see what you are saying now.
        <div className={styles.previewViewport}>
          <div className={styles.previewText}>{previewText}</div>
        </div>
      ) : (
        <div className={styles.label}>{caption || "\u00a0"}</div>
      )}
      <div className={styles.mascotClip}>
        <Mascot phase={phase} levelRef={levelRef} />
      </div>
    </div>
  );
}
