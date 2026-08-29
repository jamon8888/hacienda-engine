import * as React from "react";
import { Search, Plus, Settings2, Trash2, User, Building2, MapPin, CreditCard, MoreHorizontal, Flag } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from "@/components/ui/accordion";
import { DETECTION_CATEGORIES } from "@/lib/pii-categories";
import type { PiiEntity } from "@/lib/pii-engine";

export interface CategorySidebarProps {
  readonly findings: ReadonlyArray<PiiEntity>;
  readonly onRemove?: (index: number) => void;
}

function displayTitle(title: string): string {
  switch (title) {
    case "DONNÉES PERSONNELLES": return "Données personnelles";
    case "DONNÉES D'ENTREPRISES": return "Données d'entreprises";
    case "DONNÉES DE LOCALISATION": return "Données de localisation";
    case "DONNÉES FINANCIÈRES": return "Données financières";
    case "DONNÉES DIVERSES": return "Données diverses";
    default: return title;
  }
}

function wireMatchesCategory(wire: string | { custom: string }, category: string): boolean {
  const cat = category.toLowerCase();
  if (typeof wire === "string") return wire.toLowerCase() === cat;
  return wire.custom.toLowerCase() === cat;
}

function groupIcon(title: string) {
  switch (title) {
    case "DONNÉES PERSONNELLES": return User;
    case "DONNÉES D'ENTREPRISES": return Building2;
    case "DONNÉES DE LOCALISATION": return MapPin;
    case "DONNÉES FINANCIÈRES": return CreditCard;
    case "DONNÉES DIVERSES": return MoreHorizontal;
    default: return Flag;
  }
}

export function CategorySidebar({ findings, onRemove }: CategorySidebarProps) {
  const [filter, setFilter] = React.useState("");

  const filteredFindings = findings.filter((f) => filter === "" || f.text.toLowerCase().includes(filter.toLowerCase()) || f.category.toLowerCase().includes(filter.toLowerCase()));

  // Extra group for Formats spécifiques : France (not in DETECTION_CATEGORIES but shown in screenshot)
  const extraGroup = { title: "Formats spécifiques : France", categories: [] as const, findings: [] as PiiEntity[] };

  return (
    <div className="flex h-full flex-col bg-white">
      <div className="p-3">
        <Button variant="outline" size="sm" className="w-full justify-center gap-2">
          <Plus size={14} /> Ajouter une détection
        </Button>
        <div className="relative mt-3">
          <Search size={14} className="absolute left-2.5 top-2.5 text-slate-400" />
          <Input
            placeholder="Rechercher dans les détections..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="pl-8"
          />
        </div>
      </div>
      <div className="flex-1 overflow-auto border-t border-slate-100">
        <Accordion type="multiple" className="w-full">
          {DETECTION_CATEGORIES.map((group) => {
            const Icon = groupIcon(group.title);
            const groupFindings = filteredFindings.filter((f) => group.categories.some((c) => wireMatchesCategory(c.wire, f.category)));
            return (
              <AccordionItem key={group.title} value={group.title}>
                <AccordionTrigger className="px-3 py-2 hover:no-underline">
                  <span className="flex items-center gap-2 text-xs font-medium">
                    <Icon size={14} className="text-slate-500" />
                    {displayTitle(group.title)}
                    {groupFindings.length > 0 ? <Badge variant="secondary" className="ml-1 text-xs">{String(groupFindings.length)}</Badge> : null}
                  </span>
                </AccordionTrigger>
                <AccordionContent className="px-3 pb-3">
                  {groupFindings.length === 0 ? (
                    <p className="py-2 text-xs text-slate-400">Aucune détection</p>
                  ) : (
                    <ul className="space-y-2">
                      {groupFindings.map((f, idx) => {
                        const originalIndex = findings.indexOf(f);
                        return (
                          <li key={idx} className="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-2 py-1.5">
                            <div className="flex items-center gap-2">
                              <Badge variant="outline" className="bg-cyan-50 text-cyan-700">CIE</Badge>
                              <span className="text-xs text-green-700">{f.text}</span>
                            </div>
                            <span className="flex items-center gap-1">
                              <Button variant="ghost" size="icon" className="h-6 w-6">
                                <Settings2 size={12} />
                              </Button>
                              {onRemove ? (
                                <Button variant="ghost" size="icon" className="h-6 w-6 text-red-400" onClick={() => onRemove(originalIndex)}>
                                  <Trash2 size={12} />
                                </Button>
                              ) : null}
                            </span>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </AccordionContent>
              </AccordionItem>
            );
          })}
          {/* 6th group */}
          <AccordionItem value={extraGroup.title}>
            <AccordionTrigger className="px-3 py-2 hover:no-underline">
              <span className="flex items-center gap-2 text-xs font-medium">
                <Flag size={14} className="text-slate-500" />
                {extraGroup.title}
              </span>
            </AccordionTrigger>
            <AccordionContent className="px-3 pb-3">
              <p className="py-2 text-xs text-slate-400">Aucune détection</p>
            </AccordionContent>
          </AccordionItem>
        </Accordion>
      </div>
    </div>
  );
}
