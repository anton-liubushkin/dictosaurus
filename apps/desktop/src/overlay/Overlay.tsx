import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import type { DictationPhase, DictationState } from "../lib/ipc";
import styles from "./Overlay.module.css";

const CANVAS_SIZE = 148;

const LABEL_KEYS: Partial<Record<DictationPhase, string>> = {
  recording: "overlay.listening",
  transcribing: "overlay.transcribing",
  inserted: "overlay.inserted",
  error: "overlay.error",
};

type RingPalette = [string, string][];

const RECORDING_COLORS: RingPalette = [
  ["#7c5cff", "#4ea9ff"],
  ["#2fd4ff", "#7c5cff"],
  ["#ff5ca8", "#6f6bff"],
];
const ERROR_COLORS: RingPalette = [
  ["#ff4d4d", "#ff9a5c"],
  ["#ff6b6b", "#ff4da6"],
  ["#ffa14d", "#ff4d4d"],
];
const OK_COLORS: RingPalette = [
  ["#2fe38f", "#4ea9ff"],
  ["#59f2c1", "#2fd4ff"],
  ["#8dff9e", "#2fe38f"],
];

const RINGS = [
  { speed: 0.9, offset: 0.0, amp: 1.0 },
  { speed: -1.3, offset: 2.1, amp: 0.8 },
  { speed: 1.7, offset: 4.2, amp: 0.6 },
];

function paletteFor(phase: DictationPhase): RingPalette {
  if (phase === "error") return ERROR_COLORS;
  if (phase === "inserted") return OK_COLORS;
  return RECORDING_COLORS;
}

function drawOrb(
  ctx: CanvasRenderingContext2D,
  size: number,
  t: number,
  level: number,
  phase: DictationPhase,
) {
  const c = size / 2;
  ctx.clearRect(0, 0, size, size);
  if (phase === "idle" || phase === "canceled") return;

  const recording = phase === "recording";
  const busy = phase === "transcribing";
  const palette = paletteFor(phase);

  const base = size * 0.2 + level * size * 0.13;
  ctx.globalCompositeOperation = "lighter";

  RINGS.forEach((ring, i) => {
    const wobble = recording
      ? 0.06 + Math.min(1, level * 6) * 0.3 * ring.amp
      : busy
        ? 0.1 + 0.05 * Math.sin(t * 3 + i)
        : 0.05;
    const breathe = busy ? 1 + 0.06 * Math.sin(t * 2.4 + i * 1.3) : 1;
    const R = base * (1 + i * 0.08) * breathe;

    ctx.beginPath();
    const N = 72;
    for (let k = 0; k <= N; k++) {
      const th = (k / N) * Math.PI * 2;
      const r =
        R *
        (1 +
          wobble *
            (0.5 * Math.sin(3 * th + t * ring.speed * 2 + ring.offset) +
              0.3 * Math.sin(5 * th - t * ring.speed * 1.4) +
              0.2 * Math.sin(8 * th + t * ring.speed * 0.8 + i)));
      const x = c + r * Math.cos(th);
      const y = c + r * Math.sin(th);
      if (k === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.closePath();

    const angle = t * ring.speed * 0.7 + ring.offset;
    const gx = Math.cos(angle) * R;
    const gy = Math.sin(angle) * R;
    const gradient = ctx.createLinearGradient(c - gx, c - gy, c + gx, c + gy);
    gradient.addColorStop(0, palette[i][0] + "cc");
    gradient.addColorStop(1, palette[i][1] + "22");
    ctx.fillStyle = gradient;
    ctx.fill();
  });

  const coreR = base * 0.55;
  const core = ctx.createRadialGradient(c, c, 0, c, c, coreR);
  core.addColorStop(0, "rgba(255, 255, 255, 0.9)");
  core.addColorStop(1, "rgba(255, 255, 255, 0)");
  ctx.fillStyle = core;
  ctx.beginPath();
  ctx.arc(c, c, coreR, 0, Math.PI * 2);
  ctx.fill();

  ctx.globalCompositeOperation = "source-over";
}

export default function Overlay() {
  const { t } = useTranslation("common");
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [phase, setPhase] = useState<DictationPhase>("idle");
  const phaseRef = useRef<DictationPhase>("idle");
  const levelRef = useRef(0);

  useEffect(() => {
    const unState = listen<DictationState>("dictation-state", (event) => {
      phaseRef.current = event.payload.phase;
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

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = CANVAS_SIZE * dpr;
    canvas.height = CANVAS_SIZE * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    let raf = 0;
    let smoothed = 0;
    const started = performance.now();

    const frame = (now: number) => {
      const t = (now - started) / 1000;
      // Fast attack, slow release keeps the orb lively but not jittery.
      const target = Math.min(1, levelRef.current * 6);
      smoothed += (target - smoothed) * (target > smoothed ? 0.35 : 0.08);
      drawOrb(ctx, CANVAS_SIZE, t, smoothed, phaseRef.current);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, []);

  const visible = phase !== "idle" && phase !== "canceled";
  const labelKey = LABEL_KEYS[phase];
  const label = labelKey ? t(labelKey) : "";

  return (
    <div className={`${styles.root} ${visible ? "" : styles.hidden}`}>
      <canvas
        ref={canvasRef}
        className={styles.canvas}
        style={{ width: CANVAS_SIZE, height: CANVAS_SIZE }}
      />
      <div className={styles.label}>{label || "\u00a0"}</div>
    </div>
  );
}
