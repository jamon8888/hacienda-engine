import type { AppConfig } from "@/lib/types";
import { Button } from "@/components/ui/button";

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

export function Settings({ config, onChange }: Props) {
  const storageUsed = 168; // mock
  const storageTotal = 10_000; // MB

  return (
    <div className="flex min-h-screen flex-col">
      <header className="border-b border-border bg-card px-6 py-3">
        <nav className="mx-auto flex max-w-6xl items-center justify-between">
          <div className="font-semibold">Hacienda Studio</div>
          <div className="flex items-center gap-4 text-sm">
            <a className="hover:underline">Studio</a>
            <a className="hover:underline">Ressources</a>
            <a className="rounded-md bg-muted px-3 py-1">Paramètres</a>
          </div>
        </nav>
      </header>
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
                  onClick={() => onChange({ ...config, redactionMode: m.value as any })}
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

          <div className="mt-3 flex items-center justify-between">
            <span className="text-sm">Reconnaissance optique</span>
            <button
              onClick={() => onChange({ ...config, enableTranscription: !config.enableTranscription })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.enableTranscription ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.enableTranscription ? "translate-x-5" : ""}`} />
            </button>
          </div>
        </section>

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
    </div>
  );
}
