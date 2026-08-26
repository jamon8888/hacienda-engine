import { createFileRoute } from "@tanstack/react-router";
import { NouvelleSession } from "@/pages/NouvelleSession";

export const Route = createFileRoute("/")({
  component: HomePage,
});

function HomePage() {
  return <NouvelleSession />;
}
