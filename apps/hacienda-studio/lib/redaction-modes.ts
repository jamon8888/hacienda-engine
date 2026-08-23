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

/** Matches the `[CATEGORY:key_id:...]` shape `AppConfig.redactionMode`'s doc comment
 * describes — the one shape a plain mask (`"[EMAIL]"`) never takes. */
function looksLikePseudonymToken(template: string): boolean {
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
 * SHA-256 digest for Hash mode when it's applied at processing time (`worker/
 * pipeline.ts`) rather than in this file's instant in-memory viewer toggle. Unlike
 * `fingerprint` above, this output gets baked into exported markdown, so it needs to be a
 * real cryptographic digest, not a fast display-only one — `crypto.subtle` is available
 * in the worker with no new dependency, the same way `lib/pseudonymize.ts` already uses it.
 */
export async function hashSpanForProcessing(category: string, text: string): Promise<string> {
  const bytes = new TextEncoder().encode(`${category}:${text}`);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  const hex = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
  return `#${category}:${hex.slice(0, 16)}`;
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
