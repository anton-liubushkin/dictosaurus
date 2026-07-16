import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getDictionary,
  reloadDictionary,
  resetDictionary,
  updateDictionary,
  type DictionaryDocument,
  type DictionaryEntry,
  type DictionaryIpcError,
} from "../lib/ipc";
import sharedStyles from "./SettingsView.module.css";
import styles from "./DictionarySection.module.css";

type LoadState = "loading" | "healthy" | "error";

const SAVE_DEBOUNCE_MS = 400;

let fallbackIdSequence = 0;

function canResetError(error: DictionaryIpcError): boolean {
  return error.code === "corruptJson" || error.code === "unsupportedVersion";
}

function createEntryId(entries: DictionaryEntry[]): string {
  const usedIds = new Set(entries.map((entry) => entry.id));
  let id: string;

  do {
    id =
      typeof globalThis.crypto?.randomUUID === "function"
        ? globalThis.crypto.randomUUID()
        : `entry-${Date.now()}-${fallbackIdSequence++}`;
  } while (usedIds.has(id));

  return id;
}

export function parseVariants(text: string): string[] {
  return text
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

export function formatVariants(aliases: string[]): string {
  return aliases.join(", ");
}

function isBlankEntry(term: string, aliases: string[]): boolean {
  return !term.trim() && aliases.length === 0;
}

function aliasesEqual(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

export type EntryDraftEvaluation =
  | { action: "save"; term: string; aliases: string[]; hint?: "variantsEmpty" }
  | { action: "discard" }
  | { action: "hint"; hint: "termRequired" }
  | { action: "noop" };

export function evaluateEntryDraft(
  term: string,
  variantsText: string,
  saved: Pick<DictionaryEntry, "term" | "aliases">,
): EntryDraftEvaluation {
  const trimmedTerm = term.trim();
  const aliases = parseVariants(variantsText);

  if (isBlankEntry(term, aliases)) {
    return { action: "discard" };
  }

  if (!trimmedTerm) {
    return { action: "hint", hint: "termRequired" };
  }

  if (trimmedTerm === saved.term && aliasesEqual(aliases, saved.aliases)) {
    return { action: "noop" };
  }

  return {
    action: "save",
    term: trimmedTerm,
    aliases,
    ...(aliases.length === 0 ? { hint: "variantsEmpty" as const } : {}),
  };
}

export default function DictionarySection() {
  const { t } = useTranslation("common");
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [document, setDocument] = useState<DictionaryDocument | null>(null);
  const [loadError, setLoadError] = useState<DictionaryIpcError | null>(null);
  const [canReset, setCanReset] = useState(false);
  const [operationError, setOperationError] = useState<DictionaryIpcError | null>(null);
  const [saving, setSaving] = useState(false);
  const documentRef = useRef<DictionaryDocument | null>(null);
  const persistQueueRef = useRef<Promise<DictionaryDocument | null>>(Promise.resolve(null));
  const requestGenerationRef = useRef(0);

  useEffect(() => {
    documentRef.current = document;
  }, [document]);

  const load = useCallback(async () => {
    const generation = ++requestGenerationRef.current;
    setLoadState("loading");
    setLoadError(null);
    try {
      const loaded = await getDictionary();
      if (generation !== requestGenerationRef.current) return;
      setDocument(loaded);
      setCanReset(false);
      setLoadState("healthy");
    } catch (error) {
      if (generation !== requestGenerationRef.current) return;
      const ipcError = error as DictionaryIpcError;
      setDocument(null);
      setLoadError(ipcError);
      setCanReset(canResetError(ipcError));
      setLoadState("error");
    }
  }, []);

  useEffect(() => {
    void load();
    return () => {
      requestGenerationRef.current += 1;
    };
  }, [load]);

  const enqueuePersist = useCallback(
    (
      buildNext: (current: DictionaryDocument) => DictionaryDocument,
    ): Promise<DictionaryDocument | null> => {
      const run = async (): Promise<DictionaryDocument | null> => {
        const current = documentRef.current;
        if (!current) return null;

        const generation = ++requestGenerationRef.current;
        setOperationError(null);

        try {
          const saved = await updateDictionary(buildNext(current));
          if (generation === requestGenerationRef.current) {
            documentRef.current = saved;
            setDocument(saved);
          }
          return saved;
        } catch (error) {
          if (generation === requestGenerationRef.current) {
            setOperationError(error as DictionaryIpcError);
          }
          return null;
        }
      };

      persistQueueRef.current = persistQueueRef.current.then(run, run);
      return persistQueueRef.current;
    },
    [],
  );

  const persist = useCallback(
    async (next: DictionaryDocument) => (await enqueuePersist(() => next)) !== null,
    [enqueuePersist],
  );

  const reset = useCallback(async () => {
    const generation = ++requestGenerationRef.current;
    setSaving(true);
    try {
      await resetDictionary();
      await load();
    } catch (error) {
      if (generation === requestGenerationRef.current) {
        const ipcError = error as DictionaryIpcError;
        setLoadError(ipcError);
        setCanReset((current) => current || canResetError(ipcError));
        setLoadState("error");
      }
    } finally {
      setSaving(false);
    }
  }, [load]);

  const retry = useCallback(async () => {
    const generation = ++requestGenerationRef.current;
    setSaving(true);
    try {
      const reloaded = await reloadDictionary();
      if (generation !== requestGenerationRef.current) return;
      documentRef.current = reloaded;
      setDocument(reloaded);
      setLoadError(null);
      setCanReset(false);
      setLoadState("healthy");
    } catch (error) {
      if (generation === requestGenerationRef.current) {
        const ipcError = error as DictionaryIpcError;
        setLoadError(ipcError);
        setCanReset((current) => current || canResetError(ipcError));
        setLoadState("error");
      }
    } finally {
      setSaving(false);
    }
  }, []);

  const addRow = useCallback(() => {
    if (!document) return;
    const entry: DictionaryEntry = {
      id: createEntryId(document.entries),
      term: "",
      aliases: [],
      enabled: true,
    };
    setDocument({ ...document, entries: [...document.entries, entry] });
  }, [document]);

  const removeRow = useCallback(
    (entryId: string) => {
      if (!document) return;
      setDocument({
        ...document,
        entries: document.entries.filter((item) => item.id !== entryId),
      });
    },
    [document],
  );

  const saveEntry = useCallback(
    async (entryId: string, term: string, aliases: string[]) => {
      const saved = await enqueuePersist((current) => ({
        ...current,
        entries: current.entries.map((item) =>
          item.id === entryId ? { ...item, term, aliases, enabled: true } : item,
        ),
      }));
      return saved?.entries.find((item) => item.id === entryId) ?? null;
    },
    [enqueuePersist],
  );

  const deleteEntry = useCallback(
    (entryId: string) =>
      enqueuePersist((current) => ({
        ...current,
        entries: current.entries.filter((item) => item.id !== entryId),
      })),
    [enqueuePersist],
  );

  return (
    <div className={sharedStyles.card}>
        {loadState === "loading" && (
          <p className={styles.stateMessage}>{t("dictionary.loading")}</p>
        )}

        {loadState === "error" && (
          <div className={styles.statePanel}>
            <p className={styles.error} role="alert">
              {loadError && t(`dictionary.error.${loadError.code}`)}
              {loadError?.message && (
                <span className={styles.diagnostic}>{loadError.message}</span>
              )}
            </p>
            <div className={styles.recoveryActions}>
              <button
                type="button"
                className={sharedStyles.buttonSecondary}
                disabled={saving}
                onClick={() => void retry()}
              >
                {t("dictionary.retry")}
              </button>
              {canReset && (
                <button
                  type="button"
                  className={sharedStyles.buttonSecondary}
                  disabled={saving}
                  onClick={() => void reset()}
                >
                  {t("dictionary.reset")}
                </button>
              )}
            </div>
          </div>
        )}

        {loadState === "healthy" && document && (
          <>
            <div className={sharedStyles.row}>
              <div>
                <div className={sharedStyles.rowLabel}>{t("dictionary.enabled")}</div>
                <div className={sharedStyles.rowDetail}>{t("dictionary.enabledDetail")}</div>
              </div>
              <Switch
                checked={document.enabled}
                disabled={saving}
                label={t("dictionary.enabled")}
                onChange={() => void persist({ ...document, enabled: !document.enabled })}
              />
            </div>

            {operationError && (
              <p className={styles.operationError} role="alert">
                {t(`dictionary.error.${operationError.code}`)}
                {operationError.message && (
                  <span className={styles.diagnostic}>{operationError.message}</span>
                )}
              </p>
            )}

            <div className={styles.table}>
              {document.entries.length > 0 && (
                <div className={styles.tableHeader}>
                  <span>{t("dictionary.variants")}</span>
                  <span aria-hidden="true" />
                  <span>{t("dictionary.term")}</span>
                  <span aria-hidden="true" />
                </div>
              )}

              {document.entries.length === 0 && (
                <p className={styles.empty}>{t("dictionary.empty")}</p>
              )}

              {document.entries.map((entry) => (
                <EntryRow
                  key={entry.id}
                  entry={entry}
                  onSave={(term, aliases) => saveEntry(entry.id, term, aliases)}
                  onDelete={() => void deleteEntry(entry.id)}
                  onDiscard={() => removeRow(entry.id)}
                />
              ))}
            </div>

            <div className={styles.footer}>
              <button
                type="button"
                className={sharedStyles.buttonSecondary}
                disabled={saving}
                onClick={addRow}
              >
                {t("dictionary.add")}
              </button>
            </div>
          </>
        )}
    </div>
  );
}

function Switch({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled: boolean;
  label: string;
  onChange: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`${sharedStyles.toggle} ${checked ? sharedStyles.toggleOn : ""}`}
      disabled={disabled}
      onClick={onChange}
    >
      <span className={sharedStyles.toggleKnob} />
    </button>
  );
}

function EntryRow({
  entry,
  onSave,
  onDelete,
  onDiscard,
}: {
  entry: DictionaryEntry;
  onSave: (term: string, aliases: string[]) => Promise<DictionaryEntry | null>;
  onDelete: () => void;
  onDiscard: () => void;
}) {
  const { t } = useTranslation("common");
  const [rowHint, setRowHint] = useState<"termRequired" | "variantsEmpty" | null>(null);
  const initialDraftRef = useRef({
    term: entry.term,
    variants: formatVariants(entry.aliases),
  });
  const variantsInputRef = useRef<HTMLInputElement>(null);
  const termInputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const focusedFieldRef = useRef<"variants" | "term" | null>(null);
  const hasUnblurredEditsRef = useRef({ variants: false, term: false });
  const draftRef = useRef(initialDraftRef.current);
  const savedEntryRef = useRef({ term: entry.term, aliases: entry.aliases });
  const rowLabel = entry.term.trim() || t("dictionary.newEntry");

  useEffect(() => {
    savedEntryRef.current = { term: entry.term, aliases: entry.aliases };

    const variants = formatVariants(entry.aliases);
    if (
      focusedFieldRef.current !== "variants" &&
      !hasUnblurredEditsRef.current.variants
    ) {
      draftRef.current.variants = variants;
      if (variantsInputRef.current) variantsInputRef.current.value = variants;
    }
    if (focusedFieldRef.current !== "term" && !hasUnblurredEditsRef.current.term) {
      draftRef.current.term = entry.term;
      if (termInputRef.current) termInputRef.current.value = entry.term;
    }
  }, [entry.aliases, entry.term]);

  useEffect(
    () => () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    },
    [],
  );

  const showDraftFeedback = useCallback(
    (term: string, variants: string) => {
      const evaluation = evaluateEntryDraft(term, variants, savedEntryRef.current);
      if (evaluation.action === "hint") {
        setRowHint(evaluation.hint);
        return;
      }
      if (evaluation.action === "save" && evaluation.hint) {
        setRowHint(evaluation.hint);
        return;
      }
      setRowHint(null);
    },
    [],
  );

  const applyEvaluation = useCallback(
    async (
      evaluation: EntryDraftEvaluation,
      options?: { discardBlank?: boolean },
    ): Promise<DictionaryEntry | null> => {
      if (evaluation.action === "discard") {
        setRowHint(null);
        if (options?.discardBlank) onDiscard();
        return null;
      }

      if (evaluation.action === "hint") {
        setRowHint(evaluation.hint);
        return null;
      }

      if (evaluation.action === "noop") {
        setRowHint(null);
        return { ...entry, ...savedEntryRef.current };
      }

      setRowHint(evaluation.hint ?? null);
      const saved = await onSave(evaluation.term, evaluation.aliases);
      if (saved) {
        savedEntryRef.current = { term: saved.term, aliases: saved.aliases };
      }
      return saved;
    },
    [entry, onDiscard, onSave],
  );

  const scheduleSave = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      debounceRef.current = null;
      const draft = draftRef.current;
      void applyEvaluation(
        evaluateEntryDraft(draft.term, draft.variants, savedEntryRef.current),
      );
    }, SAVE_DEBOUNCE_MS);
  }, [applyEvaluation]);

  const changeVariants = (value: string) => {
    draftRef.current = { ...draftRef.current, variants: value };
    hasUnblurredEditsRef.current.variants = true;
    showDraftFeedback(draftRef.current.term, value);
    scheduleSave();
  };

  const changeTerm = (value: string) => {
    draftRef.current = { ...draftRef.current, term: value };
    hasUnblurredEditsRef.current.term = true;
    showDraftFeedback(value, draftRef.current.variants);
    scheduleSave();
  };

  const blurField = async (field: "variants" | "term") => {
    focusedFieldRef.current = null;
    if (debounceRef.current) {
      clearTimeout(debounceRef.current);
      debounceRef.current = null;
    }

    const draft = draftRef.current;
    const evaluation = evaluateEntryDraft(
      draft.term,
      draft.variants,
      savedEntryRef.current,
    );

    if (evaluation.action === "discard") {
      hasUnblurredEditsRef.current[field] = false;
      onDiscard();
      return;
    }

    if (field === "variants") {
      const normalized = formatVariants(parseVariants(draft.variants));
      draftRef.current = { ...draftRef.current, variants: normalized };
      if (variantsInputRef.current) variantsInputRef.current.value = normalized;
    } else {
      const normalized = draft.term.trim();
      draftRef.current = { ...draftRef.current, term: normalized };
      if (termInputRef.current) termInputRef.current.value = normalized;
    }

    hasUnblurredEditsRef.current[field] = false;
    const saved = await applyEvaluation(evaluation, { discardBlank: true });
    if (!saved || focusedFieldRef.current === field || hasUnblurredEditsRef.current[field]) {
      return;
    }

    if (field === "variants") {
      const variants = formatVariants(saved.aliases);
      draftRef.current.variants = variants;
      if (variantsInputRef.current) variantsInputRef.current.value = variants;
    } else {
      draftRef.current.term = saved.term;
      if (termInputRef.current) termInputRef.current.value = saved.term;
    }
  };

  return (
    <div className={styles.row} role="group" aria-label={rowLabel}>
      <label className={styles.cell}>
        <span className={styles.srOnly}>{t("dictionary.variants")}</span>
        <input
          ref={variantsInputRef}
          className={styles.input}
          defaultValue={initialDraftRef.current.variants}
          placeholder={t("dictionary.variantsPlaceholder")}
          onInput={(event) => changeVariants(event.currentTarget.value)}
          onFocus={() => {
            focusedFieldRef.current = "variants";
          }}
          onBlur={() => void blurField("variants")}
        />
      </label>

      <span className={styles.arrow} aria-hidden="true">
        →
      </span>

      <label className={styles.cell}>
        <span className={styles.srOnly}>{t("dictionary.term")}</span>
        <input
          ref={termInputRef}
          className={styles.input}
          defaultValue={initialDraftRef.current.term}
          placeholder={t("dictionary.termPlaceholder")}
          onInput={(event) => changeTerm(event.currentTarget.value)}
          onFocus={() => {
            focusedFieldRef.current = "term";
          }}
          onBlur={() => void blurField("term")}
        />
      </label>

      <button
        type="button"
        className={sharedStyles.buttonGhost}
        aria-label={t("dictionary.deleteEntry", { term: rowLabel })}
        onClick={onDelete}
      >
        {t("dictionary.delete")}
      </button>

      {rowHint && (
        <p
          className={rowHint === "termRequired" ? styles.rowError : styles.rowHint}
          role={rowHint === "termRequired" ? "alert" : "status"}
        >
          {t(`dictionary.${rowHint}`)}
        </p>
      )}
    </div>
  );
}
