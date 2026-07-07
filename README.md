# Dictosaurus

Local AI dictation for desktop. Hold a hotkey, speak, release — the transcribed
text is inserted into the active input and copied to the clipboard. Speech
recognition runs fully on-device (whisper.cpp with Metal acceleration).

## Monorepo layout

- `apps/desktop` — Tauri 2 + React desktop app (macOS Apple Silicon first;
  macOS Intel and Windows 10+ planned)
- `apps/marketing-site` — marketing website (planned)

## Development

Prerequisites: Rust (stable), pnpm 9, Xcode Command Line Tools, CMake
(`brew install cmake` — required to build whisper.cpp).

```bash
pnpm install
pnpm desktop:dev
```

## How it works

1. The app lives in the menu bar (tray). The settings window lets you download
   Whisper models, pick the active model, language and the push-to-talk hotkey.
2. Holding the hotkey records the microphone and shows a voice-reactive orb
   overlay (an NSPanel on macOS, so it renders above fullscreen apps too).
3. On release the audio is transcribed locally, the text is copied to the
   clipboard and pasted into the focused input via a synthetic Cmd+V.

macOS permissions required: Microphone (recording) and Accessibility
(synthetic paste). Both can be checked/requested from the settings window.
