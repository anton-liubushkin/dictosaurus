import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
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

const MODIFIER_ORDER = ["Ctrl", "Alt", "Shift", "Super"] as const;

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

function heldModifiers(e: KeyboardEvent): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  return mods;
}

function comboFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_CODES.has(e.code)) return null;

  const mods = heldModifiers(e);
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
  const { t } = useTranslation("common");
  const [capturing, setCapturing] = useState(false);
  const capturingRef = useRef(false);

  // Largest modifier set held simultaneously during the current capture;
  // committed as a modifier-only hotkey when everything is released without
  // a regular key ever being pressed (2+ modifiers required — a single one
  // would misfire constantly during normal typing).
  const maxModsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!capturing) return;
    capturingRef.current = true;
    maxModsRef.current = new Set();

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setCapturing(false);
        return;
      }
      const held = heldModifiers(e);
      if (held.length > maxModsRef.current.size) {
        maxModsRef.current = new Set(held);
      }
      const combo = comboFromEvent(e);
      if (combo) {
        setCapturing(false);
        onChange(combo);
      }
    };

    const onKeyUp = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (heldModifiers(e).length > 0) return;
      const max = maxModsRef.current;
      if (max.size >= 2) {
        setCapturing(false);
        onChange(MODIFIER_ORDER.filter((m) => max.has(m)).join("+"));
      } else {
        // Too few modifiers for a modifier-only combo — keep capturing.
        maxModsRef.current = new Set();
      }
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      capturingRef.current = false;
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [capturing, onChange]);

  return (
    <button
      type="button"
      className={`${styles.hotkeyButton} ${capturing ? styles.hotkeyCapturing : ""}`}
      onClick={() => setCapturing(true)}
    >
      {capturing ? t("hotkeyRecorder.capturing") : formatHotkey(value)}
    </button>
  );
}
