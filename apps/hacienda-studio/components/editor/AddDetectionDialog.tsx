import * as React from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Accordion, AccordionItem, AccordionTrigger, AccordionContent } from "@/components/ui/accordion";
import { Label } from "@/components/ui/label";
import { DETECTION_CATEGORIES } from "@/lib/pii-categories";

export interface AddDetectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (start: number, end: number, category: string) => void;
}

export function AddDetectionDialog({ open, onOpenChange, onAdd }: AddDetectionDialogProps) {
  const [query, setQuery] = React.useState("");
  const [selectedCode, setSelectedCode] = React.useState<string | null>(null);
  const [start, setStart] = React.useState<number>(0);
  const [end, setEnd] = React.useState<number>(0);

  React.useEffect(() => {
    if (open) {
      setSelectedCode(null);
      setQuery("");
    }
  }, [open]);

  const filtered = DETECTION_CATEGORIES.flatMap(g => g.categories.filter(c => c.label.toLowerCase().includes(query.toLowerCase())));

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Ajouter une détection</DialogTitle>
          <DialogDescription>Sélectionnez la catégorie PII à ajouter à la sélection.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <Input placeholder="Filtrer les catégories..." value={query} onChange={e => setQuery(e.target.value)} />
          <Accordion type="multiple" className="max-h-[300px] overflow-auto">
            {DETECTION_CATEGORIES.map(g => (
              <AccordionItem key={g.title} value={g.title}>
                <AccordionTrigger>{g.title}</AccordionTrigger>
                <AccordionContent>
                  <ul className="space-y-1">
                    {g.categories.filter(c => c.label.toLowerCase().includes(query.toLowerCase())).map(c => (
                      <li key={c.code}>
                        <Button variant={selectedCode===c.code?"default":"outline"} size="sm" className="w-full justify-start" onClick={()=>setSelectedCode(c.code)}>
                          {c.label} <span className="ml-auto text-xs">{c.code}</span>
                        </Button>
                      </li>
                    ))}
                  </ul>
                </AccordionContent>
              </AccordionItem>
            ))}
          </Accordion>
          <div className="grid grid-cols-2 gap-2">
            <Label htmlFor="start">Début</Label>
            <Input id="start" type="number" value={start} onChange={e=>setStart(parseInt(e.target.value||"0",10))} />
            <Label htmlFor="end">Fin</Label>
            <Input id="end" type="number" value={end} onChange={e=>setEnd(parseInt(e.target.value||"0",10))} />
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={()=>onOpenChange(false)}>Annuler</Button>
            <Button disabled={!selectedCode} onClick={()=>{ if(selectedCode){ onAdd(start,end,selectedCode); onOpenChange(false);} }}>Ajouter</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
