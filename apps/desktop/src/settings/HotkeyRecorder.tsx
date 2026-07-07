import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
]);

const MODIFIER_ORDER = ["Fn", "Ctrl", "Alt", "Shift", "Super"] as const;

const MAC_SYMBOLS: Record<string, string> = {
  Fn: "fn",
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

    // WebKit never delivers the Fn (Globe) key to the DOM, so the backend
    // streams the held modifier set (incl. Fn) from a CGEventTap while we
    // capture. When the tap is active it owns modifier-only tracking; the
    // DOM keyup path stays as a fallback (e.g. tap creation failed).
    let rustAssist = false;

    const finalizeModifierOnly = () => {
      const max = maxModsRef.current;
      if (max.size >= 2) {
        setCapturing(false);
        onChange(MODIFIER_ORDER.filter((m) => max.has(m)).join("+"));
      } else {
        // Too few modifiers for a modifier-only combo — keep capturing.
        maxModsRef.current = new Set();
      }
    };

    const onKeyDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setCapturing(false);
        return;
      }
      if (!rustAssist) {
        const held = heldModifiers(e);
        if (held.length > maxModsRef.current.size) {
          maxModsRef.current = new Set(held);
        }
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
      if (rustAssist) return;
      if (heldModifiers(e).length > 0) return;
      finalizeModifierOnly();
    };

    const onBlur = () => setCapturing(false);

    let disposed = false;
    let unlisten: (() => void) | undefined;
    invoke("start_hotkey_capture")
      .then(async () => {
        rustAssist = true;
        const fn = await listen<string[]>("hotkey-capture-update", (event) => {
          if (!capturingRef.current) return;
          const held = event.payload;
          if (held.length > maxModsRef.current.size) {
            maxModsRef.current = new Set(held);
          }
          if (held.length === 0) finalizeModifierOnly();
        });
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // No capture assist (e.g. not macOS) — DOM-only capture still works
        // for everything except Fn.
      });

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      capturingRef.current = false;
      disposed = true;
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", onBlur);
      unlisten?.();
      invoke("stop_hotkey_capture").catch(() => {});
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
