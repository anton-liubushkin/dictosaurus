# Native macOS UI/UX Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Dictosaurus feel like a native macOS menu-bar utility: System Settings–style Light/Dark Settings, frosted dictation HUD (no mascot), first-run onboarding, then quiet tray launches.

**Architecture:** Keep Tauri 2 + React + CSS modules. Drive appearance with `prefers-color-scheme` and macOS-aligned tokens. Split Settings into a sidebar shell + section panes. Replace overlay mascot with a compact HUD. Persist `onboardingCompleted` in `AppSettings`; gate `RunEvent::Ready` and hotkey start on readiness.

**Tech Stack:** Tauri 2, React 18, CSS modules, i18next, Rust settings store, `tauri-plugin-macos-permissions-api`

**Spec:** `docs/superpowers/specs/2026-07-16-native-macos-ui-ux-design.md`

## Global Constraints

- macOS-first; Windows UI polish out of scope
- Follow system Light and Dark — no in-app theme toggle
- System blue accent — no purple/cyan AI glow palette
- No mascot in dictation overlay after Task 6
- No SwiftUI rewrite; stay in React + CSS modules
- Keep VAD/preview/transcription backend behavior unchanged
- Copy and commits in English; UI strings via i18n EN/RU
- YAGNI: no tray readiness badge

## File map

| File | Responsibility |
|---|---|
| `apps/desktop/src/index.css` | Light/Dark design tokens, selection rules |
| `apps/desktop/src-tauri/tauri.conf.json` | Window background (neutral; theme via CSS) |
| `apps/desktop/src/settings/settingsTheme.module.css` | Shared grouped-list / control styles |
| `apps/desktop/src/settings/SettingsShell.tsx` | Sidebar + active section routing |
| `apps/desktop/src/settings/SettingsView.tsx` | Data loading + section content (refactored) |
| `apps/desktop/src/settings/SettingsView.module.css` | Page layout for shell |
| `apps/desktop/src/settings/OnboardingView.tsx` | First-run 3-step wizard |
| `apps/desktop/src/settings/DictionarySection.module.css` | Align with new tokens |
| `apps/desktop/src/overlay/Overlay.tsx` | Frosted HUD UI |
| `apps/desktop/src/overlay/Overlay.module.css` | HUD styles |
| `apps/desktop/src/overlay/Mascot.tsx` (+ css + assets) | Delete after HUD ships |
| `apps/desktop/src-tauri/src/overlay.rs` | Smaller HUD window size |
| `apps/desktop/src-tauri/src/settings.rs` | `onboarding_completed: bool` |
| `apps/desktop/src/lib/ipc.ts` | `onboardingCompleted` on `AppSettings` |
| `apps/desktop/src-tauri/src/tray.rs` | Localized menu; helpers to show settings |
| `apps/desktop/src-tauri/src/lib.rs` | Quiet start / open onboarding when needed |
| `apps/desktop/src-tauri/src/dictation.rs` | Route incomplete setup to Settings |
| `apps/desktop/src/locales/{en,ru}/common.json` | Sidebar, onboarding, HUD, tray keys |
| `apps/desktop/src-tauri/src/settings.rs` tests (inline) | Default + serde for new field |

---

### Task 1: Design tokens (Light/Dark) + selectable Settings text

**Files:**
- Modify: `apps/desktop/src/index.css`
- Modify: `apps/desktop/src-tauri/tauri.conf.json` (window `backgroundColor` → `#F5F5F7` light-neutral; dark handled by CSS once webview paints — OR use `#00000000` if transparent titlebar is skipped; prefer opaque `#F5F5F7` matching light System Settings and let dark mode CSS override body immediately)
- Test: manual / visual; optional snapshot not required

**Interfaces:**
- Produces: CSS variables `--bg`, `--bg-grouped`, `--bg-row`, `--separator`, `--text`, `--text-secondary`, `--accent`, `--danger`, `--ok` under `:root` and `@media (prefers-color-scheme: dark)`
- Consumes: none

- [ ] **Step 1: Replace fixed purple tokens in `index.css`**

Replace the `:root` block and body rules with:

```css
* {
  box-sizing: border-box;
}

:root {
  /* Light — System Settings–adjacent */
  --bg: #f5f5f7;
  --bg-sidebar: rgba(246, 246, 246, 0.92);
  --bg-grouped: #ffffff;
  --bg-row-hover: rgba(0, 0, 0, 0.04);
  --separator: rgba(0, 0, 0, 0.08);
  --text: #1d1d1f;
  --text-secondary: #6e6e73;
  --accent: #007aff;
  --accent-hover: #0066d6;
  --danger: #ff3b30;
  --ok: #34c759;
  --control-fill: rgba(120, 120, 128, 0.16);
  --hud-bg: rgba(255, 255, 255, 0.72);
  --hud-text: #1d1d1f;
  --hud-border: rgba(0, 0, 0, 0.08);
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1e1e1e;
    --bg-sidebar: rgba(40, 40, 40, 0.92);
    --bg-grouped: #2a2a2a;
    --bg-row-hover: rgba(255, 255, 255, 0.06);
    --separator: rgba(255, 255, 255, 0.1);
    --text: #f5f5f7;
    --text-secondary: #a1a1a6;
    --accent: #0a84ff;
    --accent-hover: #409cff;
    --danger: #ff453a;
    --ok: #30d158;
    --control-fill: rgba(120, 120, 128, 0.32);
    --hud-bg: rgba(40, 40, 40, 0.78);
    --hud-text: #f5f5f7;
    --hud-border: rgba(255, 255, 255, 0.12);
  }
}

html,
body,
#root {
  margin: 0;
  padding: 0;
  height: 100%;
}

body {
  font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI",
    Roboto, sans-serif;
  -webkit-font-smoothing: antialiased;
  color: var(--text);
  background: var(--bg);
  cursor: default;
}

body[data-mode="settings"] {
  user-select: text;
}

body[data-mode="overlay"] {
  background: transparent;
  overflow: hidden;
  user-select: none;
}

button {
  font: inherit;
}
```

- [ ] **Step 2: Set `data-mode` on body from `App.tsx` / `main.tsx`**

In `apps/desktop/src/App.tsx` (or wherever mode is known), ensure:

```tsx
useEffect(() => {
  document.body.dataset.mode = mode; // "settings" | "overlay"
}, [mode]);
```

If `App.tsx` already branches on `mode`, add the effect there.

- [ ] **Step 3: Neutral window background in `tauri.conf.json`**

Set main window `backgroundColor` to `"#F5F5F7"` (light system gray). Dark users may see a brief flash until CSS applies — acceptable for this iteration (no vibrancy dependency).

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src/index.css apps/desktop/src/App.tsx apps/desktop/src-tauri/tauri.conf.json
git commit -m "$(cat <<'EOF'
Add macOS Light/Dark design tokens for Settings and HUD.

EOF
)"
```

---

### Task 2: Settings shell — sidebar + grouped list restyle

**Files:**
- Create: `apps/desktop/src/settings/settingsChrome.module.css`
- Create: `apps/desktop/src/settings/SettingsShell.tsx`
- Modify: `apps/desktop/src/settings/SettingsView.tsx`
- Modify: `apps/desktop/src/settings/SettingsView.module.css`
- Modify: `apps/desktop/src/settings/DictionarySection.module.css` (replace old `--bg-card` / purple refs with new tokens)
- Modify: `apps/desktop/src/locales/en/common.json`, `apps/desktop/src/locales/ru/common.json`

**Interfaces:**
- Produces: `SettingsShell` props `{ section: SettingsSection; onSectionChange: (s: SettingsSection) => void; children: React.ReactNode }`
- Produces: `export type SettingsSection = "general" | "model" | "hotkey" | "vocabulary" | "permissions"`
- Consumes: tokens from Task 1; existing settings data hooks stay in `SettingsView`

- [ ] **Step 1: Add i18n keys**

In `en/common.json` add:

```json
"nav": {
  "general": "General",
  "model": "Model",
  "hotkey": "Hotkey",
  "vocabulary": "Vocabulary",
  "permissions": "Permissions"
}
```

Mirror in `ru/common.json` (Русский equivalents: «Основные», «Модель», «Хоткей», «Словарь», «Разрешения»).

- [ ] **Step 2: Create `settingsChrome.module.css`**

Include at minimum:

```css
.shell {
  display: flex;
  height: 100%;
  background: var(--bg);
}

.sidebar {
  width: 200px;
  flex-shrink: 0;
  padding: 16px 10px;
  background: var(--bg-sidebar);
  border-right: 1px solid var(--separator);
}

.navButton {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--text);
  font-size: 13px;
  text-align: left;
  cursor: default;
}

.navButtonActive {
  background: var(--accent);
  color: #fff;
}

.content {
  flex: 1;
  overflow: auto;
  padding: 28px 32px 48px;
}

.paneTitle {
  margin: 0 0 16px;
  font-size: 22px;
  font-weight: 700;
  letter-spacing: -0.02em;
}

.group {
  background: var(--bg-grouped);
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 0 0 1px var(--separator);
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 11px 14px;
  min-height: 44px;
}

.row + .row {
  border-top: 1px solid var(--separator);
}

.rowLabel {
  font-size: 13px;
  font-weight: 500;
}

.rowDetail {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-secondary);
}

.buttonPrimary {
  border: 0;
  border-radius: 6px;
  padding: 6px 12px;
  background: var(--accent);
  color: #fff;
  font-size: 13px;
  font-weight: 500;
}

.buttonPrimary:hover {
  background: var(--accent-hover);
}

.buttonSecondary {
  border: 0;
  border-radius: 6px;
  padding: 6px 12px;
  background: var(--control-fill);
  color: var(--text);
  font-size: 13px;
}

.toggle {
  /* 40×22 track, white knob — AppKit-like; use accent when on */
}
```

Implement `.toggle` / `.toggleOn` / `.toggleKnob` similar to current sizes but with `--accent` when on (no purple).

- [ ] **Step 3: Create `SettingsShell.tsx`**

```tsx
import { useTranslation } from "react-i18next";
import chrome from "./settingsChrome.module.css";

export type SettingsSection =
  | "general"
  | "model"
  | "hotkey"
  | "vocabulary"
  | "permissions";

const SECTIONS: SettingsSection[] = [
  "general",
  "model",
  "hotkey",
  "vocabulary",
  "permissions",
];

type Props = {
  section: SettingsSection;
  onSectionChange: (section: SettingsSection) => void;
  children: React.ReactNode;
};

export default function SettingsShell({
  section,
  onSectionChange,
  children,
}: Props) {
  const { t } = useTranslation("common");
  return (
    <div className={chrome.shell}>
      <nav className={chrome.sidebar} aria-label="Settings">
        {SECTIONS.map((id) => (
          <button
            key={id}
            type="button"
            className={`${chrome.navButton} ${
              section === id ? chrome.navButtonActive : ""
            }`}
            onClick={() => onSectionChange(id)}
          >
            {t(`nav.${id}`)}
          </button>
        ))}
      </nav>
      <main className={chrome.content}>{children}</main>
    </div>
  );
}
```

- [ ] **Step 4: Refactor `SettingsView.tsx` to use shell + section panes**

- Keep existing data fetching / save / permissions / models logic.
- Add `const [section, setSection] = useState<SettingsSection>("general")`.
- Remove glowing `.logoDot` header and gradient `.hint` banner from the default chrome (optional one-line subtitle under pane title is OK).
- Render only the active section’s grouped rows inside `SettingsShell`.
- Map old sections:
  - General → UI language, speech language, autostart
  - Model → model list download/delete/select
  - Hotkey → HotkeyRecorder row
  - Vocabulary → `DictionarySection`
  - Permissions → mic / accessibility rows

Replace classNames that referenced old purple styles with `settingsChrome.module.css` (import as `chrome`).

- [ ] **Step 5: Update `DictionarySection.module.css` to use new tokens**

Replace any `--bg-card`, `--accent` purple assumptions with `var(--bg-grouped)`, `var(--accent)`, `var(--separator)`, `var(--text-secondary)`.

- [ ] **Step 6: Smoke-check in browser / `pnpm desktop:dev`**

Expected: sidebar navigates; Light/Dark follow system; no purple glow; text selectable.

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src/settings apps/desktop/src/locales
git commit -m "$(cat <<'EOF'
Restyle Settings as a System Settings sidebar shell.

EOF
)"
```

---

### Task 3: Quiet start + localized tray + open-settings helpers

**Files:**
- Modify: `apps/desktop/src-tauri/src/tray.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/src/settings.rs`
- Modify: `apps/desktop/src/lib/ipc.ts`
- Test: unit test in `settings.rs` for default `onboarding_completed: false`

**Interfaces:**
- Produces: `AppSettings.onboarding_completed: bool` (serde `onboardingCompleted`)
- Produces: `tray::show_settings(app)` (existing) + `tray::menu_labels(ui_language: &str) -> (&'static str, &'static str)`
- Produces: on Ready — show settings only if `!onboarding_completed || !models::any_downloaded()` (helper)

- [ ] **Step 1: Write failing test for default settings**

In `apps/desktop/src-tauri/src/settings.rs` `#[cfg(test)]`:

```rust
#[test]
fn default_onboarding_completed_is_false() {
    let s = AppSettings::default();
    assert!(!s.onboarding_completed);
}

#[test]
fn missing_onboarding_field_deserializes_to_false() {
    let s: AppSettings = serde_json::from_str(r#"{
        "hotkey": "Alt+Space",
        "modelId": "base",
        "language": "auto",
        "uiLanguage": "auto"
    }"#)
    .unwrap();
    assert!(!s.onboarding_completed);
}
```

- [ ] **Step 2: Run test — expect FAIL (field missing)**

```bash
cd apps/desktop/src-tauri && cargo test default_onboarding_completed_is_false -- --nocapture
```

Expected: compile error or fail until field exists.

- [ ] **Step 3: Add field to `AppSettings`**

```rust
pub struct AppSettings {
    pub hotkey: String,
    pub model_id: String,
    pub language: String,
    pub ui_language: String,
    /// Set after first-run onboarding finishes or after minimum readiness dismiss.
    pub onboarding_completed: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            hotkey: "Alt+Space".into(),
            model_id: "base".into(),
            language: "auto".into(),
            ui_language: "auto".into(),
            onboarding_completed: false,
        }
    }
}
```

Update `apps/desktop/src/lib/ipc.ts`:

```ts
export type AppSettings = {
  hotkey: string;
  modelId: string;
  language: string;
  uiLanguage: string;
  onboardingCompleted: boolean;
};
```

- [ ] **Step 4: Run tests — expect PASS**

```bash
cd apps/desktop/src-tauri && cargo test default_onboarding_completed_is_false missing_onboarding_field_deserializes_to_false -- --nocapture
```

- [ ] **Step 5: Localize tray menu**

In `tray.rs`:

```rust
fn tray_strings(ui_language: &str) -> (&'static str, &'static str) {
    match ui_language {
        "ru" => ("Настройки…", "Выйти из Dictosaurus"),
        _ => ("Settings…", "Quit Dictosaurus"),
    }
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let ui_language = app
        .state::<crate::AppState>()
        .settings
        .lock()
        .ok()
        .map(|s| s.current().ui_language.clone())
        .unwrap_or_else(|| "auto".into());
    let resolved = resolve_tray_language(&ui_language);
    let (settings_label, quit_label) = tray_strings(resolved);
    // ... build menu with settings_label / quit_label
}
```

Add `resolve_tray_language`: if `auto`, use `os_locale` / `sys_locale` if already a dependency, else default `"en"`. Prefer existing i18n approach in the repo — if none on Rust side, map `auto` → `"en"` for tray for this task (document limitation) OR read `AppleLanguages` — keep KISS: `auto` → `"en"` unless `LANG` starts with `ru`.

- [ ] **Step 6: Quiet `RunEvent::Ready`**

In `lib.rs`, replace unconditional `tray::show_settings(app)` with:

```rust
if let tauri::RunEvent::Ready = event {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().current().clone();
    let needs_setup =
        !settings.onboarding_completed || !crate::models::any_model_downloaded();
    if needs_setup {
        tray::show_settings(app);
    }
}
```

Add helper in `models.rs`:

```rust
pub fn any_model_downloaded() -> bool {
    curated().iter().any(|def| resolve_paths(def).is_some())
}
```

- [ ] **Step 7: Commit**

```bash
git add apps/desktop/src-tauri/src/settings.rs apps/desktop/src-tauri/src/tray.rs \
  apps/desktop/src-tauri/src/lib.rs apps/desktop/src-tauri/src/models.rs \
  apps/desktop/src/lib/ipc.ts
git commit -m "$(cat <<'EOF'
Add onboarding flag, quiet tray launch, and localized tray labels.

EOF
)"
```

---

### Task 4: Onboarding UI + deep-link section

**Files:**
- Create: `apps/desktop/src/settings/OnboardingView.tsx`
- Create: `apps/desktop/src/settings/OnboardingView.module.css`
- Modify: `apps/desktop/src/settings/SettingsView.tsx`
- Modify: `apps/desktop/src/locales/en/common.json`, `ru/common.json`
- Optional: listen for event `settings-navigate` with section id from Rust

**Interfaces:**
- Consumes: `AppSettings.onboardingCompleted`, models list, permission checkers
- Produces: calls `updateSettings({ ...settings, onboardingCompleted: true })` when finished/dismissed after min readiness
- Produces: optional `export function navigateSettingsSection(section: SettingsSection)` via custom event for dictation deep-link (Task 5)

- [ ] **Step 1: Add onboarding i18n keys** (`en`):

```json
"onboarding": {
  "title": "Welcome to Dictosaurus",
  "stepPermissions": "Permissions",
  "stepModel": "Speech model",
  "stepHotkey": "Hotkey",
  "continue": "Continue",
  "back": "Back",
  "finish": "Get started",
  "skipReady": "Start using Dictosaurus",
  "needModel": "Download a model to continue."
}
```

Add Russian translations.

- [ ] **Step 2: Implement `OnboardingView`**

3 steps (local `step` state 0..2):

0. Permissions — reuse grant buttons from Settings permissions rows  
1. Model — show compact list of curated models with Download; disable Continue until `models.some(m => m.downloaded)`  
2. Hotkey — show `HotkeyRecorder` + Finish  

On Finish or SkipReady (only enabled when a model is downloaded):

```ts
await updateSettings({ ...settings, onboardingCompleted: true });
```

- [ ] **Step 3: Gate `SettingsView`**

```tsx
if (settings && !settings.onboardingCompleted && !models.some((m) => m.downloaded)) {
  return (
    <OnboardingView
      settings={settings}
      models={models}
      /* permissions + save handlers */
      onCompleted={(next) => setSettings(next)}
    />
  );
}
```

Also show onboarding when `!onboardingCompleted` even if a model exists (user never finished wizard) — allow SkipReady on step 0/1 only when model downloaded.

- [ ] **Step 4: Support deep-link section**

In `SettingsView`:

```tsx
useEffect(() => {
  const un = listen<SettingsSection>("settings-open-section", (e) => {
    setSection(e.payload);
  });
  return () => {
    un.then((f) => f());
  };
}, []);
```

- [ ] **Step 5: Manual verify**

Delete `settings.json` onboarding flag / use fresh data dir → app opens onboarding → download model → finish → relaunch stays in tray.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/settings apps/desktop/src/locales
git commit -m "$(cat <<'EOF'
Add first-run onboarding for permissions, model, and hotkey.

EOF
)"
```

---

### Task 5: Hotkey readiness routing

**Files:**
- Modify: `apps/desktop/src-tauri/src/dictation.rs` (`hotkey_pressed`)
- Modify: `apps/desktop/src-tauri/src/tray.rs` (emit section event after show)
- Modify: `apps/desktop/src-tauri/src/lib.rs` if a small helper is cleaner

**Interfaces:**
- Consumes: `models::is_downloaded`, permission checks if available from Rust; otherwise open Settings Permissions and let UI explain
- Produces: `tray::show_settings_section(app, section: &str)` emitting `settings-open-section`

- [ ] **Step 1: Add `show_settings_section`**

```rust
pub fn show_settings_section(app: &AppHandle, section: &str) {
    show_settings(app);
    let _ = app.emit("settings-open-section", section);
}
```

(`section` values: `"model" | "permissions" | "hotkey" | ..."` matching `SettingsSection`)

- [ ] **Step 2: Change `hotkey_pressed` early exits**

Replace “no model → error overlay flash” with:

```rust
if !models::is_downloaded(&settings.model_id) {
    // Prefer any downloaded model only for preview; for start we still require
    // the selected model OR any model — per spec: open Model setup.
    if !models::any_model_downloaded() {
        crate::tray::show_settings_section(app, "model");
        return;
    }
    // selected model missing but another exists — still open Model pane
    crate::tray::show_settings_section(app, "model");
    return;
}
```

Keep behavior: do not start recording without the configured model downloaded (current check). Opening Model pane is the UX fix.

For permissions: if `tauri-plugin-macos-permissions` is only on the frontend, skip Rust-side mic check in this task unless an existing Rust API exists. Spec requires routing when missing — if only JS can check, emit an event `dictation-needs-permissions` that Settings listens to and switches section. Prefer:

```rust
// After model check, on macOS, optionally best-effort; else rely on capture failure.
```

Minimal acceptable: on microphone start failure, `show_settings_section(app, "permissions")` instead of generic error-only flash.

Update the `audio::start_recording` `Err` branch accordingly.

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src-tauri/src/dictation.rs apps/desktop/src-tauri/src/tray.rs
git commit -m "$(cat <<'EOF'
Open Settings to the right pane when dictation setup is incomplete.

EOF
)"
```

---

### Task 6: Frosted HUD overlay (remove mascot)

**Files:**
- Modify: `apps/desktop/src/overlay/Overlay.tsx`
- Modify: `apps/desktop/src/overlay/Overlay.module.css`
- Modify: `apps/desktop/src-tauri/src/overlay.rs` (`WIDTH`/`HEIGHT`)
- Delete: `apps/desktop/src/overlay/Mascot.tsx`, `Mascot.module.css`, `apps/desktop/src/assets/mascot/*`
- Modify: `apps/desktop/src/locales/en/common.json`, `ru/common.json` if status strings need tweaks

**Interfaces:**
- Consumes: existing `dictation-state` / `audio-level` events
- Produces: HUD with pulse dot scaled by level; no `Mascot` import

- [ ] **Step 1: Resize overlay window**

In `overlay.rs`:

```rust
const WIDTH: f64 = 420.0;
const HEIGHT: f64 = 120.0;
```

Update comments to describe HUD capsule (pulse + 2–3 lines text).

- [ ] **Step 2: Rewrite `Overlay.tsx`**

```tsx
export default function Overlay() {
  const { t } = useTranslation("common");
  const [phase, setPhase] = useState<DictationPhase>("idle");
  const [previewText, setPreviewText] = useState("");
  const [level, setLevel] = useState(0);

  useEffect(() => {
    const unState = listen<DictationState>("dictation-state", (e) => {
      setPhase(e.payload.phase);
      setPreviewText(
        e.payload.phase === "recording" && e.payload.text ? e.payload.text : "",
      );
    });
    const unLevel = listen<number>("audio-level", (e) => setLevel(e.payload));
    return () => {
      unState.then((f) => f());
      unLevel.then((f) => f());
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
          style={{ transform: `scale(${0.75 + pulse * 0.6})`, opacity: 0.5 + pulse * 0.5 }}
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
```

- [ ] **Step 3: HUD CSS**

```css
.root {
  height: 100%;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-bottom: 12px;
}

.hud {
  display: flex;
  align-items: center;
  gap: 10px;
  max-width: 400px;
  min-height: 44px;
  padding: 10px 14px;
  border-radius: 16px;
  background: var(--hud-bg);
  border: 1px solid var(--hud-border);
  backdrop-filter: blur(18px);
  -webkit-backdrop-filter: blur(18px);
  color: var(--hud-text);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
  animation: hudIn 220ms ease-out;
}

@keyframes hudIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

.pulse {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--accent);
  flex-shrink: 0;
}

.previewViewport {
  max-height: 3.9em;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  -webkit-mask-image: linear-gradient(to top, #000 55%, transparent 100%);
  mask-image: linear-gradient(to top, #000 55%, transparent 100%);
}

.previewText,
.status {
  font-size: 13px;
  line-height: 1.3;
  font-weight: 510;
  overflow-wrap: anywhere;
}
```

- [ ] **Step 4: Delete mascot files**

Remove `Mascot.tsx`, `Mascot.module.css`, and `src/assets/mascot/*`. Grep for `Mascot` / `mascot` and clear references.

- [ ] **Step 5: Manual dictate test**

Expected: bottom frosted pill; pulse reacts to voice; preview text; no dinosaur; hide after insert.

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src/overlay apps/desktop/src/assets apps/desktop/src-tauri/src/overlay.rs apps/desktop/src/locales
git commit -m "$(cat <<'EOF'
Replace dictation mascot with a frosted system HUD.

EOF
)"
```

---

### Task 7: Verification pass

**Files:** none new — verify only

- [ ] **Step 1: Run backend tests**

```bash
cd apps/desktop/src-tauri && cargo test
```

Expected: all pass (including onboarding serde tests).

- [ ] **Step 2: Run frontend tests**

```bash
cd apps/desktop && pnpm test
```

Expected: existing DictionarySection tests still pass (update selectors if class names broke them).

- [ ] **Step 3: Spec checklist**

Confirm against `docs/superpowers/specs/2026-07-16-native-macos-ui-ux-design.md` success criteria 1–5.

- [ ] **Step 4: Final commit only if fixes were needed**

```bash
git commit -m "$(cat <<'EOF'
Fix leftovers from native macOS UI verification.

EOF
)"
```

---

## Spec coverage (self-review)

| Spec requirement | Task |
|---|---|
| System Light/Dark tokens, system blue | Task 1 |
| Sidebar + grouped lists, no purple SaaS | Task 2 |
| Selectable Settings text | Task 1 |
| Localized tray | Task 3 |
| Quiet start after onboarding | Task 3 |
| `onboardingCompleted` + 3-step wizard | Task 3–4 |
| Hotkey → Settings when not ready | Task 5 |
| Frosted HUD, pulse dot, no mascot | Task 6 |
| Overlay window resize | Task 6 |
| Delete mascot assets | Task 6 |
| Keep ASR/VAD backend | (untouched) |

## Placeholder scan

No TBD/TODO left in tasks; tray `auto` language resolution explicitly defaults with a KISS rule in Task 3.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-16-native-macos-ui-ux.md`.

**Two execution options:**

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks  
2. **Inline Execution** — execute tasks in this session with checkpoints  

Which approach?
