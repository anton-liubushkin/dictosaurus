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
  const levelRef = useRef(0);

  useEffect(() => {
    const unState = listen<DictationState>("dictation-state", (event) => {
      setPhase(event.payload.phase);
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
  const label = labelKey ? t(labelKey) : "";

  return (
    <div className={styles.root}>
      <div className={styles.label}>{label || "\u00a0"}</div>
      <div className={styles.mascotClip}>
        <Mascot phase={phase} levelRef={levelRef} />
      </div>
    </div>
  );
}
