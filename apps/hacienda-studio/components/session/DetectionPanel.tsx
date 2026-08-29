import * as React from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";

export interface DetectionPanelProps {
  selectedCount: number;
  totalCount: number;
  onOpenModal: () => void;
}

export function DetectionPanel({ selectedCount, totalCount, onOpenModal }: DetectionPanelProps) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-sm">Détection</CardTitle>
        <Badge variant="secondary">{selectedCount}/{totalCount}</Badge>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-xs text-muted-foreground">Sélectionner les données à détecter</p>
        <Button size="sm" onClick={onOpenModal}>Sélectionner les données</Button>
      </CardContent>
    </Card>
  );
}
