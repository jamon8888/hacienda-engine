import * as React from "react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";

export function TexteSimple() {
  const [text, setText] = React.useState("");
  const [result, setResult] = React.useState<string>("");

  const handleProcess = () => {
    setResult(text.split("\n").filter(Boolean).join("\n[Processed]"));
  };

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Texte simple</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <Textarea value={text} onChange={e=>setText(e.target.value)} placeholder="Collez votre texte ici..." className="min-h-[200px]" />
          <div className="flex gap-2">
            <Button size="sm" onClick={handleProcess}>Traiter</Button>
            <Button size="sm" variant="outline">Effacer</Button>
          </div>
        </CardContent>
      </Card>
      {result && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Résultat</CardTitle>
          </CardHeader>
          <CardContent>
            <pre className="whitespace-pre-wrap text-xs">{result}</pre>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
