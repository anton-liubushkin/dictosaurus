import { invoke } from "@tauri-apps/api/core";

export type AppSettings = {
  hotkey: string;
  modelId: string;
  language: string;
  uiLanguage: string;
};

export type DictionaryEntry = {
  id: string;
  term: string;
  aliases: string[];
  enabled: boolean;
};

export type DictionaryDocument = {
  version: number;
  enabled: boolean;
  entries: DictionaryEntry[];
};

const DICTIONARY_ERROR_CODES = [
  "corruptJson",
  "unsupportedVersion",
  "storage",
  "validation",
  "unavailable",
  "internal",
] as const;

export type DictionaryErrorCode = (typeof DICTIONARY_ERROR_CODES)[number];

export type DictionaryIpcError = {
  code: DictionaryErrorCode;
  message: string;
};

function normalizeDictionaryError(error: unknown): DictionaryIpcError {
  if (typeof error === "object" && error !== null) {
    const candidate = error as { code?: unknown; message?: unknown };
    if (
      typeof candidate.code === "string" &&
      DICTIONARY_ERROR_CODES.includes(candidate.code as DictionaryErrorCode) &&
      typeof candidate.message === "string"
    ) {
      return {
        code: candidate.code as DictionaryErrorCode,
        message: candidate.message,
      };
    }
    if (typeof candidate.message === "string") {
      return { code: "internal", message: candidate.message };
    }
  }
  if (typeof error === "string") return { code: "internal", message: error };
  return { code: "internal", message: "Unknown dictionary error" };
}

async function invokeDictionary<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return args === undefined
      ? await invoke<T>(command)
      : await invoke<T>(command, args);
  } catch (error) {
    throw normalizeDictionaryError(error);
  }
}

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
export const getDictionary = () => invokeDictionary<DictionaryDocument>("get_dictionary");
export const updateDictionary = (document: DictionaryDocument) =>
  invokeDictionary<DictionaryDocument>("update_dictionary", { document });
export const resetDictionary = () => invokeDictionary<void>("reset_dictionary");
export const reloadDictionary = () =>
  invokeDictionary<DictionaryDocument>("reload_dictionary");
export const listModels = () => invoke<ModelInfo[]>("list_models");
export const downloadModel = (modelId: string) => invoke<void>("download_model", { modelId });
export const deleteModel = (modelId: string) => invoke<void>("delete_model", { modelId });
