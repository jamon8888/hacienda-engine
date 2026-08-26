import type { ReactNode } from "react";
import type { Session } from "@/lib/session";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";

export interface AppShellProps {
  readonly sessions?: Session[];
  readonly activeSession?: Session | null;
  readonly onNewSession?: () => void;
  readonly children?: ReactNode;
}

export function AppShell({
  sessions: _sessions = [],
  activeSession = null,
  onNewSession,
  children,
}: AppShellProps) {
  const sessionName = activeSession?.name ?? "Session du 26/08";

  return (
    <div className="flex min-h-screen flex-row bg-[#f8fafc]">
      <Sidebar activeItem="Sessions" />
      <div className="flex min-h-screen flex-1 flex-col">
        <Topbar sessionName={sessionName} onNewSession={onNewSession} />
        <main className="flex-1 overflow-auto bg-[#f8fafc]">{children}</main>
      </div>
    </div>
  );
}
