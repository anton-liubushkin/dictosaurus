import { useEffect, useRef, useState } from "react";
import styles from "./SettingsView.module.css";

const MODIFIER_CODES = new Set([
  "MetaLeft",
  "MetaRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "CapsLock",
  "Fn",
]);

const MAC_SYMBOLS: Record<string, string> = {
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Super: "⌘",
};

/** Converts a KeyboardEvent.code into the key token global-shortcut expects. */
function keyToken(code: string): string {
  if (code.startsWith("Key")) return code.slice(3);
  if (code.startsWith("Digit")) return code.slice(5);
  return code; // "Space", "F5", "ArrowUp", "Backquote", … — W3C codes parse as-is
}

function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;

  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");

  const key = keyToken(e.code);
  const isFnKey = /^F\d{1,2}$/.test(key);
  if (mods.length === 0 && !isFnKey) return null;

  return [...mods, key].join("+");
}

export function formatHotkey(hotkey: string): string {
  return hotkey
    .split("+")
    .map((part) => MAC_SYMBOLS[part] ?? part)
    .join(" ");
}

type Props = {
  value: string;
  onChange: (hotkey: string) => void;
};

export default function HotkeyRecorder({ value, onChange }: Props) {
  const [capturing, setCapturing] = useState(false);
  const capturingRef = useRef(false);

  useEffect(() => {
    if (!capturing) return;
    capturingRef.current = true;

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setCapturing(false);
        return;
      }
      const combo = comboFromEvent(e);
      if (combo) {
        setCapturing(false);
        onChange(combo);
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      capturingRef.current = false;
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [capturing, onChange]);

  return (
    <button
      type="button"
      className={`${styles.hotkeyButton} ${capturing ? styles.hotkeyCapturing : ""}`}
      onClick={() => setCapturing(true)}
    >
      {capturing ? "Press shortcut… (Esc to cancel)" : formatHotkey(value)}
    </button>
  );
}
