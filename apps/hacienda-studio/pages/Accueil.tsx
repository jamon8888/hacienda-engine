import * as React from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { ArrowRight } from "lucide-react";

export function Accueil() {
  return (
    <div className="flex flex-col items-center justify-center min-h-[60vh] space-y-8 py-12">
      <div className="text-center space-y-2">
        <h1 className="text-3xl font-semibold">Marvin Systems</h1>
        <p className="text-sm text-muted-foreground">Découvrez le futur de la confidentialité automatisée !</p>
      </div>
      <div className="grid gap-6 md:grid-cols-2 max-w-3xl w-full">
        <Card>
          <CardContent className="flex flex-col items-center text-center space-y-4 pt-8 pb-8">
            <div className="size-12 rounded bg-primary/10 flex items-center justify-center text-primary text-2xl">📄</div>
            <h3 className="text-sm font-medium">Pseudonymisez vos documents</h3>
            <p className="text-xs text-muted-foreground">Importez vos fichiers en un clic, obtenez des versions prêtes à être partagées.</p>
            <Button size="sm">Créer une session <ArrowRight className="ml-2 size-3" /></Button>
          </CardContent>
        </Card>
        <Card>
          <CardContent className="flex flex-col items-center text-center space-y-4 pt-8 pb-8">
            <div className="size-12 rounded bg-primary/10 flex items-center justify-center text-primary text-2xl">🔗</div>
            <h3 className="text-sm font-medium">Configurez l'intégration <span className="text-primary">Claude Cowork</span></h3>
            <p className="text-xs text-muted-foreground">Laissez Marvin pseudonymiser vos documents avant d'atteindre les serveurs de l'IA.</p>
            <Button size="sm" variant="outline">Découvrir l'intégration <ArrowRight className="ml-2 size-3" /></Button>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
