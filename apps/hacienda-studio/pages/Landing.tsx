import { Lock, ScanEye, Layers, ShieldCheck, EyeOff, Server } from "lucide-react";

const FEATURES = [
  {
    icon: Lock,
    title: "Extraire n'importe quoi",
    body: "PDF, DOCX, XLSX, PPTX, CSV et texte brut sont convertis en markdown propre avec une carte de mise en page.",
  },
  {
    icon: EyeOff,
    title: "Trouver les données personnelles",
    body: "Les PII sont affichées avec des scores de confiance, ajustés par sensibilité et presets verticaux.",
  },
  {
    icon: Layers,
    title: "Masquer à votre façon",
    body: "Masquez, hachez, pseudonymisez ou supprimez chaque portion — aperçu en direct contre l'original.",
  },
  {
    icon: ScanEye,
    title: "Revoir et exporter",
    body: "Modifiez le markdown masqué à la main, puis exportez une archive avec originaux, masquages et résultats.",
  },
  {
    icon: ShieldCheck,
    title: "Traçabilité inviolable",
    body: "Chaque masquage s'ajoute à un journal d'audit local chaîné par hachage que vous pouvez vérifier à tout moment.",
  },
  {
    icon: Server,
    title: "Pas de serveur, pas de compte",
    body: "Les documents sont traités dans un Web Worker et mis en cache dans IndexedDB sur votre appareil.",
  },
];

/**
 * Redesign: the hero (badge, headline, pipeline steps) now lives at the top of
 * `Studio.tsx` — the upload block is the homepage's first section, with the headline as
 * that section's H1. This component is only the marketing content that follows it.
 */
export function Landing() {
  return (
    <div className="flex-1">
      <section className="border-b border-border px-6 py-16">
        <div className="mx-auto max-w-5xl">
          <h2 className="mb-8 text-2xl font-semibold">Ce que fait le studio</h2>
          <div className="grid grid-cols-1 gap-px overflow-hidden rounded-lg border border-border bg-border sm:grid-cols-3">
            {FEATURES.map(({ icon: Icon, title, body }) => (
              <div key={title} className="bg-card p-6">
                <Icon className="mb-4 size-5 text-primary" />
                <h3 className="mb-1.5 font-semibold">{title}</h3>
                <p className="text-sm text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <footer className="border-t border-border px-6 py-8 text-center text-sm text-muted-foreground">
        Hacienda Studio — les documents sont traités sur l'appareil. Pas de compte, pas de stockage serveur.
      </footer>
    </div>
  );
}
