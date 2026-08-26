import * as React from "react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Table, TableHeader, TableRow, TableHead, TableBody, TableCell } from "@/components/ui/table";

export interface PseudonymEntry {
  category: string;
  original: string;
  pseudonym: string;
}

export interface PseudonymGridProps {
  entries: ReadonlyArray<PseudonymEntry>;
  onUpdate?: (index: number, pseudonym: string) => void;
  onAdd?: () => void;
}

export function PseudonymGrid({ entries, onUpdate, onAdd }: PseudonymGridProps) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-medium">Pseudonymes par catégorie</h2>
        <Button size="sm" onClick={onAdd}>Ajouter</Button>
      </div>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Catégorie</TableHead>
            <TableHead>Original</TableHead>
            <TableHead>Pseudonyme</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {entries.map((e, i) => (
            <TableRow key={i}>
              <TableCell><Badge variant="outline">{e.category}</Badge></TableCell>
              <TableCell className="font-mono text-xs">{e.original}</TableCell>
              <TableCell>
                <Input value={e.pseudonym} onChange={ev => onUpdate?.(i, ev.target.value)} className="h-8" />
              </TableCell>
            </TableRow>
          ))}
          {entries.length === 0 && (
            <TableRow>
              <TableCell colSpan={3} className="text-center text-xs text-muted-foreground py-8">Aucun pseudonyme généré</TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>
    </div>
  );
}
