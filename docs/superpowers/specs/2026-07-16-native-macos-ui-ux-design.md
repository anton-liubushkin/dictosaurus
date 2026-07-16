# Native macOS UI/UX — Design Spec

**Date:** 2026-07-16  
**Product:** Dictosaurus (Tauri 2 + React desktop dictation)  
**Platform focus:** macOS 13+ (Apple Silicon first; Windows polish out of scope)

## Goal

Make Dictosaurus feel like a professional macOS menu-bar utility: Settings match System Settings language; the dictation overlay is a calm system HUD; first-run setup is guided, then the app starts quietly in the tray.

## Decisions (locked)

| Topic | Choice |
|---|---|
| Visual direction | Approach **A** first (System Settings aesthetic), then **C** (UX flows) |
| Appearance | Follow **system** Light and Dark (`prefers-color-scheme`; no in-app theme toggle) |
| Overlay | **System HUD** — no mascot / T-Rex in the dictation flow |
| Launch | **First-run onboarding**, then quiet tray-only start |
| Implementation approach | **Hybrid:** System Settings–style shell in React + Tauri window chrome; not a SwiftUI rewrite |

## Non-goals

- Windows-specific UI polish
- Rewriting Settings in SwiftUI/AppKit
- Changing paste/injection strategy
- Tray “Ready / Needs setup” status item (YAGNI for this iteration)
- Reintroducing marketing gradients, purple/cyan AI glow, uppercase SaaS section labels

---

## 1. Settings — visual system

### Window chrome

- Main window follows system appearance (Light and Dark).
- Align webview background with the system window background so title bar and content do not clash.
- Prefer a compact, utility-like chrome (no branded glowing logo orb as the primary identity in the header).

### Layout

- **Left:** narrow sidebar with sections (icons + labels), e.g.:
  - General
  - Model
  - Hotkey
  - Vocabulary
  - Permissions
- **Right:** inset **grouped lists** (System Settings pattern): rows with label + control, separators inside a rounded group, not free-floating “cards with neon borders”.

### Color and type

- Remove fixed purple brand palette (`#7c5cff` / cyan duo) as the primary accent.
- Use **system blue** (or macOS accent-aligned tokens) for primary actions and focus.
- Body / labels: SF system stack (already present); sizing and hierarchy closer to macOS Settings (title, secondary label, footnote).
- No uppercase tracked section headers as the default pattern.

### Controls

- Restyle toggles, selects, buttons, and progress to AppKit-like proportions (height, radius, borders) without claiming pixel-perfect native widgets.
- Allow text selection in Settings content (do not keep global `user-select: none` on the settings body). Overlay mode may still disable selection.

### Tray menu

- Localize tray strings (`Settings…`, `Quit Dictosaurus`) for EN/RU to match UI language.

---

## 2. Overlay — system HUD

### Replace mascot

- Remove the T-Rex mascot assets and animation from the live dictation overlay path.
- Replace with a compact **frosted capsule / pill** anchored near the bottom of the screen (current pin position can stay).

### Content

- Voice activity indicator: a **pulse dot** driven by mic level (simple; no full waveform UI in this iteration).
- Status / live preview text:
  - ~2–3 lines max
  - newest content bottom-aligned
  - soft fade at the top
- Avoid heavy marketing text-shadows; ensure contrast on both Light and Dark (and over arbitrary desktop wallpapers via translucent fill + readable text color).

### States

| Phase | HUD behavior |
|---|---|
| `recording` | Pulse / level + live preview text |
| `transcribing` | Calm indeterminate affordance + short “Recognizing…” (localized) |
| `inserted` | Brief success cue → hide |
| `error` / `canceled` | Short message → hide |
| Idle | Hidden |

### Motion

- Appear/dismiss ~200–300 ms, restrained easing (no playful mascot overshoot).

### Window sizing

- Resize the overlay native window to fit the HUD (smaller than the current mascot-oriented height), keeping NSPanel behavior on macOS.

---

## 3. UX flows — launch and readiness

### First-run onboarding

Show a short guided flow inside Settings when the app is not ready for dictation (first launch or incomplete setup):

1. **Permissions** — Microphone + Accessibility: status + open System Settings actions  
2. **Model** — download / select a speech model  
3. **Hotkey** — confirm or set the push-to-talk shortcut  

Persist `onboardingCompleted` in app settings when either:

- the user finishes all three steps, or
- the user dismisses/continues after **minimum readiness** (at least one model downloaded).

Re-show onboarding (or deep-link into the missing step) if the user later deletes all models or tries to dictate without Mic/Accessibility where we can detect that.

**Minimum ready to dictate:** at least one model downloaded. Permissions are requested/shown prominently in onboarding; if missing at hotkey press, open Settings on Permissions instead of a dead-end error flash.

### Subsequent launches

- **Quiet start:** tray icon only; do **not** open Settings on every `RunEvent::Ready`.
- Open Settings from tray, or automatically when the user tries to dictate and setup is incomplete.

### In-session UX

- Hotkey with no model → open Settings on Model (or onboarding), do not start recording.
- Hotkey with missing permissions → open Permissions guidance.
- Successful dictate path unchanged functionally (record → preview HUD → transcribe → paste); only presentation changes.

---

## 4. Technical notes (implementation constraints)

- Stay within existing Tauri + React + CSS modules stack.
- Drive Light/Dark via `prefers-color-scheme` (and window `backgroundColor` / vibrancy options if available in Tauri 2 without large native rewrites).
- i18n: extend `en`/`ru` strings for onboarding, HUD statuses, tray.
- Delete or stop shipping unused mascot overlay code/assets once HUD ships (clean up dead code).
- Keep dictation/VAD/preview backend behavior; this spec is UI/UX presentation and launch flow.

---

## 5. Success criteria

1. In Light and Dark macOS appearances, Settings reads as a System Settings–like utility (sidebar + grouped lists, system accent, no purple SaaS glow).
2. Dictation feedback is a frosted HUD without the mascot.
3. Fresh install: onboarding → model available → quiet subsequent launches.
4. Tray menu language matches the app UI language.
5. Text in Settings is selectable; overlay remains non-selectable / click-through as today.

## 6. Out of scope checklist

- [ ] Windows UI parity  
- [ ] SwiftUI Settings  
- [ ] Clipboard-bypass typing  
- [ ] Tray readiness badge  
- [ ] Redesigning the marketing site  
