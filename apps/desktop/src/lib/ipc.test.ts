import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getDictionary,
  reloadDictionary,
  resetDictionary,
  updateDictionary,
  type DictionaryDocument,
  type DictionaryIpcError,
} from "./ipc";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

describe("dictionary IPC", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("uses the get_dictionary command", async () => {
    const document: DictionaryDocument = { version: 1, enabled: true, entries: [] };
    invokeMock.mockResolvedValue(structuredClone(document));

    await expect(getDictionary()).resolves.toEqual(document);
    expect(invokeMock).toHaveBeenCalledWith("get_dictionary");
  });

  it("passes the whole document to update_dictionary", async () => {
    const document: DictionaryDocument = { version: 1, enabled: false, entries: [] };
    invokeMock.mockResolvedValue(structuredClone(document));

    await expect(updateDictionary(document)).resolves.toEqual(document);

    expect(invokeMock).toHaveBeenCalledWith("update_dictionary", { document });
  });

  it("uses the reset_dictionary command", async () => {
    invokeMock.mockResolvedValue(undefined);

    await resetDictionary();

    expect(invokeMock).toHaveBeenCalledWith("reset_dictionary");
  });

  it("uses the reload_dictionary command", async () => {
    const document: DictionaryDocument = { version: 1, enabled: true, entries: [] };
    invokeMock.mockResolvedValue(structuredClone(document));

    await expect(reloadDictionary()).resolves.toEqual(document);
    expect(invokeMock).toHaveBeenCalledWith("reload_dictionary");
  });

  it("preserves a valid typed dictionary rejection", async () => {
    const error: DictionaryIpcError = {
      code: "corruptJson",
      message: "invalid dictionary JSON",
    };
    invokeMock.mockRejectedValue(error);

    await expect(getDictionary()).rejects.toEqual(error);
  });

  it("normalizes malformed dictionary rejections to internal", async () => {
    invokeMock.mockRejectedValue("legacy string error");

    await expect(updateDictionary({ version: 1, enabled: true, entries: [] })).rejects.toEqual({
      code: "internal",
      message: "legacy string error",
    });
  });
});
