import * as React from "react";
import { Building2, CreditCard, Layers, MapPin, User } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import {
  DEFAULT_SELECTED_CODES,
  DETECTION_CATEGORIES,
  TOTAL_CATEGORIES,
} from "@/lib/pii-categories";

export interface DetectionModalProps {
  readonly open: boolean;
  readonly selection: ReadonlySet<string>;
  readonly onChange: (next: ReadonlySet<string>) => void;
  readonly onOpenChange?: (open: boolean) => void;
}

const BADGE_COLORS: Record<string, string> = {
  PR: "bg-rose-100 text-rose-700 border-rose-200",
  MAIL: "bg-pink-100 text-pink-700 border-pink-200",
  PHON: "bg-rose-100 text-rose-700 border-rose-200",
  AGE: "bg-slate-100 text-slate-600 border-slate-200",
  TR: "bg-slate-100 text-slate-600 border-slate-200",
  CIE: "bg-blue-100 text-blue-700 border-blue-200",
  CID: "bg-blue-100 text-blue-700 border-blue-200",
  ACT: "bg-sky-100 text-sky-700 border-sky-200",
  PROD: "bg-slate-100 text-slate-600 border-slate-200",
  ADR: "bg-teal-100 text-teal-700 border-teal-200",
  LOC: "bg-teal-100 text-teal-700 border-teal-200",
  CP: "bg-emerald-100 text-emerald-700 border-emerald-200",
  GEO: "bg-slate-100 text-slate-600 border-slate-200",
  CARD: "bg-orange-100 text-orange-700 border-orange-200",
  IBAN: "bg-orange-100 text-orange-700 border-orange-200",
  BIC: "bg-slate-100 text-slate-600 border-slate-200",
  URL: "bg-zinc-100 text-zinc-600 border-zinc-200",
  FILE: "bg-zinc-100 text-zinc-600 border-zinc-200",
  REF: "bg-violet-100 text-violet-700 border-violet-200",
  IP: "bg-slate-100 text-slate-600 border-slate-200",
  DT: "bg-slate-100 text-slate-600 border-slate-200",
  TEMP: "bg-slate-100 text-slate-600 border-slate-200",
  ORG: "bg-slate-100 text-slate-600 border-slate-200",
  NIR: "bg-red-100 text-red-700 border-red-200",
  SIREN: "bg-blue-100 text-blue-700 border-blue-200",
  SIRET: "bg-blue-100 text-blue-700 border-blue-200",
  TVA: "bg-amber-100 text-amber-700 border-amber-200",
};

function badgeColor(code: string): string {
  return BADGE_COLORS[code] ?? "bg-slate-100 text-slate-600 border-slate-200";
}

function groupIcon(title: string): React.ReactNode {
  switch (title) {
    case "DONNÉES PERSONNELLES":
      return <User size={14} className="text-slate-500" />;
    case "DONNÉES D'ENTREPRISES":
      return <Building2 size={14} className="text-slate-500" />;
    case "DONNÉES DE LOCALISATION":
      return <MapPin size={14} className="text-slate-500" />;
    case "DONNÉES FINANCIÈRES":
      return <CreditCard size={14} className="text-slate-500" />;
    case "DONNÉES DIVERSES":
      return <Layers size={14} className="text-slate-500" />;
    default:
      return null;
  }
}

export function DetectionModal({
  open,
  selection,
  onChange,
  onOpenChange,
}: DetectionModalProps) {
  function toggleCode(code: string): void {
    const next = new Set(selection);
    if (next.has(code)) next.delete(code);
    else next.add(code);
    onChange(next);
  }

  function toggleGroup(codes: readonly string[]): void {
    const allSelected = codes.every((c) => selection.has(c));
    const next = new Set(selection);
    if (allSelected) {
      for (const c of codes) next.delete(c);
    } else {
      for (const c of codes) next.add(c);
    }
    onChange(next);
  }

  function selectAll(): void {
    const next = new Set<string>();
    for (const g of DETECTION_CATEGORIES) {
      for (const c of g.categories) next.add(c.code);
    }
    onChange(next);
  }

  function deselectAll(): void {
    onChange(new Set());
  }

  function resetDefaults(): void {
    onChange(new Set(DEFAULT_SELECTED_CODES));
  }

  const count = selection.size;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] max-w-3xl flex-col gap-0 p-0">
        <DialogHeader className="border-b px-6 pb-4 pt-6 text-left">
          <DialogTitle className="text-base font-semibold">Données sélectionnées</DialogTitle>
          <DialogDescription className="text-sm text-slate-500">
            Choisissez les types de données à pseudonymiser.
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto px-6 py-4">
          <div className="space-y-4">
            {DETECTION_CATEGORIES.map((group) => {
              const groupCodes = group.categories.map((c) => c.code);
              const allSelected = groupCodes.every((c) => selection.has(c));
              return (
                <div
                  key={group.title}
                  className="rounded-lg border border-slate-200 bg-slate-50 p-4"
                >
                  <div className="mb-3 flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      {groupIcon(group.title)}
                      <span className="text-[11px] font-semibold uppercase tracking-widest text-slate-700">
                        {group.title}
                      </span>
                    </div>
                    <button
                      type="button"
                      onClick={() => toggleGroup(groupCodes)}
                      className="text-xs font-medium text-violet-600 hover:text-violet-700 hover:underline"
                    >
                      {allSelected ? "Tout désélectionner" : "Tout sélectionner"}
                    </button>
                  </div>
                  <Separator className="mb-3 bg-slate-200" />
                  <div className="grid grid-cols-3 gap-3">
                    {group.categories.map((cat) => {
                      const checked = selection.has(cat.code);
                      const id = `detection-${cat.code}`;
                      return (
                        <div key={cat.code} className="flex items-center gap-2">
                          <Checkbox
                            id={id}
                            checked={checked}
                            onCheckedChange={() => toggleCode(cat.code)}
                            aria-label={cat.label}
                          />
                          <Badge
                            variant="outline"
                            className={`h-5 min-w-[42px] justify-center rounded px-1.5 text-[10px] font-semibold leading-none ${badgeColor(cat.code)}`}
                          >
                            {cat.code}
                          </Badge>
                          <Label
                            htmlFor={id}
                            className="cursor-pointer text-xs font-normal leading-none text-slate-700"
                          >
                            {cat.label}
                          </Label>
                        </div>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        <div className="sticky bottom-0 flex items-center justify-between border-t bg-white px-6 py-4">
          <span className="text-sm font-medium text-slate-700">
            {String(count)} sur {String(TOTAL_CATEGORIES)} sélectionnés
          </span>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={resetDefaults}>
              Paramètres par défaut
            </Button>
            <button
              type="button"
              onClick={selectAll}
              className="px-3 py-1.5 text-sm font-medium text-violet-600 hover:text-violet-700 hover:underline"
            >
              Tout sélectionner
            </button>
            <button
              type="button"
              onClick={deselectAll}
              className="px-3 py-1.5 text-sm font-medium text-slate-600 hover:text-slate-900 hover:underline"
            >
              Tout désélectionner
            </button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
