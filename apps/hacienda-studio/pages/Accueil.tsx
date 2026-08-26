import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";

export function Accueil() {
  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Marvin Systems</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground">Carte 1 – informations et actions rapides.</p>
          <Button size="sm">Nouvelle session</Button>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Marvin Systems</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground">Carte 2 – statistiques et accès récents.</p>
          <Button size="sm" variant="outline">Voir détails</Button>
        </CardContent>
      </Card>
    </div>
  );
}
