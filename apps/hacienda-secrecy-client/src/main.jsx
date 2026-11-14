import React, { useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { EditorContent, useEditor } from "@tiptap/react";
import { Mark, mergeAttributes } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { createRendererBridge, createTurnRequest, normalizePiiClass, readTokenizedChunks, tokenizedTextToDocument } from "./tauri-bridge.js";
import "./styles.css";

const SensitiveMark = Mark.create({
  name: "haciendaSensitive",
  inclusive: false,
  addAttributes() {
    return {
      tokenClass: { default: "sensitive" },
      tokenId: { default: "manual" },
      mode: { default: "manual-mask" },
      source: { default: "manual" },
      disclosure: { default: "viewer-only" },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-hacienda-sensitive]" }];
  },
  renderHTML({ mark, HTMLAttributes }) {
    const tokenClass = normalizePiiClass(mark.attrs.tokenClass);
    const categoryLabel = CLASS_LABELS[tokenClass] ?? CLASS_LABELS.sensitive;
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-hacienda-sensitive": "true",
        "data-token-class": mark.attrs.tokenClass,
        "aria-label": `Jeton de catégorie ${categoryLabel}`,
        class: `hacienda-sensitive-token pii-token pii-${tokenClass}`,
      }),
      ["span", { class: "token-value" }, 0],
      ["span", { class: "token-class-label" }, categoryLabel],
    ];
  },
});

const CLASS_LABELS = {
  person: "Personne",
  organization: "Organisation",
  email: "Adresse e-mail",
  phone: "Téléphone",
  matter: "Dossier",
  location: "Lieu",
  address: "Adresse",
  date: "Date",
  identity: "Identifiant officiel",
  financial: "Donnée financière",
  health: "Donnée de santé",
  credential: "Identifiant d’accès",
  network: "Identifiant réseau",
  sensitive: "Donnée sensible",
};

let sessionNumber = 0;
function createSessionIds() {
  sessionNumber += 1;
  const session = `${Date.now().toString(36)}-${sessionNumber.toString(36)}`;
  return { conversationId: `conversation-${session}`, contextId: `context-${session}` };
}

function TokenizedChunk({ chunk, label }) {
  const document = useMemo(() => tokenizedTextToDocument(chunk), [chunk]);
  const editor = useEditor({
    extensions: [StarterKit.configure({ history: false }), SensitiveMark],
    content: document,
    editable: false,
    editorProps: {
      attributes: {
        class: "token-history-editor",
        role: "group",
        "aria-label": label,
      },
    },
  });
  return editor ? <EditorContent editor={editor} /> : <p>Chargement de la réponse tokenisée…</p>;
}

function TokenizedHistory({ turns }) {
  if (!turns.length) {
    return <p className="empty-history">Les réponses tokenisées apparaîtront ici. Le texte saisi n’est pas ajouté à l’historique.</p>;
  }
  return (
    <ol className="history" aria-label="Historique des réponses tokenisées" aria-live="polite">
      {turns.map((turn, index) => (
        <li className="history-turn" key={`${turn.sequence}-${index}`}>
          <div className="turn-heading">
            <span>Réponse</span>
            <span>{turn.provider === "claude" ? "Claude" : "Codex"}</span>
          </div>
          {turn.chunks.map((chunk, chunkIndex) => (
            <TokenizedChunk chunk={chunk} label={`Réponse tokenisée, extrait ${chunkIndex + 1}`} key={`${chunk.sequence}-${chunkIndex}`} />
          ))}
        </li>
      ))}
    </ol>
  );
}

function OriginalViewer() {
  return (
    <details className="original-panel">
      <summary>Ouvrir l’original dans le viewer natif</summary>
      <p className="status" role="status">
        Viewer indisponible dans cette session : le renderer ne reçoit pas de références opaques
        issues par un viewer autorisé, expirant et audité. Aucun contenu original n’est affiché ici.
      </p>
    </details>
  );
}

function ManualPseudonymizationUnavailable() {
  const [category, setCategory] = useState("person");
  return (
    <section className="manual-panel" aria-labelledby="manual-title">
      <div>
        <p className="eyebrow">ÉDITION TOKEN-ONLY</p>
        <h3 id="manual-title">Pseudonymiser une sélection</h3>
      </div>
      <div className="manual-controls">
        <label>
          <span>Catégorie</span>
          <select value={category} onChange={(event) => setCategory(event.target.value)}>
            <option value="person">Personne</option>
            <option value="organization">Organisation</option>
            <option value="email">Adresse e-mail</option>
            <option value="phone">Téléphone</option>
            <option value="address">Adresse</option>
            <option value="financial">Donnée financière</option>
            <option value="health">Donnée de santé</option>
          </select>
        </label>
        <button type="button" disabled aria-disabled="true" title="Commande Tauri indisponible">
          Pseudonymiser la sélection
        </button>
      </div>
      <p className="status" role="status">
        Action indisponible : la commande Tauri `manual-pseudonymize-selection` n’est pas
        présente dans ce runtime. Aucune annotation ni modification de document n’est appliquée.
      </p>
    </section>
  );
}

function App() {
  const ids = useMemo(createSessionIds, []);
  const bridge = useMemo(() => createRendererBridge(invoke), []);
  const [provider, setProvider] = useState("claude");
  const [turns, setTurns] = useState([]);
  const [conversationContext, setConversationContext] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [hasContent, setHasContent] = useState(false);
  const editor = useEditor({
    extensions: [StarterKit.configure({ history: false }), SensitiveMark],
    content: "",
    editorProps: {
      attributes: {
        class: "composer-editor",
        id: "message-composer",
        "aria-label": "Message à envoyer au fournisseur",
        "aria-multiline": "true",
      },
    },
    onUpdate({ editor: currentEditor }) {
      setHasContent(!currentEditor.isEmpty);
    },
  });

  async function submit(event) {
    event.preventDefault();
    if (!editor || busy) return;
    const prompt = editor.getText({ blockSeparator: "\n" }).trim();
    if (!prompt) return;

    editor.commands.clearContent(true);
    setBusy(true);
    setError("");
    try {
      const request = createTurnRequest({ ...ids, prompt, context: conversationContext });
      const response = await bridge.stream(provider, request);
      const chunks = readTokenizedChunks(response, ids.conversationId, ids.contextId)
        .sort((left, right) => left.sequence - right.sequence);
      const turnNumber = conversationContext.length / 2 + 1;
      setConversationContext((current) => [
        ...current,
        { label: `User turn ${turnNumber}`, content: prompt },
        { label: `Assistant turn ${turnNumber}`, content: chunks.map((chunk) => chunk.text).join("\n") },
      ]);
      setTurns((current) => [...current, { provider, chunks, sequence: current.length }]);
    } catch {
      setError("Échec fermé : le fournisseur ou le filtre local est indisponible. Aucun texte de réponse n’a été ajouté à l’historique.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="shell">
      <header className="topbar">
        <div className="brand-mark" aria-hidden="true">H</div>
        <div>
          <p className="eyebrow">HACIENDA · LOCAL FIRST</p>
          <h1>Secrecy Client</h1>
        </div>
        <span className="session-badge"><span className="session-dot" />Session éphémère</span>
      </header>

      <section className="workspace" aria-label="Conversation sécurisée">
        <div className="section-title">
          <div>
            <p className="eyebrow">ESPACE DE TRAVAIL</p>
            <h2>Conversation tokenisée</h2>
          </div>
          <label className="provider-picker">
            <span>Fournisseur</span>
            <select value={provider} onChange={(event) => setProvider(event.target.value)} disabled={busy}>
              <option value="claude">Claude Desktop</option>
              <option value="codex">Codex</option>
            </select>
          </label>
        </div>

        <TokenizedHistory turns={turns} />

        <form className="composer" onSubmit={submit}>
          <label className="composer-label" htmlFor="message-composer">Votre message</label>
          <div className="editor-frame">
            <EditorContent editor={editor} />
          </div>
          <div className="composer-footer">
            <p>Le message est envoyé au bridge Tauri pour traitement local avant transport.</p>
            <button type="submit" disabled={!editor || busy || !hasContent}>
              {busy ? "Traitement local…" : "Envoyer"}
            </button>
          </div>
        </form>
        {error && <p className="error-banner" role="alert">{error}</p>}

        <ManualPseudonymizationUnavailable />
        <OriginalViewer />
      </section>
      <footer className="footer-note">Les conversations et les messages ne sont pas enregistrés sur cet appareil.</footer>
    </main>
  );
}

createRoot(document.getElementById("app")).render(<App />);
