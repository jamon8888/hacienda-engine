/**
 * Track F1: adopts hacienda-private's `PiiPanel.tsx` UX pattern — detected spans render as
 * tokens with a category `Badge`, clicking opens a popup that asks for a passphrase, and
 * the plaintext is shown and forgotten on close, never persisted. Does **not** adopt its
 * data model: hacienda-private's `PiiEntity`/`rehydrateSpanForUi` assume an opaque,
 * non-reversible redaction token and a "matter passphrase" concept Studio doesn't have.
 * This operates on this app's own `lib/pii-engine.ts` `PiiEntity` and `lib/pseudonymize.ts`
 * (Track F2) instead — real reveal, not a mocked one, when the document was redacted with
 * `redactionMode: "pseudonymize"`.
 *
 * One `Popover`, not hacienda-private's nested Popover-containing-a-Dialog: Radix's Popover
 * and Dialog each run their own outside-click/focus-trap dismissal logic, and nesting one
 * inside the other's content causes the Popover to dismiss itself the instant the Dialog
 * opens (confirmed live — the Dialog became unreachable, not a theoretical concern). Base
 * UI (what hacienda-private actually uses now, not plain Radix — see
 * `components/ui/README.md`) may compose the two more gracefully; plain Radix doesn't, so
 * the passphrase form and the revealed value both live directly in the Popover's content.
 *
 * A finding is only revealable if its `redact_template` actually parses as a pseudonym
 * token (`revealToken` returns `null` for a plain mask like `"[EMAIL]"`, which is not a bug
 * to report here — it means this document used mask mode, not pseudonymize).
 *
 * Track I4: `onRemove`, when given, adds a "Remove" button per finding — for a false
 * positive the detector flagged that isn't actually PII. Removing a finding here only
 * drops it from the array `App.tsx` re-splices on export; it does not restore an entity
 * link that `renderAnnotatedMarkdown` dropped for overlapping this span originally (see
 * that scope note in `App.tsx`'s re-export handler).
 */
import { useState } from "react";
import { recordPiiReveal } from "../lib/pii-engine";
import type { PiiEntity } from "../lib/pii-engine";
import { listKnownKeys, recordKeyUsage } from "../lib/pseudonym-keys";
import { deriveKeyHex, revealToken } from "../lib/pseudonymize";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "./ui/card";
import { Input } from "./ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

export function PiiPanel({
  findings,
  onRemove,
}: {
  findings: PiiEntity[];
  onRemove?: (index: number) => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Detected PII ({findings.length})</CardTitle>
      </CardHeader>
      <CardContent className="max-h-[60vh] overflow-auto">
        {findings.length === 0 ? (
          <p className="text-sm text-muted-foreground">No PII detected.</p>
        ) : (
          <ul className="space-y-2">
            {findings.map((finding, i) => (
              <li key={i} className="flex items-center justify-between gap-2 text-sm">
                <RevealableFinding finding={finding} />
                <div className="flex items-center gap-2">
                  <Badge variant="destructive">{finding.category}</Badge>
                  {onRemove && (
                    <button
                      type="button"
                      className="pii-remove-finding text-xs text-muted-foreground hover:text-destructive"
                      aria-label={`Remove finding ${i + 1}`}
                      onClick={() => onRemove(i)}
                    >
                      ✕
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function RevealableFinding({ finding }: { finding: PiiEntity }) {
  const [keyId, setKeyId] = useState("session");
  const [passphrase, setPassphrase] = useState("");
  const [revealed, setRevealed] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Read once per mount (the popover is remounted each time it opens, so this still picks
  // up keys recorded since the panel last rendered) rather than on every keystroke.
  const [knownKeys] = useState(() => listKnownKeys());

  const CREDENTIAL_ERROR =
    "Wrong passphrase, wrong key id, or this document used mask mode.";

  async function onReveal() {
    setBusy(true);
    setError(null);
    let value: string | null;
    try {
      const keyHex = await deriveKeyHex(passphrase, keyId);
      value = await revealToken(finding.redact_template, keyId, keyHex);
    } catch {
      setError(CREDENTIAL_ERROR);
      setBusy(false);
      setPassphrase("");
      return;
    }

    if (value === null) {
      setError(CREDENTIAL_ERROR);
      setBusy(false);
      setPassphrase("");
      return;
    }

    // The audit write is deliberately *not* inside the credential try block above. It runs
    // only after the token has already been successfully revealed, so a failure here says
    // nothing about the passphrase — reporting it as "wrong passphrase" told the user their
    // credentials were bad when they were correct and only the audit store was unavailable,
    // leaving them retyping a passphrase that was never the problem.
    //
    // Plaintext still stays hidden on an audit failure, matching `redactPii`'s "a failed
    // audit write fails the call" philosophy (`lib/pii-engine.ts`): a compliance feature
    // that can silently reveal PII without recording it is worse than one that refuses.
    try {
      await recordPiiReveal(value, finding.category, finding.source);
    } catch {
      setError(
        "Your passphrase was correct, but the reveal could not be written to the audit chain, so the value is not shown. Try again — revealing PII without an audit record is not permitted.",
      );
      setBusy(false);
      setPassphrase("");
      return;
    }

    setRevealed(value);
    recordKeyUsage(keyId);
    setBusy(false);
    setPassphrase("");
  }

  return (
    <Popover
      onOpenChange={(next) => {
        // Session-only reveal: closing the popover forgets the plaintext, never persisted.
        if (!next) {
          setRevealed(null);
          setError(null);
        }
      }}
    >
      <PopoverTrigger asChild>
        <span className="pii-finding-trigger cursor-pointer truncate rounded bg-muted px-2 py-1 font-mono text-xs">
          {revealed ?? finding.redact_template}
        </span>
      </PopoverTrigger>
      <PopoverContent className="w-64 space-y-2">
        {/* `!== null`, not truthiness: a revealed value can legitimately be an empty
         * string (an empty match), which must still render as "revealed", not fall back
         * to the passphrase form. */}
        {revealed !== null ? (
          <p className="pii-revealed-value break-words text-sm">{revealed}</p>
        ) : (
          <>
            <Input
              value={keyId}
              onChange={(e) => setKeyId(e.target.value)}
              placeholder="Key id (default: session)"
              list="pii-panel-known-keys"
            />
            <datalist id="pii-panel-known-keys">
              {knownKeys.map((k) => (
                <option key={k.keyId} value={k.keyId}>
                  {k.label}
                </option>
              ))}
            </datalist>
            <Input
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              placeholder="Phrase secrète"
              onKeyDown={(e) => {
                if (e.key === "Enter" && passphrase && !busy) onReveal();
              }}
            />
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button
              className="pii-reveal-submit"
              size="sm"
              onClick={onReveal}
              disabled={busy || !passphrase}
            >
              {busy ? "Révélation…" : "Révéler"}
            </Button>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
}
