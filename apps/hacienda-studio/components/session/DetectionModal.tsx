import * as React from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { RotateCcw, CheckSquare, Square } from "lucide-react";
import { DETECTION_CATEGORIES } from "@/lib/pii-categories";

export interface DetectionModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selected: Set<string>;
  onToggle: (code: string) => void;
  onToggleGroup: (group: string, all: boolean) => void;
  onDefaults: () => void;
}

export function DetectionModal({ open, onOpenChange, selected, onToggle, onToggleGroup, onDefaults }: DetectionModalProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>Données sélectionnées</DialogTitle>
          <DialogDescription>Choisissez les types de données à pseudonymiser.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4 max-h-[60vh] overflow-auto pr-2">
          {DETECTION_CATEGORIES.map(g => {
            const codes = g.categories.map(c => c.code);
            const allSelected = codes.every(c => selected.has(c));
            return (
              <div key={g.title} className="space-y-2 border-b pb-3 last:border-0">
                <div className="flex items-center justify-between">
                  <h4 className="text-xs font-medium uppercase tracking-wider flex items-center gap-2">
                    {g.title}
                    <button
                      type="button"
                      onClick={() => onToggleGroup(g.title, !allSelected)}
                      className="text-[10px] text-muted-foreground hover:text-foreground"
                    >
                      Tout sélectionner
                    </button>
                  </h4>
                </div>
                <div className="grid grid-cols-3 gap-3">
                  {g.categories.map(c => (
                    <label key={c.code} className="flex items-center gap-2 text-xs">
                      <Checkbox checked={selected.has(c.code)} onCheckedChange={()=>onToggle(c.code)} />
                      <Badge variant="outline" className="bg-cyan-50 text-cyan-700 shrink-0 text-[10px]">{c.code}</Badge>
                      <span className="truncate">{c.label}</span>
                    </label>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
        <div className="flex items-center justify-between border-t pt-3 text-xs">
          <span className="text-muted-foreground">{selected.size} sur {DETECTION_CATEGORIES.flatMap(g=>g.categories).length} entrées</span>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="sm" onClick={onDefaults}>
              <RotateCcw className="size-3 mr-1" /> Paramètres par défaut
            </Button>
            <Button variant="ghost" size="sm" onClick={()=>{
              const all = DETECTION_CATEGORIES.flatMap(g=>g.categories).map(c=>c.code);
              for (const c of all) if (!selected.has(c)) onToggle(c);
            }}>Tout sélectionner</Button>
            <Button variant="ghost" size="sm" onClick={()=>{
              const all = DETECTION_CATEGORIES.flatMap(g=>g.categories).map(c=>c.code);
              for (const c of all) if (selected.has(c)) onToggle(c);
            }}>Tout désélectionner</Button>
            <Button variant="ghost" onClick={()=>onOpenChange(false)}>Annuler</Button>
            <Button onClick={()=>onOpenChange(false)}>Enregistrer</Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
