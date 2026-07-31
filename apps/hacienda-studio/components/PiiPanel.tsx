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
 */
import { useState } from "react";
import type { PiiEntity } from "../lib/pii-engine";
import { deriveKeyHex, revealToken } from "../lib/pseudonymize";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "./ui/card";
import { Input } from "./ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "./ui/popover";

export function PiiPanel({ findings }: { findings: PiiEntity[] }) {
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
                <Badge variant="destructive">{finding.category}</Badge>
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

  async function onReveal() {
    setBusy(true);
    setError(null);
    try {
      const keyHex = await deriveKeyHex(passphrase, keyId);
      const value = await revealToken(finding.redact_template, keyId, keyHex);
      if (value === null) {
        setError("Wrong passphrase, wrong key id, or this document used mask mode.");
      } else {
        setRevealed(value);
      }
    } catch {
      setError("Wrong passphrase, wrong key id, or this document used mask mode.");
    } finally {
      setBusy(false);
      setPassphrase("");
    }
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
            />
            <Input
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              placeholder="Passphrase"
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
              {busy ? "Revealing…" : "Reveal"}
            </Button>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
}
