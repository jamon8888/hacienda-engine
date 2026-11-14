import type { SensitiveMarkAttrs, SensitiveMarkSource } from "../../hacienda-studio/lib/sensitive-mark";

const providerSource: SensitiveMarkSource = "regex";
const manualSource: SensitiveMarkSource = "manual";
const providerMark: SensitiveMarkAttrs = {
  tokenClass: "PERSON",
  tokenId: "[PERSON:session:opaque]",
  mode: "pseudonymize",
  source: providerSource,
  disclosure: "viewer-only",
};
const manualMark: SensitiveMarkAttrs = { ...providerMark, source: manualSource };

// @ts-expect-error A renderer provenance value must stay inside the canonical mark union.
const invalidMark: SensitiveMarkAttrs = { ...providerMark, source: "provider" };

void invalidMark;
void manualMark;
