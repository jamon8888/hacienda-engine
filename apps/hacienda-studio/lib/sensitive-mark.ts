export type SensitiveMode = "pseudonymize" | "manual-mask" | "delete";
export type SensitiveMarkSource = "xberg_metadata" | "gliner" | "manual" | "regex";

export interface SensitiveMarkAttrs {
  tokenClass: string;
  tokenId: string;
  mode: SensitiveMode;
  source: SensitiveMarkSource;
  disclosure: "viewer-only";
}
