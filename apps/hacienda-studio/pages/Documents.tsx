import { useMemo, useState } from "react";
import { Folder, FileText, Search, Upload, Archive, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { groupByFolder, baseNameOf, ROOT_FOLDER, type LibraryDocument } from "@/lib/library";
import { exportDocumentsZip } from "@/lib/export-zip";

function formatKB(markdown: string): string {
  return `${Math.max(1, Math.round(new Blob([markdown]).size / 1024))} KB`;
}

export function Documents({
  documents,
  onOpenDocument,
  onDeleteDocuments,
  onAddFiles,
}: {
  documents: LibraryDocument[];
  onOpenDocument: (name: string) => void;
  onDeleteDocuments: (names: string[]) => void;
  onAddFiles: () => void;
}) {
  const [activeFolder, setActiveFolder] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const folders = useMemo(() => groupByFolder(documents), [documents]);
  const visible = useMemo(() => {
    let list = activeFolder === null ? documents : documents.filter((d) => d.folder === activeFolder);
    if (query.trim()) {
      const q = query.toLowerCase();
      list = list.filter((d) => baseNameOf(d.result.name).toLowerCase().includes(q));
    }
    return list;
  }, [documents, activeFolder, query]);

  function toggle(name: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  const selectedDocs = documents.filter((d) => selected.has(d.result.name));

  return (
    <div className="flex flex-1 gap-6 px-6 py-8">
      <div>
        <div className="mb-6 flex items-center justify-between">
          <div>
            <h1 className="text-2xl font-semibold">Documents</h1>
            <p className="text-sm text-muted-foreground">
              {documents.length} processed · stored locally on this device
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={onAddFiles}>
              <Upload className="size-4" /> Add files
            </Button>
            <Button
              disabled={documents.length === 0}
              onClick={() => exportDocumentsZip(selectedDocs.length > 0 ? selectedDocs : visible)}
            >
              <Archive className="size-4" /> Export
            </Button>
            {selected.size > 0 && (
              <Button
                variant="destructive"
                onClick={() => {
                  onDeleteDocuments([...selected]);
                  setSelected(new Set());
                }}
              >
                <Trash2 className="size-4" /> Delete ({selected.size})
              </Button>
            )}
          </div>
        </div>
      </div>

      <div className="flex flex-1 gap-6">
        <aside className="w-56 shrink-0">
          <button
            type="button"
            onClick={() => setActiveFolder(null)}
            className={
              "flex w-full items-center justify-between rounded-md px-3 py-2 text-sm " +
              (activeFolder === null ? "bg-muted font-medium" : "text-muted-foreground hover:bg-muted/60")
            }
          >
            <span className="flex items-center gap-2">
              <FileText className="size-4" /> All documents
            </span>
            <span>{documents.length}</span>
          </button>
          {folders.map((f) => (
            <button
              key={f.name}
              type="button"
              onClick={() => setActiveFolder(f.name)}
              className={
                "mt-1 flex w-full items-center justify-between rounded-md px-3 py-2 text-sm " +
                (activeFolder === f.name ? "bg-muted font-medium" : "text-muted-foreground hover:bg-muted/60")
              }
            >
              <span className="flex items-center gap-2 truncate">
                <Folder className="size-4 text-primary" />
                {f.name === ROOT_FOLDER ? "Root" : f.name}
              </span>
              <span>{f.documents.length}</span>
            </button>
          ))}
        </aside>

        <div className="flex-1 rounded-lg border border-border">
          <div className="flex items-center gap-2 border-b border-border px-3 py-2">
            <Search className="size-4 text-muted-foreground" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search all folders"
              className="w-full bg-transparent text-sm outline-none"
            />
          </div>

          {visible.length === 0 ? (
            <p className="p-8 text-center text-sm text-muted-foreground">
              No documents yet.
            </p>
          ) : (
            <ul>
              {visible.map((doc) => (
                <li
                  key={doc.result.name}
                  className="flex items-center gap-3 border-b border-border px-3 py-3 last:border-b-0 hover:bg-muted/40"
                >
                  <input
                    type="checkbox"
                    checked={selected.has(doc.result.name)}
                    onChange={() => toggle(doc.result.name)}
                    className="accent-primary"
                  />
                  <FileText className="size-4 text-muted-foreground" />
                  <button
                    type="button"
                    className="flex-1 truncate text-left text-sm"
                    onClick={() => onOpenDocument(doc.result.name)}
                  >
                    {baseNameOf(doc.result.name)}
                  </button>
                  <span className="rounded-full border border-border px-2 py-0.5 text-xs text-muted-foreground">
                    {doc.findings.length} finding{doc.findings.length === 1 ? "" : "s"}
                  </span>
                  <span className="w-16 text-right text-xs text-muted-foreground">
                    {formatKB(doc.result.markdown)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
