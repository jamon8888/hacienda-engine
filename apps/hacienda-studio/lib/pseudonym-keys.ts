/**
 * A small client-side "known keys" list for pseudonymization key ids — never the
 * passphrase itself, only the id a passphrase was derived against (see
 * `AppConfig.pseudonymKeyId`'s doc comment for why an id needs to be remembered
 * separately from the passphrase that derives its key).
 *
 * This exists so a user doesn't have to retype/remember a key id by hand every time they
 * process a batch or reveal a finding across documents — `PiiPanel.tsx` and
 * `Settings.tsx` both read this list to populate a picker, and record a use here on
 * success. Stored in `localStorage`, not `AppConfig`/IndexedDB: it is pure UI convenience
 * (a label a human chose plus which ids exist), never a secret, and unlike `AppConfig` it
 * should persist across a page reload without being part of the pipeline's own settings
 * object save/restore path.
 *
 * Deliberately out of scope: this is not a key vault. It cannot store, rotate, or revoke
 * actual key material — the key itself never exists anywhere but derived, in memory, from
 * a passphrase the user re-enters each time (see `lib/pseudonymize.ts`'s `deriveKeyHex`).
 * Removing an entry here only forgets a label; it cannot make a key unreadable.
 */

const STORAGE_KEY = "hacienda-studio:pseudonym-keys";

export interface KnownPseudonymKey {
  keyId: string;
  label: string;
  /** ISO timestamp of the most recent successful mint/reveal recorded against this id. */
  lastUsedAt: string;
}

function readAll(): KnownPseudonymKey[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    // Corrupt or foreign localStorage content is not this module's problem to diagnose —
    // treat it the same as "no keys known yet" rather than throwing out of every caller.
    return [];
  }
}

function writeAll(keys: KnownPseudonymKey[]): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(keys));
}

/** Newest-used first, so the picker's most likely choice is also its first option. */
export function listKnownKeys(): KnownPseudonymKey[] {
  return [...readAll()].sort((a, b) => b.lastUsedAt.localeCompare(a.lastUsedAt));
}

/**
 * Record that `keyId` was just used to mint or reveal a token. Adds it to the known-keys
 * list if new (label defaults to the id itself), or just bumps `lastUsedAt` if it's
 * already known — never overwrites a label the user chose in `renameKnownKey`.
 */
export function recordKeyUsage(keyId: string): void {
  const trimmed = keyId.trim();
  if (!trimmed) return;
  const keys = readAll();
  const existing = keys.find((k) => k.keyId === trimmed);
  const now = new Date().toISOString();
  if (existing) {
    existing.lastUsedAt = now;
  } else {
    keys.push({ keyId: trimmed, label: trimmed, lastUsedAt: now });
  }
  writeAll(keys);
}

export function renameKnownKey(keyId: string, label: string): void {
  const keys = readAll();
  const existing = keys.find((k) => k.keyId === keyId);
  if (existing) {
    existing.label = label.trim() || keyId;
    writeAll(keys);
  }
}

/** Forgets the label only — see this module's doc comment for why that's all it can do. */
export function removeKnownKey(keyId: string): void {
  writeAll(readAll().filter((k) => k.keyId !== keyId));
}
