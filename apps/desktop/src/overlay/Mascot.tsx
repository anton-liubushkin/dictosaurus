import { useEffect, useRef } from "react";
import type { MutableRefObject } from "react";
import type { DictationPhase } from "../lib/ipc";
import bodyUrl from "../assets/mascot/body.png";
import headUrl from "../assets/mascot/head.png";
import pencilUrl from "../assets/mascot/arm-pencil.png";
import notepadUrl from "../assets/mascot/arm-notepad.png";
import styles from "./Mascot.module.css";

type Props = {
  phase: DictationPhase;
  /** Live microphone level (0..1), updated outside React. */
  levelRef: MutableRefObject<number>;
};

/**
 * Layered dictating T-Rex. All motion is procedural (sine-based) and driven
 * by a single rAF loop; the head bob amplitude follows the voice level, the
 * pencil scribbles fast while recording and calms down while transcribing.
 */
export default function Mascot({ phase, levelRef }: Props) {
  const bodyRef = useRef<HTMLImageElement>(null);
  const headRef = useRef<HTMLImageElement>(null);
  const pencilRef = useRef<HTMLImageElement>(null);
  const notepadRef = useRef<HTMLImageElement>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  useEffect(() => {
    let raf = 0;
    let voice = 0;
    const started = performance.now();

    const frame = (now: number) => {
      const t = (now - started) / 1000;
      const p = phaseRef.current;
      const recording = p === "recording";
      const busy = p === "transcribing";

      // Fast attack, slow release — same envelope the orb used.
      const target = Math.min(1, levelRef.current * 6);
      voice += (target - voice) * (target > voice ? 0.35 : 0.06);

      const body = bodyRef.current;
      const head = headRef.current;
      const pencil = pencilRef.current;
      const notepad = notepadRef.current;
      if (body && head && pencil && notepad) {
        // Body: slow breathing, slightly deeper while "thinking".
        const breath = 1 + (busy ? 0.012 : 0.008) * Math.sin(t * (busy ? 1.6 : 2.2));
        body.style.transform = `scaleY(${breath})`;

        // Head: gentle bob; the voice adds tilt and a small nod.
        const bobSpeed = busy ? 1.1 : 1.6;
        const tilt =
          Math.sin(t * bobSpeed) * (2 + voice * 4) +
          (recording ? Math.sin(t * 7) * voice * 1.5 : 0);
        const nod = Math.sin(t * bobSpeed * 0.9 + 1) * (1.5 + voice * 4);
        head.style.transform = `rotate(${tilt}deg) translateY(${nod}px)`;

        // Pencil: fast scribble while recording, lazy drift otherwise.
        if (recording) {
          const s = t * (9 + voice * 5);
          const rot = -5 + Math.sin(s) * 5 + Math.sin(s * 2.3) * 2;
          const dx = Math.sin(s * 1.7) * 3;
          const dy = 2 + Math.sin(s * 2.9) * 2.5;
          pencil.style.transform = `rotate(${rot}deg) translate(${dx}px, ${dy}px)`;
        } else {
          pencil.style.transform = `rotate(${Math.sin(t * 1.4) * 2}deg)`;
        }

        // Notepad: barely sways against the pencil so the grip feels alive.
        notepad.style.transform = `rotate(${Math.sin(t * 1.3 + 2) * 1.2}deg)`;
      }

      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  }, [levelRef]);

  // Slides away as soon as the text lands ("inserted") or the session ends;
  // stays up while recording, thinking, or showing an error.
  const visible =
    phase === "recording" || phase === "transcribing" || phase === "error";

  return (
    <div className={`${styles.mascot} ${visible ? styles.visible : ""}`}>
      <img ref={bodyRef} className={styles.body} src={bodyUrl} alt="" />
      <img ref={headRef} className={styles.head} src={headUrl} alt="" />
      <img ref={notepadRef} className={styles.notepad} src={notepadUrl} alt="" />
      <img ref={pencilRef} className={styles.pencil} src={pencilUrl} alt="" />
    </div>
  );
}
