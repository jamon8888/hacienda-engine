import type { AppConfig } from "@/lib/types";
import { Button } from "@/components/ui/button";

type Props = {
  config: AppConfig;
  onChange: (c: AppConfig) => void;
};

const REDACTION_MODES = ["Mask", "Hash", "Pseudonymize", "Remove"] as const;

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
            <a className="hover:underline">Assets</a>
            <a className="rounded-md bg-muted px-3 py-1">Settings</a>
          </div>
        </nav>
      </header>
      <main className="mx-auto w-full max-w-3xl px-6 py-12">
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">Defaults for new batches, plus everything cached on this device.</p>

        <section className="mt-8 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-4 text-sm font-medium">Pipeline defaults</h2>

          <div className="mb-4">
            <div className="mb-2 text-sm font-medium">Redaction mode</div>
            <div className="flex gap-2">
              {REDACTION_MODES.map((m) => (
                <button
                  key={m}
                  onClick={() => onChange({ ...config, redactionMode: m.toLowerCase() as any })}
                  className={`rounded border px-3 py-1 text-sm ${
                    config.redactionMode === m.toLowerCase()
                      ? "border-primary bg-primary/10"
                      : "border-border bg-background"
                  }`}
                >
                  {m}
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
                <option value="general">general</option>
                <option value="m&a">m&a</option>
                <option value="financial_services">financial_services</option>
              </select>
            </div>
            <div>
              <label className="mb-1 block text-sm">Sensitivity</label>
              <select
                value={config.sensitivity || "balanced"}
                onChange={(e) => onChange({ ...config, sensitivity: e.target.value as any })}
                className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
              >
                <option value="low">Low</option>
                <option value="balanced">Balanced</option>
                <option value="high">High</option>
              </select>
            </div>
          </div>

          <div className="mt-4 flex items-center justify-between">
            <span className="text-sm">Entity recognition</span>
            <button
              onClick={() => onChange({ ...config, enablePiiDetection: !config.enablePiiDetection })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.enablePiiDetection ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.enablePiiDetection ? "translate-x-5" : ""}`} />
            </button>
          </div>

          <div className="mt-3 flex items-center justify-between">
            <span className="text-sm">Optical recognition</span>
            <button
              onClick={() => onChange({ ...config, enableTranscription: !config.enableTranscription })}
              className={`h-6 w-11 rounded-full p-0.5 transition-colors ${config.enableTranscription ? "bg-primary" : "bg-muted"}`}
            >
              <span className={`block h-5 w-5 rounded-full bg-white transition-transform ${config.enableTranscription ? "translate-x-5" : ""}`} />
            </button>
          </div>
        </section>

        <section className="mt-6 rounded-lg border border-border bg-card p-6">
          <h2 className="mb-2 text-sm font-medium">Local storage</h2>
          <p className="mb-3 text-xs text-muted-foreground">168 KB used of 10.00 GB available</p>
          <div className="flex gap-2">
            <Button variant="outline" size="sm">Verify audit chain</Button>
            <Button variant="outline" size="sm">Clear drafts</Button>
            <Button variant="outline" size="sm">Clear cached assets</Button>
            <Button variant="destructive" size="sm">Delete everything</Button>
          </div>
        </section>
      </main>
    </div>
  );
}
