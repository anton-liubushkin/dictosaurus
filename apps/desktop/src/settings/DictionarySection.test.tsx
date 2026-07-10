import { StrictMode } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../i18n/i18n";
import {
  getDictionary,
  reloadDictionary,
  resetDictionary,
  updateDictionary,
  type DictionaryDocument,
  type DictionaryIpcError,
} from "../lib/ipc";
import DictionarySection from "./DictionarySection";

vi.mock("../lib/ipc", () => ({
  getDictionary: vi.fn(),
  reloadDictionary: vi.fn(),
  resetDictionary: vi.fn(),
  updateDictionary: vi.fn(),
}));

const document: DictionaryDocument = {
  version: 1,
  enabled: true,
  entries: [
    {
      id: "rust",
      term: "Rust",
      aliases: ["rustlang"],
      enabled: true,
    },
  ],
};

const multiEntryDocument: DictionaryDocument = {
  ...document,
  entries: [
    document.entries[0],
    {
      id: "python",
      term: "Python",
      aliases: ["py"],
      enabled: true,
    },
  ],
};

const cloneDocument = (value: DictionaryDocument = document) => structuredClone(value);

const getDictionaryMock = vi.mocked(getDictionary);
const reloadDictionaryMock = vi.mocked(reloadDictionary);
const updateDictionaryMock = vi.mocked(updateDictionary);
const resetDictionaryMock = vi.mocked(resetDictionary);

const dictionaryError = (
  code: DictionaryIpcError["code"],
  message: string,
): DictionaryIpcError => ({ code, message });

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("DictionarySection", () => {
  beforeEach(async () => {
    vi.resetAllMocks();
    updateDictionaryMock.mockImplementation(async (next) => structuredClone(next));
    reloadDictionaryMock.mockResolvedValue({ version: 1, enabled: true, entries: [] });
    resetDictionaryMock.mockResolvedValue();
    await i18n.changeLanguage("en");
  });

  it("shows loading and renders the loaded dictionary", async () => {
    let resolveDictionary: ((value: DictionaryDocument) => void) | undefined;
    getDictionaryMock.mockReturnValue(
      new Promise((resolve) => {
        resolveDictionary = resolve;
      }),
    );

    render(<DictionarySection />);

    expect(screen.getByText("Loading custom vocabulary…")).toBeInTheDocument();

    resolveDictionary?.(cloneDocument());

    const entry = await screen.findByRole("group", { name: "Rust" });
    expect(within(entry).getByDisplayValue("Rust")).toBeInTheDocument();
    expect(within(entry).getByDisplayValue("rustlang")).toBeInTheDocument();
  });

  it("adds a valid entry by saving the whole document", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const form = await screen.findByRole("group", { name: "Add entry" });
    await user.type(within(form).getByLabelText("Canonical term"), "TypeScript");
    await user.type(within(form).getByLabelText("Alias"), "TS");
    await user.click(within(form).getByRole("button", { name: "Add entry" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        version: 1,
        enabled: true,
        entries: [
          document.entries[0],
          {
            id: expect.any(String),
            term: "TypeScript",
            aliases: ["TS"],
            enabled: true,
          },
        ],
      }),
    );
    expect(await screen.findByRole("group", { name: "TypeScript" })).toBeInTheDocument();
  });

  it("does not persist a blank entry", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const form = await screen.findByRole("group", { name: "Add entry" });
    await user.click(within(form).getByRole("button", { name: "Add entry" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Canonical term is required.");
    expect(updateDictionaryMock).not.toHaveBeenCalled();
  });

  it("rolls back an optimistic entry save and displays the backend error", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    updateDictionaryMock.mockRejectedValue(
      dictionaryError("validation", "alias conflict: rustlang"),
    );
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    const term = within(entry).getByLabelText("Canonical term");
    const alias = within(entry).getByLabelText("Alias 1");
    await user.clear(term);
    await user.type(term, "Rust Language");
    await user.clear(alias);
    await user.type(alias, "Ferris");
    await user.click(within(entry).getByRole("button", { name: "Save" }));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "The custom vocabulary contains invalid or conflicting values.",
    );
    expect(alert).toHaveTextContent("alias conflict: rustlang");
    expect(screen.getByDisplayValue("Rust Language")).toBeInTheDocument();
    expect(screen.getByDisplayValue("Ferris")).toBeInTheDocument();
    expect(updateDictionaryMock).toHaveBeenCalledWith({
      ...document,
      entries: [{ ...document.entries[0], term: "Rust Language", aliases: ["Ferris"] }],
    });
  });

  it("replaces optimistic values with the canonical document returned by the backend", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    updateDictionaryMock.mockResolvedValue({
      ...document,
      entries: [
        {
          ...document.entries[0],
          term: "Rust Language",
          aliases: ["rust lang"],
        },
      ],
    });
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    const term = within(entry).getByLabelText("Canonical term");
    const alias = within(entry).getByLabelText("Alias 1");
    await user.clear(term);
    await user.type(term, "  Rust   Language  ");
    await user.clear(alias);
    await user.type(alias, "  rust   lang  ");
    await user.click(within(entry).getByRole("button", { name: "Add alias" }));
    await user.type(within(entry).getByLabelText("Alias 2"), "RUST LANG");
    await user.click(within(entry).getByRole("button", { name: "Save" }));

    const canonicalEntry = await screen.findByRole("group", { name: "Rust Language" });
    expect(within(canonicalEntry).getByLabelText("Canonical term")).toHaveValue("Rust Language");
    expect(within(canonicalEntry).getByLabelText("Alias 1")).toHaveValue("rust lang");
    expect(within(canonicalEntry).queryByLabelText("Alias 2")).not.toBeInTheDocument();
  });

  it("resets a dictionary that failed to load", async () => {
    getDictionaryMock
      .mockRejectedValueOnce(dictionaryError("corruptJson", "malformed source"))
      .mockResolvedValueOnce({ version: 1, enabled: true, entries: [] });
    const user = userEvent.setup();

    render(<DictionarySection />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The custom vocabulary file is corrupted.",
    );
    await user.click(screen.getByRole("button", { name: "Reset dictionary" }));

    await waitFor(() => expect(resetDictionaryMock).toHaveBeenCalledOnce());
    expect(await screen.findByRole("switch", { name: "Enable custom vocabulary" })).toBeChecked();
  });

  it("keeps reset available after a transient reset failure", async () => {
    getDictionaryMock
      .mockRejectedValueOnce(dictionaryError("corruptJson", "malformed source"))
      .mockResolvedValueOnce({ version: 1, enabled: true, entries: [] });
    resetDictionaryMock.mockRejectedValueOnce(
      dictionaryError("storage", "temporary write failure"),
    );
    const user = userEvent.setup();

    render(<DictionarySection />);

    await user.click(await screen.findByRole("button", { name: "Reset dictionary" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("temporary write failure");

    const retry = screen.getByRole("button", { name: "Reset dictionary" });
    await user.click(retry);

    expect(await screen.findByRole("switch", { name: "Enable custom vocabulary" })).toBeChecked();
    expect(resetDictionaryMock).toHaveBeenCalledTimes(2);
  });

  it("does not offer a destructive reset for a storage load error", async () => {
    getDictionaryMock.mockRejectedValue(
      dictionaryError("storage", "invalid dictionary JSON but classified storage"),
    );

    render(<DictionarySection />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The custom vocabulary file could not be accessed.",
    );
    expect(screen.queryByRole("button", { name: "Reset dictionary" })).not.toBeInTheDocument();
  });

  it("recovers through Retry after the dictionary file is fixed externally", async () => {
    getDictionaryMock.mockRejectedValue(
      dictionaryError("storage", "open dictionary: permission denied"),
    );
    reloadDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    await user.click(await screen.findByRole("button", { name: "Retry" }));

    expect(await screen.findByRole("group", { name: "Rust" })).toBeInTheDocument();
    expect(reloadDictionaryMock).toHaveBeenCalledOnce();
  });

  it("ignores a stale StrictMode load after Retry succeeds", async () => {
    const staleLoad = deferred<DictionaryDocument>();
    const reloaded: DictionaryDocument = {
      version: 1,
      enabled: true,
      entries: [{ id: "fresh", term: "Fresh", aliases: [], enabled: true }],
    };
    getDictionaryMock
      .mockReturnValueOnce(staleLoad.promise)
      .mockRejectedValueOnce(dictionaryError("storage", "temporary read failure"));
    reloadDictionaryMock.mockResolvedValue(reloaded);
    const user = userEvent.setup();

    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    await user.click(await screen.findByRole("button", { name: "Retry" }));
    expect(await screen.findByRole("group", { name: "Fresh" })).toBeInTheDocument();

    await act(async () => {
      staleLoad.resolve(cloneDocument());
      await staleLoad.promise;
    });

    expect(screen.getByRole("group", { name: "Fresh" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });

  it("ignores a stale StrictMode load after a successful reset and reload", async () => {
    const staleLoad = deferred<DictionaryDocument>();
    const resetDocument: DictionaryDocument = {
      version: 1,
      enabled: true,
      entries: [{ id: "fresh", term: "Fresh", aliases: [], enabled: true }],
    };
    getDictionaryMock
      .mockReturnValueOnce(staleLoad.promise)
      .mockRejectedValueOnce(dictionaryError("corruptJson", "malformed source"))
      .mockResolvedValueOnce(resetDocument);
    const user = userEvent.setup();

    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    await user.click(await screen.findByRole("button", { name: "Reset dictionary" }));
    expect(await screen.findByRole("group", { name: "Fresh" })).toBeInTheDocument();

    await act(async () => {
      staleLoad.resolve(cloneDocument());
      await staleLoad.promise;
    });

    expect(screen.getByRole("group", { name: "Fresh" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });

  it("ignores a stale StrictMode load after a successful save", async () => {
    const staleLoad = deferred<DictionaryDocument>();
    getDictionaryMock
      .mockReturnValueOnce(staleLoad.promise)
      .mockResolvedValueOnce(cloneDocument());
    const user = userEvent.setup();

    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    const entry = await screen.findByRole("group", { name: "Rust" });
    const term = within(entry).getByLabelText("Canonical term");
    await user.clear(term);
    await user.type(term, "Rustacean");
    await user.click(within(entry).getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("group", { name: "Rustacean" })).toBeInTheDocument();

    await act(async () => {
      staleLoad.resolve(cloneDocument());
      await staleLoad.promise;
    });

    expect(screen.getByRole("group", { name: "Rustacean" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });

  it("persists global and per-entry toggles in order", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    await user.click(
      await screen.findByRole("switch", { name: "Enable custom vocabulary" }),
    );
    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenNthCalledWith(1, {
        ...document,
        enabled: false,
      }),
    );

    await user.click(screen.getByRole("switch", { name: "Enable Rust" }));
    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenNthCalledWith(2, {
        ...document,
        enabled: false,
        entries: [{ ...document.entries[0], enabled: false }],
      }),
    );
  });

  it("keeps an unsaved entry draft when toggling that entry", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    const term = within(entry).getByLabelText("Canonical term");
    await user.clear(term);
    await user.type(term, "Rustacean");
    expect(updateDictionaryMock).not.toHaveBeenCalled();

    await user.click(within(entry).getByRole("switch", { name: "Enable Rust" }));
    await waitFor(() => expect(updateDictionaryMock).toHaveBeenCalledTimes(1));
    await user.click(within(entry).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenNthCalledWith(2, {
        ...document,
        entries: [{ ...document.entries[0], term: "Rustacean", enabled: false }],
      }),
    );
  });

  it("keeps entry A draft across global toggle and entry B save responses", async () => {
    getDictionaryMock.mockResolvedValue(structuredClone(multiEntryDocument));
    const user = userEvent.setup();

    render(<DictionarySection />);

    const rustEntry = await screen.findByRole("group", { name: "Rust" });
    const rustTerm = within(rustEntry).getByLabelText("Canonical term");
    const rustAlias = within(rustEntry).getByLabelText("Alias 1");
    await user.clear(rustTerm);
    await user.type(rustTerm, "Rustacean");
    await user.clear(rustAlias);
    await user.type(rustAlias, "Ferris");

    await user.click(screen.getByRole("switch", { name: "Enable custom vocabulary" }));
    await waitFor(() => expect(updateDictionaryMock).toHaveBeenCalledTimes(1));
    expect(rustTerm).toHaveValue("Rustacean");
    expect(rustAlias).toHaveValue("Ferris");

    const pythonEntry = screen.getByRole("group", { name: "Python" });
    const pythonTerm = within(pythonEntry).getByLabelText("Canonical term");
    await user.clear(pythonTerm);
    await user.type(pythonTerm, "Python Language");
    await user.click(within(pythonEntry).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(updateDictionaryMock).toHaveBeenCalledTimes(2));

    expect(rustTerm).toHaveValue("Rustacean");
    expect(rustAlias).toHaveValue("Ferris");
  });

  it("adds an alias to the draft and persists it only on Save", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    await user.click(within(entry).getByRole("button", { name: "Add alias" }));
    await user.type(within(entry).getByLabelText("Alias 2"), "Ferris");
    expect(updateDictionaryMock).not.toHaveBeenCalled();
    await user.click(within(entry).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        entries: [{ ...document.entries[0], aliases: ["rustlang", "Ferris"] }],
      }),
    );
  });

  it("removes an alias from the draft and persists it only on Save", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    await user.click(
      within(entry).getByRole("button", { name: "Remove alias 1 from Rust" }),
    );
    expect(updateDictionaryMock).not.toHaveBeenCalled();
    await user.click(within(entry).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        entries: [{ ...document.entries[0], aliases: [] }],
      }),
    );
  });

  it("deletes an entry by persisting the remaining document", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    const user = userEvent.setup();

    render(<DictionarySection />);

    const entry = await screen.findByRole("group", { name: "Rust" });
    await user.click(within(entry).getByRole("button", { name: "Delete" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        entries: [],
      }),
    );
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });
});
