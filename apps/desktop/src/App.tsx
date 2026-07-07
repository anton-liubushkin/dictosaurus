import { useEffect } from "react";
import Overlay from "./overlay/Overlay";
import SettingsView from "./settings/SettingsView";

const mode = new URLSearchParams(window.location.search).get("mode") ?? "settings";

export default function App() {
  useEffect(() => {
    document.body.dataset.mode = mode;
  }, []);

  return mode === "overlay" ? <Overlay /> : <SettingsView />;
}
