import * as React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { AlertTriangle, Info } from "lucide-react";

export function Conservation() {
  const [fileDays, setFileDays] = React.useState("24");
  const [pseudoDays, setPseudoDays] = React.useState("48");

  return (
    <div className="space-y-6 max-w-3xl">
      <div>
        <h1 className="text-2xl font-semibold">Durée de conservation</h1>
        <p className="text-sm text-muted-foreground">Définissez les délais avant lesquels vos documents et données sont définitivement supprimés.</p>
      </div>
      <Card>
        <CardContent className="pt-6 space-y-3">
          <h3 className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Fichiers</h3>
          <p className="text-xs text-muted-foreground">Délai après lequel vos documents (en clair et anonymisés) et les données associées sont effacés. Mesuré depuis la dernière modification du document.</p>
          <div className="flex items-center gap-2">
            <input value={fileDays} onChange={e=>setFileDays(e.target.value)} className="w-20 rounded-md border bg-background px-2 py-1 text-xs" />
            <span className="text-xs">h</span>
            <Button size="sm" variant="outline">24 h</Button>
            <Button size="sm" variant="outline">1 semaine</Button>
          </div>
        </CardContent>
      </Card>
      <Card>
        <CardContent className="pt-6 space-y-3">
          <h3 className="text-xs font-medium uppercase tracking-widest text-muted-foreground">Pseudonymes et sessions</h3>
          <p className="text-xs text-muted-foreground">Délai après lequel les sessions et leurs pseudonymes sont effacés, impactant la fonctionnalité de restauration (dé-pseudonymisation). Mesuré depuis le dernier ajout d'un document à la session, ou la dernière modification de ses paramètres.</p>
          <div className="flex items-center gap-2">
            <input value={pseudoDays} onChange={e=>setPseudoDays(e.target.value)} className="w-20 rounded-md border bg-background px-2 py-1 text-xs" />
            <Button size="sm" variant="outline">48 h</Button>
            <Button size="sm" variant="outline">2 semaines</Button>
            <Button size="sm" variant="outline">3 mois</Button>
            <Button size="sm" variant="outline">12 mois</Button>
          </div>
        </CardContent>
      </Card>
      <div className="grid grid-cols-2 gap-4">
        <Card className="border-amber-300 bg-amber-50/40">
          <CardContent className="pt-4 flex gap-2">
            <AlertTriangle className="size-4 text-amber-500 shrink-0 mt-0.5" />
            <div className="text-xs">
              <p className="font-medium">Maître du shadow IT</p>
              <p className="text-muted-foreground">Le délai avant expiration est affiché sur chaque session et chaque fichier. Contrôlez ainsi précisément quelles données sont conservées et pour combien de temps.</p>
            </div>
          </CardContent>
        </Card>
        <Card className="border-amber-300 bg-amber-50/40">
          <CardContent className="pt-4 flex gap-2">
            <Info className="size-4 text-amber-500 shrink-0 mt-0.5" />
            <div className="text-xs">
              <p className="font-medium">Paramétré = Aucune conservation = (EDR)</p>
              <p className="text-muted-foreground">La durée minimale de conservation est d'environ une heure, nécessaire à la comptabilité avec le système de pseudonymes partages, l'accès par API et l'intégration Cowork.</p>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
