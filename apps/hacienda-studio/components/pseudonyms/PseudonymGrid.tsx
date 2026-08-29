import * as React from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Settings, Trash2 } from "lucide-react";

export interface PseudonymGroup {
  category: string;
  label: string;
  entries: Array<{ id: string; original: string; pseudonym: string }>;
}

export interface PseudonymGridProps {
  groups: ReadonlyArray<PseudonymGroup>;
  search?: string;
  onCopy?: () => void;
  onUpdate?: (group: string, id: string, pseudonym: string) => void;
  onDelete?: (group: string, id: string) => void;
}

export function PseudonymGrid({ groups, search = "", onCopy, onUpdate, onDelete }: PseudonymGridProps) {
  const filtered = groups.map(g => ({
    ...g,
    entries: g.entries.filter(e => e.original.toLowerCase().includes(search.toLowerCase()) || e.pseudonym.toLowerCase().includes(search.toLowerCase())),
  })).filter(g => g.entries.length > 0);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Input placeholder="Rechercher des pseudonymes..." className="flex-1" />
        <Button size="sm" variant="outline" onClick={onCopy}>Copier les pseudonymes</Button>
      </div>
      {filtered.length === 0 && (
        <p className="text-center text-xs text-muted-foreground py-8">Aucun pseudonyme</p>
      )}
      {filtered.map(g => (
        <div key={g.category} className="space-y-2">
          <div className="flex items-center gap-2">
            <h3 className="text-sm font-medium">{g.label}</h3>
            <Badge variant="secondary" className="text-[10px]">{g.entries.length}</Badge>
          </div>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
            {g.entries.map(e => (
              <div key={e.id} className="flex items-center justify-between gap-2 rounded-md border bg-muted/30 px-2 py-1.5">
                <div className="flex items-center gap-2 min-w-0">
                  <Badge variant="outline" className="bg-cyan-50 text-cyan-700 shrink-0">{g.category}</Badge>
                  <span className="truncate text-xs font-mono">{e.pseudonym}</span>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <Button variant="ghost" size="icon" className="h-6 w-6" onClick={()=>onUpdate?.(g.category, e.id, e.pseudonym)}>
                    <Settings className="size-3" />
                  </Button>
                  <Button variant="ghost" size="icon" className="h-6 w-6 text-red-400" onClick={()=>onDelete?.(g.category, e.id)}>
                    <Trash2 className="size-3" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
