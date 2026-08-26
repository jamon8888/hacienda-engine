import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Label } from "@/components/ui/label";

export interface TreatmentPanelProps {
  mode: "pseudonymize" | "anonymize";
  onChange: (mode: "pseudonymize" | "anonymize") => void;
}

export function TreatmentPanel({ mode, onChange }: TreatmentPanelProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Traitement</CardTitle>
      </CardHeader>
      <CardContent>
        <RadioGroup value={mode} onValueChange={onChange as any}>
          <div className="flex items-center space-x-2">
            <RadioGroupItem value="pseudonymize" id="pseudo" />
            <Label htmlFor="pseudo">Pseudonymiser</Label>
          </div>
          <div className="flex items-center space-x-2">
            <RadioGroupItem value="anonymize" id="anon" />
            <Label htmlFor="anon">Anonymiser</Label>
          </div>
        </RadioGroup>
      </CardContent>
    </Card>
  );
}
