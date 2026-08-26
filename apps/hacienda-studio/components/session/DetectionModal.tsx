import * as React from "react";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { DETECTION_CATEGORIES } from "@/lib/pii-categories";

export interface DetectionModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selected: Set<string>;
  onToggle: (code: string) => void;
}

export function DetectionModal({ open, onOpenChange, selected, onToggle }: DetectionModalProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Données sélectionnées</DialogTitle>
        </DialogHeader>
        <div className="space-y-4 max-h-[60vh] overflow-auto">
          {DETECTION_CATEGORIES.map(g => (
            <div key={g.title}>
              <h4 className="text-xs font-medium mb-2">{g.title}</h4>
              <div className="grid grid-cols-2 gap-2">
                {g.categories.map(c => (
                  <label key={c.code} className="flex items-center gap-2 text-xs">
                    <Checkbox checked={selected.has(c.code)} onCheckedChange={()=>onToggle(c.code)} />
                    {c.label}
                  </label>
                ))}
              </div>
            </div>
          ))}
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={()=>onOpenChange(false)}>Annuler</Button>
          <Button onClick={()=>onOpenChange(false)}>Enregistrer</Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
