import * as React from "react";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Label } from "@/components/ui/label";

export interface ConversionPanelProps {
  mode: "original" | "markdown";
  onChange: (mode: "original" | "markdown") => void;
}

export function ConversionPanel({ mode, onChange }: ConversionPanelProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Conversion</CardTitle>
      </CardHeader>
      <CardContent>
        <RadioGroup value={mode} onValueChange={onChange as any}>
          <div className="flex items-center space-x-2">
            <RadioGroupItem value="original" id="orig" />
            <Label htmlFor="orig">Format original</Label>
          </div>
          <div className="flex items-center space-x-2">
            <RadioGroupItem value="markdown" id="md" />
            <Label htmlFor="md">Format Markdown</Label>
          </div>
        </RadioGroup>
      </CardContent>
    </Card>
  );
}
