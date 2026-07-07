import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import Overlay from "./overlay/Overlay";
import SettingsView from "./settings/SettingsView";
import { getSettings } from "./lib/ipc";
import { UI_LANGUAGE_EVENT, applyUiLanguage } from "./i18n/i18n";

const mode = new URLSearchParams(window.location.search).get("mode") ?? "settings";

export default function App() {
  useEffect(() => {
    document.body.dataset.mode = mode;

    getSettings()
      .then((s) => applyUiLanguage(s.uiLanguage))
      .catch(console.error);

    const unlisten = listen<string>(UI_LANGUAGE_EVENT, (event) => {
      applyUiLanguage(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return mode === "overlay" ? <Overlay /> : <SettingsView />;
}
