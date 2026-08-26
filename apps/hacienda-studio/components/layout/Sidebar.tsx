import {
  House,
  Files,
  FileText,
  Archive,
  Brain,
  Code2,
  Clock,
  Users,
  HelpCircle,
} from "lucide-react";

interface SidebarProps {
  readonly activeItem?: string;
}

const SECTION_HEADER_CLASS =
  "px-3 pt-4 pb-1 text-[10px] font-medium uppercase tracking-widest text-[#94a3b8]";

const ITEM_BASE =
  "flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors";

const ITEM_INACTIVE = "text-[#475569] hover:bg-[#f1f5f9] hover:text-[#1e293b]";
const ITEM_ACTIVE = "bg-[#f5f3ff] text-[#6d28d9] font-medium";

function NavItem({
  icon: Icon,
  label,
  active,
  badge,
  disabled,
}: {
  readonly icon: typeof House;
  readonly label: string;
  readonly active?: boolean;
  readonly badge?: string;
  readonly disabled?: boolean;
}) {
  return (
    <div
      className={`${ITEM_BASE} ${active ? ITEM_ACTIVE : ITEM_INACTIVE} ${disabled ? "opacity-60" : ""}`}
      aria-current={active ? "page" : undefined}
      aria-disabled={disabled ? "true" : undefined}
    >
      <Icon size={16} className={active ? "text-[#6d28d9]" : "text-[#64748b]"} />
      <span className="flex-1 text-left">{label}</span>
      {badge ? (
        <span className="rounded bg-[#f1f5f9] px-1.5 py-0.5 text-[10px] font-normal text-[#64748b]">
          {badge}
        </span>
      ) : null}
    </div>
  );
}

export function Sidebar({ activeItem = "Sessions" }: SidebarProps) {
  return (
    <aside className="flex h-full w-[220px] shrink-0 flex-col border-r border-[#e2e8f0] bg-[#ffffff]">
      {/* Logo */}
      <div className="flex h-14 items-center gap-2 border-b border-[#f1f5f9] px-4">
        <div className="flex h-7 w-7 items-center justify-center rounded-md bg-[#6d28d9] text-[13px] font-bold text-white">
          H
        </div>
        <span className="text-sm font-semibold text-[#1e293b]">Hacienda</span>
      </div>

      {/* Scrollable nav */}
      <div className="flex-1 overflow-y-auto px-2 py-2">
        {/* Accueil */}
        <nav aria-label="Navigation principale">
          <div className={ITEM_BASE + " " + ITEM_INACTIVE}>
            <House size={16} className="text-[#64748b]" />
            <span>Accueil</span>
          </div>

          {/* Anonymisation */}
          <div className={SECTION_HEADER_CLASS}>Anonymisation</div>
          <ul className="space-y-0.5">
            <li>
              <NavItem icon={Files} label="Sessions" active={activeItem === "Sessions"} />
            </li>
            <li>
              <NavItem icon={FileText} label="Texte simple" />
            </li>
            <li>
              <NavItem icon={Archive} label="Archivage" badge="Bientôt disponible" disabled />
            </li>
          </ul>

          {/* Intégrations */}
          <div className={SECTION_HEADER_CLASS}>Intégrations</div>
          <ul className="space-y-0.5">
            <li>
              <NavItem icon={Brain} label="Claude Cowork" />
            </li>
            <li>
              <NavItem icon={Code2} label="API" />
            </li>
          </ul>

          {/* Préférences */}
          <div className={SECTION_HEADER_CLASS}>Préférences</div>
          <ul className="space-y-0.5">
            <li>
              <NavItem icon={Clock} label="Conservation" />
            </li>
          </ul>
        </nav>
      </div>

      {/* Footer */}
      <div className="border-t border-[#f1f5f9] px-2 py-3">
        <nav aria-label="Aide et invitation">
          <div className={`${ITEM_BASE} ${ITEM_INACTIVE}`}>
            <Users size={16} className="text-[#64748b]" />
            <span>Inviter</span>
          </div>
          <div className={`${ITEM_BASE} ${ITEM_INACTIVE}`}>
            <HelpCircle size={16} className="text-[#64748b]" />
            <span>Aide</span>
          </div>
        </nav>
      </div>
    </aside>
  );
}
