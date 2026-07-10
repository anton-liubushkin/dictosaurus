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
type EntryDraft = Pick<DictionaryEntry, "term" | "aliases">;

let fallbackIdSequence = 0;

function draftMatchesEntry(draft: EntryDraft, entry: DictionaryEntry): boolean {
  return (
    draft.term === entry.term &&
    draft.aliases.length === entry.aliases.length &&
    draft.aliases.every((alias, index) => alias === entry.aliases[index])
  );
}

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

export default function DictionarySection() {
  const { t } = useTranslation("common");
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [document, setDocument] = useState<DictionaryDocument | null>(null);
  const [loadError, setLoadError] = useState<DictionaryIpcError | null>(null);
  const [canReset, setCanReset] = useState(false);
  const [operationError, setOperationError] = useState<DictionaryIpcError | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const requestGenerationRef = useRef(0);
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

  const persist = useCallback(
    async (next: DictionaryDocument): Promise<boolean> => {
      if (!document || savingRef.current) return false;

      const previous = document;
      const generation = ++requestGenerationRef.current;
      savingRef.current = true;
      setSaving(true);
      setOperationError(null);
      setDocument(next);

      try {
        const saved = await updateDictionary(next);
        if (generation === requestGenerationRef.current) setDocument(saved);
        return true;
      } catch (error) {
        if (generation === requestGenerationRef.current) {
          setDocument(previous);
          setOperationError(error as DictionaryIpcError);
        }
        return false;
      } finally {
        savingRef.current = false;
        setSaving(false);
      }
    },
    [document],
  );

  const reset = useCallback(async () => {
    if (savingRef.current) return;

    const generation = ++requestGenerationRef.current;
    savingRef.current = true;
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
      savingRef.current = false;
      setSaving(false);
    }
  }, [load]);

  const retry = useCallback(async () => {
    if (savingRef.current) return;

    const generation = ++requestGenerationRef.current;
    savingRef.current = true;
    setSaving(true);
    try {
      const reloaded = await reloadDictionary();
      if (generation !== requestGenerationRef.current) return;
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
      savingRef.current = false;
      setSaving(false);
    }
  }, []);

  const addEntry = useCallback(
    async (term: string, alias: string): Promise<boolean> => {
      if (!document) return false;

      const entry: DictionaryEntry = {
        id: createEntryId(document.entries),
        term: term.trim(),
        aliases: alias.trim() ? [alias.trim()] : [],
        enabled: true,
      };
      return persist({ ...document, entries: [...document.entries, entry] });
    },
    [document, persist],
  );

  return (
    <section className={sharedStyles.section}>
      <h2 className={sharedStyles.sectionTitle}>{t("section.dictionary")}</h2>
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

            <div className={styles.entries}>
              {document.entries.length === 0 && (
                <p className={styles.empty}>{t("dictionary.empty")}</p>
              )}
              {document.entries.map((entry) => (
                <EntryEditor
                  key={entry.id}
                  entry={entry}
                  saving={saving}
                  onSave={(nextEntry) =>
                    persist({
                      ...document,
                      entries: document.entries.map((item) =>
                        item.id === entry.id ? nextEntry : item,
                      ),
                    })
                  }
                  onToggle={() =>
                    void persist({
                      ...document,
                      entries: document.entries.map((item) =>
                        item.id === entry.id ? { ...item, enabled: !item.enabled } : item,
                      ),
                    })
                  }
                  onDelete={() =>
                    void persist({
                      ...document,
                      entries: document.entries.filter((item) => item.id !== entry.id),
                    })
                  }
                />
              ))}
            </div>

            <AddEntryForm disabled={saving} onAdd={addEntry} />
          </>
        )}
      </div>
    </section>
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

function EntryEditor({
  entry,
  saving,
  onSave,
  onToggle,
  onDelete,
}: {
  entry: DictionaryEntry;
  saving: boolean;
  onSave: (entry: DictionaryEntry) => Promise<boolean>;
  onToggle: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation("common");
  const [draft, setDraft] = useState<EntryDraft>({
    term: entry.term,
    aliases: entry.aliases,
  });
  const [dirty, setDirty] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  useEffect(() => {
    if (dirty) return;
    setDraft((current) =>
      draftMatchesEntry(current, entry)
        ? current
        : { term: entry.term, aliases: entry.aliases },
    );
  }, [dirty, entry]);

  const changeDraft = (next: EntryDraft) => {
    setDraft(next);
    setDirty(!draftMatchesEntry(next, entry));
  };

  const save = async () => {
    if (!draft.term.trim()) {
      setValidationError(t("dictionary.termRequired"));
      return;
    }
    if (draft.aliases.some((alias) => !alias.trim())) {
      setValidationError(t("dictionary.aliasRequired"));
      return;
    }

    setValidationError(null);
    const saved = await onSave({
      ...entry,
      term: draft.term.trim(),
      aliases: draft.aliases.map((alias) => alias.trim()),
    });
    if (saved) setDirty(false);
  };

  return (
    <div className={styles.entry} role="group" aria-label={entry.term}>
      <div className={styles.entryHeader}>
        <Switch
          checked={entry.enabled}
          disabled={saving}
          label={t("dictionary.entryEnabled", { term: entry.term })}
          onChange={onToggle}
        />
        <button
          type="button"
          className={sharedStyles.buttonGhost}
          disabled={saving}
          onClick={onDelete}
        >
          {t("dictionary.delete")}
        </button>
      </div>

      <label className={styles.field}>
        <span>{t("dictionary.term")}</span>
        <input
          className={styles.input}
          value={draft.term}
          disabled={saving}
          onChange={(event) => changeDraft({ ...draft, term: event.target.value })}
        />
      </label>

      <div className={styles.aliases}>
        {draft.aliases.map((alias, index) => (
          <div className={styles.aliasRow} key={index}>
            <label className={styles.field}>
              <span>{t("dictionary.aliasNumber", { number: index + 1 })}</span>
              <input
                className={styles.input}
                value={alias}
                disabled={saving}
                onChange={(event) =>
                  changeDraft({
                    ...draft,
                    aliases: draft.aliases.map((item, aliasIndex) =>
                      aliasIndex === index ? event.target.value : item,
                    ),
                  })
                }
              />
            </label>
            <button
              type="button"
              className={styles.iconButton}
              aria-label={t("dictionary.removeAlias", { number: index + 1, term: entry.term })}
              disabled={saving}
              onClick={() =>
                changeDraft({
                  ...draft,
                  aliases: draft.aliases.filter((_, aliasIndex) => aliasIndex !== index),
                })
              }
            >
              &times;
            </button>
          </div>
        ))}
      </div>

      {validationError && (
        <p className={styles.validationError} role="alert">
          {validationError}
        </p>
      )}

      <div className={styles.entryActions}>
        <button
          type="button"
          className={sharedStyles.buttonSecondary}
          disabled={saving}
          onClick={() => changeDraft({ ...draft, aliases: [...draft.aliases, ""] })}
        >
          {t("dictionary.addAlias")}
        </button>
        <button
          type="button"
          className={sharedStyles.buttonPrimary}
          disabled={saving}
          onClick={() => void save()}
        >
          {t("dictionary.save")}
        </button>
      </div>
    </div>
  );
}

function AddEntryForm({
  disabled,
  onAdd,
}: {
  disabled: boolean;
  onAdd: (term: string, alias: string) => Promise<boolean>;
}) {
  const { t } = useTranslation("common");
  const [term, setTerm] = useState("");
  const [alias, setAlias] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);

  const add = async () => {
    if (!term.trim()) {
      setValidationError(t("dictionary.termRequired"));
      return;
    }

    setValidationError(null);
    if (await onAdd(term, alias)) {
      setTerm("");
      setAlias("");
    }
  };

  return (
    <div className={styles.addForm} role="group" aria-label={t("dictionary.add")}>
      <h3 className={styles.addTitle}>{t("dictionary.add")}</h3>
      <label className={styles.field}>
        <span>{t("dictionary.term")}</span>
        <input
          className={styles.input}
          value={term}
          disabled={disabled}
          onChange={(event) => setTerm(event.target.value)}
        />
      </label>
      <label className={styles.field}>
        <span>{t("dictionary.alias")}</span>
        <input
          className={styles.input}
          value={alias}
          disabled={disabled}
          onChange={(event) => setAlias(event.target.value)}
        />
      </label>
      {validationError && (
        <p className={styles.validationError} role="alert">
          {validationError}
        </p>
      )}
      <button
        type="button"
        className={sharedStyles.buttonPrimary}
        disabled={disabled}
        onClick={() => void add()}
      >
        {t("dictionary.add")}
      </button>
    </div>
  );
}
