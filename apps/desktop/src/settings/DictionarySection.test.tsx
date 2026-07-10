import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
import DictionarySection, {
  evaluateEntryDraft,
  formatVariants,
  parseVariants,
} from "./DictionarySection";

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

function entryRow(name: string) {
  return screen.getByRole("group", { name });
}

function variantsInput(group: HTMLElement) {
  return within(group).getByPlaceholderText("variant one, variant two");
}

function termInput(group: HTMLElement) {
  return within(group).getByPlaceholderText("Preferred spelling");
}


describe("evaluateEntryDraft", () => {
  const saved = { term: "Rust", aliases: ["rustlang"] };

  it("requests save for normalized changes", () => {
    expect(evaluateEntryDraft("Rustacean", "Ferris", saved)).toEqual({
      action: "save",
      term: "Rustacean",
      aliases: ["Ferris"],
    });
  });

  it("skips save when normalized values are unchanged", () => {
    expect(evaluateEntryDraft("  Rust  ", " rustlang , ", saved)).toEqual({ action: "noop" });
  });

  it("shows term required when variants exist without a canonical term", () => {
    expect(evaluateEntryDraft("", "ferris", saved)).toEqual({
      action: "hint",
      hint: "termRequired",
    });
  });

  it("allows saving a term without variants", () => {
    expect(evaluateEntryDraft("Rust", "", saved)).toEqual({
      action: "save",
      term: "Rust",
      aliases: [],
      hint: "variantsEmpty",
    });
  });
});

describe("parseVariants", () => {
  it("splits comma-separated values and trims whitespace", () => {
    expect(parseVariants("rustlang,  ferris  , py")).toEqual(["rustlang", "ferris", "py"]);
    expect(parseVariants("one,, two, , three")).toEqual(["one", "two", "three"]);
  });

  it("formats aliases back into comma-separated text", () => {
    expect(formatVariants(["rustlang", "ferris"])).toBe("rustlang, ferris");
  });
});

describe("DictionarySection", () => {
  beforeEach(async () => {
    vi.resetAllMocks();
    updateDictionaryMock.mockImplementation(async (next) => structuredClone(next));
    reloadDictionaryMock.mockResolvedValue({ version: 1, enabled: true, entries: [] });
    resetDictionaryMock.mockResolvedValue();
    await i18n.changeLanguage("en");
  });

  function user() {
    return userEvent.setup();
  }

  it("shows loading and renders the loaded dictionary", async () => {
    let resolveDictionary: ((value: DictionaryDocument) => void) | undefined;
    getDictionaryMock.mockReturnValue(
      new Promise((resolve) => {
        resolveDictionary = resolve;
      }),
    );

    render(<DictionarySection />);

    expect(screen.getByText("Loading custom vocabulary…")).toBeInTheDocument();

    await act(async () => {
      resolveDictionary?.(cloneDocument());
    });

    const row = await screen.findByRole("group", { name: "Rust" });
    expect(variantsInput(row)).toHaveValue("rustlang");
    expect(termInput(row)).toHaveValue("Rust");
  });

  it("adds a row and auto-saves a valid entry after debounce", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());

    render(<DictionarySection />);
    await screen.findByRole("group", { name: "Rust" });

    await user().click(screen.getByRole("button", { name: "Add entry" }));
    const row = screen.getByRole("group", { name: "New entry" });
    await user().type(variantsInput(row), "TS, type script");
    await user().type(termInput(row), "TypeScript");
    await user().tab();

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        version: 1,
        enabled: true,
        entries: [
          document.entries[0],
          {
            id: expect.any(String),
            term: "TypeScript",
            aliases: ["TS", "type script"],
            enabled: true,
          },
        ],
      }),
    );
    expect(await screen.findByRole("group", { name: "TypeScript" })).toBeInTheDocument();
  });

  it("auto-saves on blur without waiting for debounce", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());

    render(<DictionarySection />);
    const row = await screen.findByRole("group", { name: "Rust" });
    await user().clear(termInput(row));
    await user().type(termInput(row), "Rustacean");
    await user().tab();

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        entries: [{ ...document.entries[0], term: "Rustacean" }],
      }),
    );
  });

  it("never rewrites a focused field after a debounced save", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    render(<DictionarySection />);

    const row = await screen.findByRole("group", { name: "Rust" });
    const input = variantsInput(row);
    await user().click(input);
    await user().clear(input);
    await user().type(input, "  ferris  ,  rust lang  ");

    await waitFor(() => expect(updateDictionaryMock).toHaveBeenCalled());

    expect(input).toHaveFocus();
    expect(input).toHaveValue("  ferris  ,  rust lang  ");
  });

  it("does not persist a blank new row", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());

    render(<DictionarySection />);
    await screen.findByRole("group", { name: "Rust" });

    await user().click(screen.getByRole("button", { name: "Add entry" }));
    const row = screen.getByRole("group", { name: "New entry" });
    await user().click(termInput(row));
    await user().tab();

    expect(updateDictionaryMock).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: "New entry" })).not.toBeInTheDocument();
  });

  it("rolls back an optimistic save and displays the backend error", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    updateDictionaryMock.mockRejectedValue(
      dictionaryError("validation", "alias conflict: rustlang"),
    );
    render(<DictionarySection />);

    const row = await screen.findByRole("group", { name: "Rust" });
    await user().clear(variantsInput(row));
    await user().type(variantsInput(row), "Ferris");
    await user().clear(termInput(row));
    await user().type(termInput(row), "Rust Language");
    await user().tab();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "The custom vocabulary contains invalid or conflicting values.",
    );
    expect(alert).toHaveTextContent("alias conflict: rustlang");
    expect(termInput(row)).toHaveValue("Rust Language");
    expect(variantsInput(row)).toHaveValue("Ferris");
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
    render(<DictionarySection />);

    const row = await screen.findByRole("group", { name: "Rust" });
    await user().clear(variantsInput(row));
    await user().type(variantsInput(row), "  rust   lang  , RUST LANG");
    await user().clear(termInput(row));
    await user().type(termInput(row), "  Rust   Language  ");
    await user().tab();

    const canonicalRow = await screen.findByRole("group", { name: "Rust Language" });
    await waitFor(() => {
      expect(termInput(canonicalRow)).toHaveValue("Rust Language");
      expect(variantsInput(canonicalRow)).toHaveValue("rust lang");
    });
  });

  it("resets a dictionary that failed to load", async () => {
    getDictionaryMock
      .mockRejectedValueOnce(dictionaryError("corruptJson", "malformed source"))
      .mockResolvedValueOnce({ version: 1, enabled: true, entries: [] });
    render(<DictionarySection />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The custom vocabulary file is corrupted.",
    );
    await user().click(screen.getByRole("button", { name: "Reset dictionary" }));

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
    render(<DictionarySection />);

    await user().click(await screen.findByRole("button", { name: "Reset dictionary" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("temporary write failure");

    await user().click(screen.getByRole("button", { name: "Reset dictionary" }));

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
    render(<DictionarySection />);

    await user().click(await screen.findByRole("button", { name: "Retry" }));

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
    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    await user().click(await screen.findByRole("button", { name: "Retry" }));
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
    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    await user().click(await screen.findByRole("button", { name: "Reset dictionary" }));
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
    render(
      <StrictMode>
        <DictionarySection />
      </StrictMode>,
    );

    const row = await screen.findByRole("group", { name: "Rust" });
    await user().clear(termInput(row));
    await user().type(termInput(row), "Rustacean");
    await user().tab();
    expect(await screen.findByRole("group", { name: "Rustacean" })).toBeInTheDocument();

    await act(async () => {
      staleLoad.resolve(cloneDocument());
      await staleLoad.promise;
    });

    expect(screen.getByRole("group", { name: "Rustacean" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });

  it("persists the global enable toggle", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    render(<DictionarySection />);

    await user().click(
      await screen.findByRole("switch", { name: "Enable custom vocabulary" }),
    );
    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        enabled: false,
      }),
    );
  });

  it("keeps entry A draft across global toggle and entry B save responses", async () => {
    getDictionaryMock.mockResolvedValue(structuredClone(multiEntryDocument));
    updateDictionaryMock.mockImplementation(async (next) => {
      if (next.enabled === false) {
        return structuredClone({ ...multiEntryDocument, enabled: false });
      }
      return structuredClone(next);
    });
    const interaction = userEvent.setup({ delay: null });

    render(<DictionarySection />);

    const rustRow = await screen.findByRole("group", { name: "Rust" });
    const rustTerm = termInput(rustRow);
    const rustVariants = variantsInput(rustRow);
    fireEvent.input(rustTerm, { target: { value: "Rustacean" } });
    fireEvent.input(rustVariants, { target: { value: "Ferris" } });

    await interaction.click(screen.getByRole("switch", { name: "Enable custom vocabulary" }));
    await waitFor(() =>
      expect(updateDictionaryMock.mock.calls.some((call) => call[0].enabled === false)).toBe(
        true,
      ),
    );
    expect(rustTerm).toHaveValue("Rustacean");
    expect(rustVariants).toHaveValue("Ferris");

    const pythonRow = entryRow("Python");
    const pythonTerm = termInput(pythonRow);
    await interaction.click(pythonTerm);
    fireEvent.input(pythonTerm, { target: { value: "Python Language" } });
    await interaction.tab();
    await waitFor(() =>
      expect(
        updateDictionaryMock.mock.calls.some((call) =>
          call[0].entries.some((entry) => entry.term === "Python Language"),
        ),
      ).toBe(true),
    );

    expect(rustTerm).toHaveValue("Rustacean");
    expect(rustVariants).toHaveValue("Ferris");
  });

  it("deletes an entry by persisting the remaining document", async () => {
    getDictionaryMock.mockResolvedValue(cloneDocument());
    render(<DictionarySection />);

    const row = await screen.findByRole("group", { name: "Rust" });
    await user().click(within(row).getByRole("button", { name: "Delete Rust" }));

    await waitFor(() =>
      expect(updateDictionaryMock).toHaveBeenCalledWith({
        ...document,
        entries: [],
      }),
    );
    expect(screen.queryByRole("group", { name: "Rust" })).not.toBeInTheDocument();
  });
});
