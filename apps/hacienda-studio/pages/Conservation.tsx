import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Label } from "@/components/ui/label";

export function Conservation() {
  const [period, setPeriod] = React.useState("30");

  return (
    <div className="space-y-4 max-w-xl">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Durée de conservation</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <RadioGroup value={period} onValueChange={setPeriod}>
            {["7","30","90","365"].map(p=>(
              <div key={p} className="flex items-center space-x-2">
                <RadioGroupItem value={p} id={p} />
                <Label htmlFor={p}>{p} jours</Label>
              </div>
            ))}
          </RadioGroup>
          <Button size="sm">Enregistrer</Button>
        </CardContent>
      </Card>
    </div>
  );
}
