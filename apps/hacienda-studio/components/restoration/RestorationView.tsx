import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Upload } from "lucide-react";

export function RestorationView() {
  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Restauration</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="rounded-lg border border-dashed p-6 text-center">
            <Upload className="mx-auto mb-2 h-6 w-6 text-muted-foreground" />
            <p className="text-xs text-muted-foreground">Déposez le fichier de session ou collez la clé de restauration</p>
            <Button variant="outline" size="sm" className="mt-3">Importer</Button>
          </div>
          <Textarea placeholder="Clé de restauration..." className="min-h-[100px]" />
          <Button size="sm">Restaurer</Button>
        </CardContent>
      </Card>
    </div>
  );
}
