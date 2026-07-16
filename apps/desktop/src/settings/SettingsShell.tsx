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
