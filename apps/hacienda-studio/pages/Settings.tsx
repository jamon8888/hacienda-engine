import { useState } from "react";
import type { AppConfig, NerCategory } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  listKnownKeys,
  recordKeyUsage,
  removeKnownKey,
  renameKnownKey,
  type KnownPseudonymKey,
} from "@/lib/pseudonym-keys";

type Props = {
  config: AppConfig;
  onChange: (c: AppConfig) => void;
};

const REDACTION_MODES = [
  { value: "mask", label: "Masquer" },
  { value: "hash", label: "Hacher" },
  { value: "pseudonymize", label: "Pseudonymiser" },
  { value: "remove", label: "Supprimer" },
] as const;

// Only categories the engine's vocabulary accepts and the worker's NER bridge actually
// produces. Offering anything else (full_name, company, address, …) makes the engine
// reject the entire NER result, which the app surfaces as an opaque "Unknown error"
// against the document.
const ALL_CATEGORIES: Array<{ key: NerCategory; label: string; group: string }> = [
  { key: "person", label: "Personne", group: "Personne" },
  { key: "organization", label: "Organisation", group: "Organisation" },
  { key: "location", label: "Lieu", group: "Lieu" },
  { key: "email", label: "E-mail", group: "Contact" },
  { key: "phone", label: "Téléphone", group: "Contact" },
  { key: "url", label: "URL", group: "Contact" },
  { key: "date", label: "Date", group: "Temporel" },
  { key: "time", label: "Heure", group: "Temporel" },
  { key: "money", label: "Montant", group: "Financier" },
  { key: "percent", label: "Pourcentage", group: "Financier" },
];

const GROUPED = ALL_CATEGORIES.reduce<Record<string, typeof ALL_CATEGORIES>>((acc, cat) => {
  (acc[cat.group] ??= []).push(cat);
  return acc;
}, {});

/**
 * Mirrors `hacienda-core`'s `VerticalConfig::comprehensive()` (`hacienda-core/src/pii/
 * config.rs`'s `COMPREHENSIVE_LABELS`) — kept in sync by hand. Sent as `nerCustomLabels`,
 * not `nerCategories` — unlike `ALL_CATEGORIES`, these aren't in the closed `NerCategory`
 * union and only reach the model via `WasmNerConfig.customLabels` (`worker/pipeline.ts`),
 * additive to whatever categories are checked above. Opt-in and off by default: there is
 * no accuracy or latency data yet for requesting this many zero-shot labels at once.
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
  "tiny.en": "Tiny Anglais (75 Mo, le plus rapide)",
  tiny: "Tiny Multilingue (75 Mo)",
  "base.en": "Base Anglais (142 Mo, équilibré)",
  base: "Base Multilingue (142 Mo)",
  "small.en": "Small Anglais (466 Mo, meilleure qualité)",
  small: "Small Multilingue (466 Mo)",
};

const LANGUAGES = [
  { code: "auto", label: "Détection automatique" },
  { code: "de", label: "Allemand" },
  { code: "fr", label: "Français" },
  { code: "es", label: "Espagnol" },
  { code: "it", label: "Italien" },
  { code: "pt", label: "Portugais" },
  { code: "nl", label: "Néerlandais" },
  { code: "pl", label: "Polonais" },
  { code: "sv", label: "Suédois" },
  { code: "da", label: "Danois" },
  { code: "fi", label: "Finnois" },
  { code: "cs", label: "Tchèque" },
  { code: "hu", label: "Hongrois" },
  { code: "el", label: "Grec" },
  { code: "ro", label: "Roumain" },
  { code: "bg", label: "Bulgare" },
  { code: "hr", label: "Croate" },
  { code: "sk", label: "Slovaque" },
  { code: "sl", label: "Slovène" },
  { code: "et", label: "Estonien" },
  { code: "lv", label: "Letton" },
  { code: "lt", label: "Lituanien" },
  { code: "mt", label: "Maltais" },
  { code: "ga", label: "Irlandais" },
];

const fieldLabel = "flex flex-col gap-1.5 text-sm text-muted-foreground";
const control =
  "w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground";

export function Settings({ config, onChange }: Props) {
  const storageUsed = 168; // mock
  const storageTotal = 10_000; // MB
  // Read once on mount — reopening Settings is enough to pick up a key used elsewhere
  // (e.g. a reveal in `PiiPanel.tsx`) since last render.
  const [knownKeys] = useState(() => listKnownKeys());

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

  return (
    <main className="mx-auto w-full max-w-3xl px-6 py-12">
        <h1 className="text-2xl font-semibold">Paramètres</h1>
        <p className="mt-1 text-sm text-muted-foreground">Valeurs par défaut pour les nouveaux lots, plus tout ce qui est mis en cache sur cet appareil.</p>

        <section className="mt-8 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-4 text-sm font-medium">Valeurs par défaut du pipeline</h2>

          <div className="mb-4">
            <div className="mb-2 text-sm font-medium">Mode de rédaction</div>
            <div className="flex gap-2">
              {REDACTION_MODES.map((m) => (
                <button
                  key={m.value}
                  onClick={() => onChange({ ...config, redactionMode: m.value })}
                  className={`rounded border px-3 py-1 text-sm ${
                    config.redactionMode === m.value
                      ? "border-primary bg-primary/10"
                      : "border-border bg-background"
                  }`}
                >
                  {m.label}
                </button>
              ))}
            </div>
            {config.redactionMode === "pseudonymize" && (
              <div className={fieldLabel + " mt-3 max-w-sm"}>
                <label htmlFor="pseudonym-key-id">Identifiant de clé</label>
                <input
                  id="pseudonym-key-id"
                  className={control}
                  list="known-pseudonym-keys"
                  value={config.pseudonymKeyId}
                  onChange={(e) => onChange({ ...config, pseudonymKeyId: e.target.value })}
                  placeholder="session"
                />
                <datalist id="known-pseudonym-keys">
                  {knownKeys.map((k) => (
                    <option key={k.keyId} value={k.keyId}>
                      {k.label}
                    </option>
                  ))}
                </datalist>
                <label htmlFor="pseudonym-passphrase" className="mt-2">
                  Phrase secrète (ne quitte jamais votre navigateur)
                </label>
                <input
                  id="pseudonym-passphrase"
                  type="password"
                  className={control}
                  value={config.pseudonymPassphrase}
                  onChange={(e) => onChange({ ...config, pseudonymPassphrase: e.target.value })}
                  // Records the key id as "known" once the user has actually committed to
                  // using it (a non-empty passphrase), not on every keystroke — this is the
                  // point processing will derive against `config.pseudonymKeyId`.
                  onBlur={() => {
                    if (config.pseudonymPassphrase) recordKeyUsage(config.pseudonymKeyId);
                  }}
                  placeholder="Utilisée pour dériver la clé de rédaction — mémorisez-la"
                />
                {!config.pseudonymPassphrase && (
                  <p className="text-xs text-amber-600">
                    Sans phrase secrète, les résultats sont masqués au lieu d'être
                    pseudonymisés — aucune clé à dériver.
                  </p>
                )}
              </div>
            )}
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="mb-1 block text-sm">Vertical</label>
              <select
                value={config.enabledVerticals?.[0] || "general"}
                onChange={(e) => onChange({ ...config, enabledVerticals: [e.target.value] })}
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              >
                <option value="general">général</option>
                <option value="m&a">m&a</option>
                <option value="financial_services">services financiers</option>
              </select>
            </div>
            <div>
              <label className="mb-1 block text-sm">Sensibilité</label>
              <select
                value={config.sensitivity || "balanced"}
                onChange={(e) => onChange({ ...config, sensitivity: e.target.value as any })}
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              >
                <option value="low">Faible</option>
                <option value="balanced">Équilibrée</option>
                <option value="high">Élevée</option>
              </select>
            </div>
          </div>

          <div className="mt-4 flex items-center justify-between">
            <span className="text-sm">Reconnaissance d'entités</span>
            <button
              onClick={() => onChange({ ...config, enablePiiDetection: !config.enablePiiDetection })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.enablePiiDetection ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.enablePiiDetection ? "translate-x-5" : ""}`} />
            </button>
          </div>

          {/* `redactPiiInOutput` gates every redaction mode above (worker/pipeline.ts) —
           * without this toggle on, "Mode de rédaction" only counts PII, it never touches
           * the markdown, which is why mask/hash/pseudonymize/remove all looked like they
           * "did nothing." This control existed in the now-deleted ConfigPanel.tsx but was
           * dropped when its fields were ported into this file (be501cc) — restoring it. */}
          <div className="mt-3 flex items-center justify-between">
            <span className="text-sm">Rédiger les PII dans la sortie</span>
            <button
              onClick={() => onChange({ ...config, redactPiiInOutput: !config.redactPiiInOutput })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.redactPiiInOutput ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.redactPiiInOutput ? "translate-x-5" : ""}`} />
            </button>
          </div>
          {!config.redactPiiInOutput && (
            <p className="mt-1 text-xs text-amber-600">
              Désactivé : les PII détectées sont comptées mais laissées telles quelles dans
              le document exporté.
            </p>
          )}

          <div className="mt-3 flex items-center justify-between">
            <span className="text-sm">Reconnaissance optique</span>
            <button
              onClick={() => onChange({ ...config, enableTranscription: !config.enableTranscription })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.enableTranscription ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.enableTranscription ? "translate-x-5" : ""}`} />
            </button>
          </div>
          {config.enableTranscription && (
            <div className="mt-3 grid grid-cols-2 gap-4">
              <div className={fieldLabel}>
                <label htmlFor="transcription-model">Modèle</label>
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
              <div className={fieldLabel}>
                <label htmlFor="transcription-language">Langue</label>
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
              <label className="col-span-2 flex cursor-pointer items-center gap-2 text-sm text-foreground">
                <input
                  type="checkbox"
                  checked={config.translateToEnglish}
                  onChange={(e) => onChange({ ...config, translateToEnglish: e.target.checked })}
                />
                Traduire en anglais
              </label>
            </div>
          )}
        </section>

        <section className="mt-6 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-4 text-sm font-medium">Catégories NER</h2>
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
              PII exhaustif (SSN, IBAN, passeport, clés API, et {COMPREHENSIVE_PII_LABELS.length - 3} de plus)
              <span className="block text-xs text-muted-foreground">
                Demande beaucoup plus de labels zero-shot à la fois — aucune donnée de
                précision ou de latence pour ce type de document. Désactivé par défaut.
              </span>
            </span>
          </label>
        </section>

        <section className="mt-6 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-4 text-sm font-medium">Sortie</h2>
          <div className="grid grid-cols-2 gap-4">
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
                <option value="plain">Texte brut</option>
                <option value="json">JSON</option>
              </select>
            </div>
            <div className={fieldLabel}>
              <label htmlFor="chunk-size">Taille des segments (caractères)</label>
              <input
                id="chunk-size"
                type="number"
                className={control}
                min={100}
                max={8000}
                step={100}
                value={config.chunkSize}
                onChange={(e) => {
                  const n = Number(e.target.value);
                  if (!e.target.value || Number.isNaN(n)) return;
                  onChange({ ...config, chunkSize: Math.min(8000, Math.max(100, n)) });
                }}
              />
            </div>
          </div>
        </section>

        <PseudonymKeyVault />

        <section className="mt-6 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-2 text-sm font-medium">Stockage local</h2>
          <p className="mb-3 text-xs text-muted-foreground">168 Ko utilisés sur 10,00 Go disponibles</p>
          <div className="flex gap-2">
            <Button variant="outline" size="sm">Vérifier la chaîne d'audit</Button>
            <Button variant="outline" size="sm">Effacer les brouillons</Button>
            <Button variant="outline" size="sm">Effacer les ressources mises en cache</Button>
            <Button variant="destructive" size="sm">Supprimer tout</Button>
          </div>
        </section>
      </main>
  );
}

/**
 * Lists the pseudonymization key ids `Settings.tsx`'s own passphrase field and
 * `PiiPanel.tsx` have recorded a successful mint or reveal against
 * (`lib/pseudonym-keys.ts`) — never the passphrase itself, which is never stored
 * anywhere. Renaming only changes a local label; removing only forgets that label.
 * Neither can revoke or rotate the underlying key material, since no key material is
 * stored here to begin with — see that module's doc comment.
 */
function PseudonymKeyVault() {
  const [keys, setKeys] = useState<KnownPseudonymKey[]>(() => listKnownKeys());
  const [editingLabel, setEditingLabel] = useState<Record<string, string>>({});

  function commitRename(keyId: string) {
    const label = editingLabel[keyId];
    if (label !== undefined) renameKnownKey(keyId, label);
    setKeys(listKnownKeys());
    setEditingLabel((prev) => {
      const next = { ...prev };
      delete next[keyId];
      return next;
    });
  }

  return (
    <section className="mt-6 rounded-lg border border-border bg-card p-6">
      <h2 className="mb-1 text-sm font-medium">Clés de pseudonymisation connues</h2>
      <p className="mb-3 text-xs text-muted-foreground">
        Identifiants de clé utilisés sur cet appareil — jamais la phrase secrète elle-même,
        qui n'est jamais enregistrée.
      </p>
      {keys.length === 0 ? (
        <p className="text-sm text-muted-foreground">Aucune clé utilisée pour l'instant.</p>
      ) : (
        <ul className="space-y-2">
          {keys.map((k) => (
            <li key={k.keyId} className="flex items-center gap-2 text-sm">
              <span className="w-32 shrink-0 truncate font-mono text-xs text-muted-foreground">
                {k.keyId}
              </span>
              <Input
                className="h-8"
                value={editingLabel[k.keyId] ?? k.label}
                onChange={(e) =>
                  setEditingLabel((prev) => ({ ...prev, [k.keyId]: e.target.value }))
                }
                onBlur={() => commitRename(k.keyId)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                }}
              />
              <Button
                variant="destructive"
                size="sm"
                onClick={() => {
                  removeKnownKey(k.keyId);
                  setKeys(listKnownKeys());
                }}
              >
                Oublier
              </Button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
