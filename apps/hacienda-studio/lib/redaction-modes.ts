/**
 * Client-side redaction-mode switching for the document detail view's Mask/Hash/
 * Pseudonymize/Remove toolbar. Recomputes each finding's `redact_template` — the field
 * `lib/annotate.ts`'s `renderAnnotatedMarkdown` splices into the document — rather than
 * re-running the pipeline, so switching modes in the viewer is instant and needs no
 * worker round-trip.
 *
 * Mask, Hash and Remove are pure functions of the finding's own text/category and need
 * nothing the pipeline computed — they can always be produced. Pseudonymize cannot: a
 * real reversible token requires the batch's derived key, which only ever exists inside
 * the worker at process time (see `AppConfig.redactionMode`'s doc comment). Requesting
 * Pseudonymize for a document that wasn't originally processed in that mode returns
 * `unavailable: true` instead of fabricating a token nothing could later reveal.
 */
import type { PiiEntity } from "./pii-engine";

export type RedactionMode = "mask" | "hash" | "pseudonymize" | "remove";

export const REDACTION_MODES: { value: RedactionMode; label: string }[] = [
  { value: "mask", label: "Mask" },
  { value: "hash", label: "Hash" },
  { value: "pseudonymize", label: "Pseudonymize" },
  { value: "remove", label: "Remove" },
];

/**
 * Matches the `[CATEGORY:key_id:...]` shape `AppConfig.redactionMode`'s doc comment
 * describes — the one shape a plain mask (`"[EMAIL]"`), a Hash-mode fingerprint
 * (`"#email:1a2b…"`), or a Remove-mode empty string never takes.
 *
 * Exported (Task 3, spec §8 step 3) for `worker/pipeline.ts`'s `filterExportableEntities`,
 * which uses this exact shape check to decide whether a PII finding overlapping an entity
 * carries a real, reversible pseudonym token — safe to key an exported entity on — or a
 * mask/hash/remove template, which is not. Reusing this predicate rather than duplicating
 * the regex is what keeps the two call sites from silently disagreeing on what counts as
 * "a real token" if this shape ever changes.
 */
export function looksLikePseudonymToken(template: string): boolean {
  return /^\[[A-Z_]+:[^:\]]+:[^\]]+\]$/.test(template);
}

/** Deterministic, non-cryptographic fingerprint — display-only obfuscation for Hash
 * mode, not a security boundary (unlike the real blake3 audit chain in `pii-engine.ts`). */
function fingerprint(input: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

/**
 * Keyed digest for Hash mode when it's applied at processing time (`worker/pipeline.ts`)
 * rather than in this file's instant in-memory viewer toggle. This output is baked into
 * exported markdown, so it has to survive an attacker who has the export.
 *
 * **HMAC-SHA256, not a bare SHA-256.** An unsalted digest is not a confidentiality
 * boundary for PII: the inputs here are low-entropy by nature (a name, a phone number, a
 * 9-digit SSN), so every plausible value can simply be hashed and compared. A bare
 * `SHA-256("ssn:123-45-6789")` is recovered by enumerating 10^9 candidates — seconds of
 * work. Keying it with a secret the attacker does not have is what makes the digest
 * irreversible in practice, not the choice of hash function.
 *
 * A *fixed* salt would not help (it ships in the bundle, so the attacker has it), and a
 * *random per-document* salt would destroy the one property Hash mode exists to provide —
 * the same value hashing to the same token, so an analyst can follow one person through a
 * corpus without learning who they are. The key must therefore be secret and stable across
 * the corpus, which is exactly what `deriveKeyHex`'s passphrase-derived key already is.
 * That is why Hash mode now requires a passphrase, the same way Pseudonymize does, and
 * falls back to mask without one rather than emitting a reversible "redaction".
 */
export async function hashSpanForProcessing(
  category: string,
  text: string,
  keyHex: string,
): Promise<string> {
  const keyBytes = hexToBytes(keyHex);
  const key = await crypto.subtle.importKey(
    "raw",
    // `.slice()` so the view handed to WebCrypto is a standalone ArrayBuffer, matching
    // how `lib/pseudonymize.ts` feeds `importKey` — see its `fresh()` helper for why a
    // subarray view is not accepted by every runtime's `BufferSource` handling.
    keyBytes.slice().buffer,
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const mac = await crypto.subtle.sign(
    "HMAC",
    key,
    new TextEncoder().encode(`${category}:${text}`),
  );
  const hex = Array.from(new Uint8Array(mac), (b) => b.toString(16).padStart(2, "0")).join("");
  // Truncated to 16 hex chars (64 bits) for readability in the redacted document. That is
  // a collision bound, not a secrecy one — secrecy rests entirely on the key above — and
  // 64 bits keeps collisions negligible for any realistic corpus.
  return `#${category}:${hex.slice(0, 16)}`;
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export type ModeResult = { findings: PiiEntity[] } | { unavailable: true; reason: string };

export function applyRedactionMode(
  findings: PiiEntity[],
  mode: RedactionMode,
): ModeResult {
  if (mode === "pseudonymize") {
    const hasRealTokens = findings.some((f) => looksLikePseudonymToken(f.redact_template));
    if (!hasRealTokens && findings.length > 0) {
      return {
        unavailable: true,
        reason:
          "This document wasn't processed with pseudonymize mode enabled, so no reversible tokens exist for it. Re-process the file with pseudonymize turned on in the pipeline config to use this mode.",
      };
    }
    return { findings };
  }

  return {
    findings: findings.map((f) => {
      switch (mode) {
        case "mask":
          return { ...f, redact_template: `[${f.category.toUpperCase()}]` };
        case "hash":
          return {
            ...f,
            redact_template: `#${f.category}:${fingerprint(f.text || `${f.category}:${f.start}:${f.end}`)}`,
          };
        case "remove":
          return { ...f, redact_template: "" };
        default:
          return f;
      }
    }),
  };
}
