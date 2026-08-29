import { Pencil } from "lucide-react";
import { Button } from "@/components/ui/button";

interface TopbarProps {
  readonly sessionName?: string;
  readonly pseudonymCount?: number;
  readonly activeTab?: string;
  readonly onNewSession?: () => void;
  readonly onTabChange?: (tab: string) => void;
}

const TABS = [
  { id: "overview", label: "Vue d'ensemble" },
  { id: "editor", label: "Éditeur" },
  { id: "pseudonyms", label: "Pseudonymes" },
  { id: "restore", label: "Restauration" },
] as const;

export function Topbar({
  sessionName = "Session du 26/08",
  pseudonymCount = 23,
  activeTab = "overview",
  onNewSession,
  onTabChange,
}: TopbarProps) {
  return (
    <div className="flex flex-col border-b border-[#e2e8f0] bg-[#ffffff]">
      {/* Row 1: breadcrumb + actions */}
      <div className="flex h-12 items-center justify-between gap-4 px-6">
        <div className="flex items-center gap-2 text-sm">
          <span className="text-[#64748b]">Sessions</span>
          <span className="text-[#cbd5e1]">/</span>
          <span className="font-medium text-[#1e293b]">{sessionName}</span>
          <button
            type="button"
            aria-label="Renommer la session"
            className="ml-1 rounded p-1 text-[#94a3b8] hover:bg-[#f1f5f9] hover:text-[#475569]"
          >
            <Pencil size={14} />
          </button>
        </div>

        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onNewSession}>
            Nouvelle session
          </Button>
          <Button
            size="sm"
            className="bg-[#3b82f6] text-white hover:bg-[#2563eb]"
            onClick={onNewSession}
          >
            S&apos;inscrire et télécharger la session
          </Button>
        </div>
      </div>

      {/* Row 2: tabs */}
      <div className="flex items-center gap-6 px-6">
        <nav aria-label="Onglets de session" className="flex gap-6">
          {TABS.map((tab) => {
            const isActive = tab.id === activeTab;
            const label =
              tab.id === "pseudonyms" ? `${tab.label} ${String(pseudonymCount)}` : tab.label;
            const countBadge =
              tab.id === "pseudonyms" ? (
                <span className="ml-1 rounded bg-[#f1f5f9] px-1.5 py-0.5 text-[10px] leading-none text-[#64748b]">
                  {String(pseudonymCount)}
                </span>
              ) : null;
            // For test visibility we render plain text label without separate badge duplication
            // but keep accessible split.
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={isActive}
                onClick={() => onTabChange?.(tab.id)}
                className={`relative flex items-center border-b-2 px-1 py-3 text-sm transition-colors ${
                  isActive
                    ? "border-[#6d28d9] font-medium text-[#6d28d9]"
                    : "border-transparent text-[#64748b] hover:text-[#1e293b]"
                }`}
              >
                {tab.id === "pseudonyms" ? (
                  <>
                    <span>Pseudonymes</span>
                    {countBadge}
                  </>
                ) : (
                  label
                )}
              </button>
            );
          })}
        </nav>
      </div>
    </div>
  );
}
