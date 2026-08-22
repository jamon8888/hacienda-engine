import { useEffect } from "react";
import type { AppConfig, NerCategory } from "../lib/types";

// Only categories the engine's vocabulary accepts and the worker's NER bridge actually
// produces. Offering anything else (full_name, company, address, …) makes the engine
// reject the entire NER result, which the app surfaces as an opaque "Unknown error"
// against the document.
const ALL_CATEGORIES: Array<{ key: NerCategory; label: string; group: string }> = [
  { key: "person", label: "Person", group: "Person" },
  { key: "organization", label: "Organization", group: "Organization" },
  { key: "location", label: "Location", group: "Location" },
  { key: "email", label: "Email", group: "Contact" },
  { key: "phone", label: "Phone", group: "Contact" },
  { key: "url", label: "URL", group: "Contact" },
  { key: "date", label: "Date", group: "Temporal" },
  { key: "money", label: "Money", group: "Financial" },
  { key: "percent", label: "Percent", group: "Financial" },
];

const GROUPED = ALL_CATEGORIES.reduce<Record<string, typeof ALL_CATEGORIES>>((acc, cat) => {
  (acc[cat.group] ??= []).push(cat);
  return acc;
}, {});

/**
 * Mirrors `hacienda-core`'s `VerticalConfig::comprehensive()` (`hacienda-core/src/pii/
 * config.rs`'s `COMPREHENSIVE_LABELS`) — kept in sync by hand, the same way this file's
 * `ALL_CATEGORIES` already duplicates the base category list rather than sharing a
 * source with the Rust side. Sent as `nerCustomLabels`, not `nerCategories` — unlike
 * `ALL_CATEGORIES`, these aren't in the closed `NerCategory` union and only reach the
 * model via `WasmNerConfig.customLabels` (`worker/pipeline.ts`), additive to whatever
 * categories are checked above. Opt-in and off by default: there is no accuracy or
 * latency data yet for requesting this many zero-shot labels at once.
 */
const COMPREHENSIVE_PII_LABELS = [
  "address",
  "ssn",
  "passport",
  "drivers_license",
  "eu_vat",
  "national_id",
  "tax_id",
  "credit_card",
  "iban",
  "bank_account",
  "routing_number",
  "swift_bic",
  "crypto_wallet",
  "medical_record_number",
  "health_plan_number",
  "diagnosis",
  "medication",
  "username",
  "password",
  "api_key",
  "secret_token",
  "jwt_token",
  "ip_address",
  "mac_address",
  "url",
  "license_plate",
  "vehicle_vin",
  "date_of_birth",
  "full_name",
];

const TRANSCRIPTION_MODELS: AppConfig["transcriptionModel"][] = [
  "tiny.en",
  "tiny",
  "base.en",
  "base",
  "small.en",
  "small",
];

const TRANSCRIPTION_MODEL_LABELS: Record<AppConfig["transcriptionModel"], string> = {
  "tiny.en": "Tiny English (75 MB, fastest)",
  tiny: "Tiny Multilingual (75 MB)",
  "base.en": "Base English (142 MB, balanced)",
  base: "Base Multilingual (142 MB)",
  "small.en": "Small English (466 MB, better quality)",
  small: "Small Multilingual (466 MB)",
};

const LANGUAGES = [
  { code: "auto", label: "Auto-detect" },
  { code: "de", label: "German" },
  { code: "fr", label: "French" },
  { code: "es", label: "Spanish" },
  { code: "it", label: "Italian" },
  { code: "pt", label: "Portuguese" },
  { code: "nl", label: "Dutch" },
  { code: "pl", label: "Polish" },
  { code: "sv", label: "Swedish" },
  { code: "da", label: "Danish" },
  { code: "fi", label: "Finnish" },
  { code: "cs", label: "Czech" },
  { code: "hu", label: "Hungarian" },
  { code: "el", label: "Greek" },
  { code: "ro", label: "Romanian" },
  { code: "bg", label: "Bulgarian" },
  { code: "hr", label: "Croatian" },
  { code: "sk", label: "Slovak" },
  { code: "sl", label: "Slovenian" },
  { code: "et", label: "Estonian" },
  { code: "lv", label: "Latvian" },
  { code: "lt", label: "Lithuanian" },
  { code: "mt", label: "Maltese" },
  { code: "ga", label: "Irish" },
];

const VERTICALS: AppConfig["enabledVerticals"][number][] = ["m&a", "financial_services", "shared"];

const fieldLabel = "flex flex-col gap-1.5 text-sm text-muted-foreground";
const control =
  "rounded-md border border-input bg-background px-3 py-2 font-[inherit] text-foreground";

export function ConfigPanel({
  config,
  onChange,
  onDone,
}: {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  onDone: () => void;
}) {
  // Plain `role="dialog"` div (see components/ui/README.md — this app hasn't adopted the
  // @base-ui/react Dialog hacienda-private now uses), so it gets none of a real Dialog
  // primitive's behavior for free: nothing was listening for Escape at all, so the panel
  // stayed open and covering the right side of the screen for the rest of a session (this
  // is what tests/e2e's `page.keyboard.press("Escape")` calls were quietly relying on).
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === "Escape") onDone();
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onDone]);

  const toggleCategory = (key: NerCategory) => {
    const has = config.nerCategories.includes(key);
    onChange({
      ...config,
      nerCategories: has
        ? config.nerCategories.filter((c) => c !== key)
        : [...config.nerCategories, key],
    });
  };

  const comprehensivePiiEnabled = COMPREHENSIVE_PII_LABELS.every((label) =>
    config.nerCustomLabels.includes(label),
  );
  const toggleComprehensivePii = () => {
    onChange({
      ...config,
      nerCustomLabels: comprehensivePiiEnabled
        ? config.nerCustomLabels.filter((label) => !COMPREHENSIVE_PII_LABELS.includes(label))
        : [...new Set([...config.nerCustomLabels, ...COMPREHENSIVE_PII_LABELS])],
    });
  };

  const toggleVertical = (v: AppConfig["enabledVerticals"][number]) => {
    const has = config.enabledVerticals.includes(v);
    onChange({
      ...config,
      enabledVerticals: has
        ? config.enabledVerticals.filter((x) => x !== v)
        : [...config.enabledVerticals, v],
    });
  };

  return (
    <div
      className="fixed inset-y-0 right-0 z-[100] w-[360px] max-w-full overflow-y-auto border-l border-border bg-card p-6 text-card-foreground shadow-2xl"
      role="dialog"
      aria-label="Configuration"
    >
      <header className="mb-6 border-b border-border pb-3">
        <h2 className="mb-1 text-lg font-semibold">⚙ Configuration</h2>
        <p className="text-sm font-medium text-emerald-600">
          🔒 All processing runs locally in your browser
        </p>
      </header>

      <section className="mb-6">
        <h3 className="mb-2 text-sm uppercase tracking-wide text-muted-foreground">
          NER Categories
        </h3>
        <div className="flex flex-col gap-2">
          {Object.entries(GROUPED).map(([group, categories]) => (
            <fieldset key={group} className="rounded-md border border-border p-2">
              <legend className="px-1 text-xs uppercase text-muted-foreground">{group}</legend>
              {categories.map((cat) => (
                <label key={cat.key} className="flex cursor-pointer items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={config.nerCategories.includes(cat.key)}
                    onChange={() => toggleCategory(cat.key)}
                  />
                  <span>{cat.label}</span>
                </label>
              ))}
            </fieldset>
          ))}
        </div>
        <label className="mt-2 flex cursor-pointer items-start gap-2 rounded-md border border-amber-500/20 bg-amber-500/[0.06] p-2 text-sm text-foreground">
          <input
            type="checkbox"
            className="mt-0.5"
            checked={comprehensivePiiEnabled}
            onChange={toggleComprehensivePii}
          />
          <span>
            Comprehensive PII (SSN, IBAN, passport, API keys, and {COMPREHENSIVE_PII_LABELS.length - 3} more)
            <span className="block text-xs text-muted-foreground">
              Requests many more zero-shot labels at once — no accuracy or latency data yet for this document set. Off by default.
            </span>
          </span>
        </label>
      </section>

      <section className="mb-6">
        <h3 className="mb-2 text-sm uppercase tracking-wide text-muted-foreground">
          🎙️ Transcription
        </h3>
        <div className={fieldLabel}>
          <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={config.enableTranscription}
              onChange={(e) => onChange({ ...config, enableTranscription: e.target.checked })}
            />
            Enable Audio/Video Transcription
          </label>
        </div>
        {config.enableTranscription && (
          <>
            <div className={fieldLabel + " mt-2"}>
              <label htmlFor="transcription-model">Model</label>
              <select
                id="transcription-model"
                className={control}
                value={config.transcriptionModel}
                onChange={(e) =>
                  onChange({
                    ...config,
                    transcriptionModel: e.target.value as AppConfig["transcriptionModel"],
                  })
                }
              >
                {TRANSCRIPTION_MODELS.map((m) => (
                  <option key={m} value={m}>
                    {TRANSCRIPTION_MODEL_LABELS[m]}
                  </option>
                ))}
              </select>
            </div>
            <div className={fieldLabel + " mt-2"}>
              <label htmlFor="transcription-language">Language</label>
              <select
                id="transcription-language"
                className={control}
                value={config.transcriptionLanguage}
                onChange={(e) => onChange({ ...config, transcriptionLanguage: e.target.value })}
              >
                {LANGUAGES.map((lang) => (
                  <option key={lang.code} value={lang.code}>
                    {lang.label}
                  </option>
                ))}
              </select>
            </div>
            <div className={fieldLabel + " mt-2"}>
              <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
                <input
                  type="checkbox"
                  checked={config.translateToEnglish}
                  onChange={(e) => onChange({ ...config, translateToEnglish: e.target.checked })}
                />
                Translate to English
              </label>
            </div>
          </>
        )}
      </section>

      <section className="mb-6">
        <h3 className="mb-2 text-sm uppercase tracking-wide text-muted-foreground">
          🔒 PII &amp; Compliance
        </h3>
        <div className={fieldLabel}>
          <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={config.enablePiiDetection}
              onChange={(e) => onChange({ ...config, enablePiiDetection: e.target.checked })}
            />
            Enable PII Detection
          </label>
        </div>
        <div className={fieldLabel + " mt-2"}>
          <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={config.redactPiiInOutput}
              onChange={(e) => onChange({ ...config, redactPiiInOutput: e.target.checked })}
            />
            Redact PII in Output
          </label>
        </div>
        {config.redactPiiInOutput && (
          <>
            <div className={fieldLabel + " mt-2"}>
              <label htmlFor="redaction-mode">Redaction mode</label>
              <select
                id="redaction-mode"
                className={control}
                value={config.redactionMode}
                onChange={(e) =>
                  onChange({
                    ...config,
                    redactionMode: e.target.value as AppConfig["redactionMode"],
                  })
                }
              >
                <option value="mask">Mask (e.g. [EMAIL]) — not reversible</option>
                <option value="pseudonymize">
                  Pseudonymize — reversible with your passphrase
                </option>
              </select>
            </div>
            {config.redactionMode === "pseudonymize" && (
              <div className={fieldLabel + " mt-2"}>
                <label htmlFor="pseudonym-passphrase">
                  Passphrase (never leaves your browser)
                </label>
                <input
                  id="pseudonym-passphrase"
                  type="password"
                  className={control}
                  value={config.pseudonymPassphrase}
                  onChange={(e) =>
                    onChange({ ...config, pseudonymPassphrase: e.target.value })
                  }
                  placeholder="Used to derive the redaction key — remember it"
                />
                {!config.pseudonymPassphrase && (
                  <p className="text-xs text-amber-600">
                    Without a passphrase, findings are masked instead — no key to derive.
                  </p>
                )}
              </div>
            )}
          </>
        )}
      </section>

      <section className="mb-6">
        <h3 className="mb-2 text-sm uppercase tracking-wide text-muted-foreground">
          🏷️ Vertical NER
        </h3>
        <fieldset className="rounded-md border border-border p-2">
          <legend className="px-1 text-xs uppercase text-muted-foreground">
            Enabled Verticals
          </legend>
          <div className="flex flex-col gap-1">
            {VERTICALS.map((v) => (
              <label key={v} className="flex cursor-pointer items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  value={v}
                  checked={config.enabledVerticals.includes(v)}
                  onChange={() => toggleVertical(v)}
                />
                <span>{v.toUpperCase()}</span>
              </label>
            ))}
          </div>
        </fieldset>
      </section>

      <section className="mb-6">
        <h3 className="mb-2 text-sm uppercase tracking-wide text-muted-foreground">Output</h3>
        <div className={fieldLabel}>
          <label htmlFor="output-format">Format</label>
          <select
            id="output-format"
            className={control}
            value={config.outputFormat}
            onChange={(e) =>
              onChange({ ...config, outputFormat: e.target.value as AppConfig["outputFormat"] })
            }
          >
            <option value="markdown">Markdown</option>
            <option value="plain">Plain Text</option>
            <option value="json">JSON</option>
          </select>
        </div>
        <div className={fieldLabel + " mt-2"}>
          <label htmlFor="chunk-size">Chunk Size (tokens)</label>
          <input
            id="chunk-size"
            type="number"
            className={control}
            min={100}
            max={8000}
            step={100}
            value={config.chunkSize}
            onChange={(e) => onChange({ ...config, chunkSize: Number(e.target.value) })}
          />
        </div>
      </section>

      <footer className="mt-8 border-t border-border pt-3 text-right">
        <button
          className="rounded-md bg-secondary px-6 py-2 font-medium text-secondary-foreground transition-colors hover:bg-primary hover:text-primary-foreground"
          onClick={onDone}
        >
          Done
        </button>
      </footer>
    </div>
  );
}
