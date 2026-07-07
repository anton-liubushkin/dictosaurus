import { invoke } from "@tauri-apps/api/core";

export type AppSettings = {
  hotkey: string;
  modelId: string;
  language: string;
  uiLanguage: string;
};

export type ModelEngine = "whisper" | "nemo_ctc" | "nemo_transducer";

export type ModelInfo = {
  id: string;
  label: string;
  sizeLabel: string;
  description: string;
  engine: ModelEngine;
  languages: string;
  downloaded: boolean;
};

export type HfCatalogInfo = {
  /** Unix seconds of the last catalog refresh. */
  generatedAt: number;
  models: ModelInfo[];
};

export type DownloadProgress = {
  modelId: string;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number;
  done: boolean;
  error: string | null;
};

export type DictationPhase =
  | "idle"
  | "recording"
  | "transcribing"
  | "inserted"
  | "error"
  | "canceled";

export type DictationState = {
  phase: DictationPhase;
  text: string | null;
  message: string | null;
};

export const getSettings = () => invoke<AppSettings>("get_settings");
export const updateSettings = (settings: AppSettings) =>
  invoke<void>("update_settings", { settings });
export const listModels = () => invoke<ModelInfo[]>("list_models");
export const listHfModels = () => invoke<HfCatalogInfo | null>("list_hf_models");
export const downloadModel = (modelId: string) => invoke<void>("download_model", { modelId });
export const deleteModel = (modelId: string) => invoke<void>("delete_model", { modelId });
