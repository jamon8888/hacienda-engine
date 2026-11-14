import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { EditorContent, useEditor } from "@tiptap/react";
import { Mark, mergeAttributes } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import { canSendToProvider, createRendererBridge, createTurnRequest, normalizePiiClass, readTokenizedChunks, tokenizedTextToDocument } from "./tauri-bridge.js";
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

function TokenizedHistory({ turns, onOpenViewer, openingViewer }) {
  if (!turns.length) {
    return <p className="empty-history">Les réponses tokenisées apparaîtront ici. Le texte saisi n’est pas ajouté à l’historique.</p>;
  }
  return (
    <ol className="history" aria-label="Historique des réponses tokenisées" aria-live="polite">
      {turns.map((turn, index) => (
        <li className="history-turn" key={`${turn.sequence}-${index}`}>
          <div className="turn-heading">
            <span>{turn.provider === "claude" ? "Claude" : "Codex"} · réponse tokenisée</span>
            <button
              type="button"
              className="viewer-button"
              disabled={openingViewer || !turn.chunks.some((chunk) => chunk.references.some((reference) => chunk.text.includes(reference.token)))}
              onClick={() => onOpenViewer(turn.chunks)}
            >
              {openingViewer ? "Ouverture…" : "Ouvrir le viewer local"}
            </button>
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

function ManualPseudonymizationPanel({ category, onCategoryChange, disabled, onPseudonymize }) {
  return (
    <section className="manual-panel" aria-labelledby="manual-title">
      <div>
        <p className="eyebrow">ÉDITION TOKEN-ONLY</p>
        <h3 id="manual-title">Pseudonymiser une sélection</h3>
      </div>
      <div className="manual-controls">
        <label>
          <span>Catégorie</span>
          <select value={category} onChange={(event) => onCategoryChange(event.target.value)} disabled={disabled}>
            <option value="person">Personne</option>
            <option value="organization">Organisation</option>
            <option value="email">Adresse e-mail</option>
            <option value="phone_number">Téléphone</option>
            <option value="address">Adresse</option>
            <option value="credit_card">Carte bancaire</option>
            <option value="date_of_birth">Date de naissance</option>
          </select>
        </label>
        <button type="button" disabled={disabled} onClick={onPseudonymize}>
          Pseudonymiser la sélection
        </button>
      </div>
      <p className="status" role="status">
        Sélectionnez le texte du message. La valeur brute passe uniquement par la commande locale;
        le texte sélectionné est remplacé par le token avant l’envoi.
      </p>
    </section>
  );
}

const VIEWER_TTL_MS = 59_000;

async function closeViewerWindow() {
  try {
    await getCurrentWindow().close();
  } catch {
    // The capability may already have expired and closed the native window.
  }
}

function LocalViewer() {
  const bridge = useMemo(() => createRendererBridge(invoke), []);
  const sessionActive = useRef(true);
  const [payload, setPayload] = useState(null);
  const [revealed, setRevealed] = useState({});
  const [loadingToken, setLoadingToken] = useState("");
  const [status, setStatus] = useState("Chargement du viewer local token-only…");

  useEffect(() => {
    let active = true;
    const closeTimer = window.setTimeout(() => {
      if (!active) return;
      sessionActive.current = false;
      setRevealed({});
      setStatus("Session locale expirée. Fermeture du viewer.");
      void closeViewerWindow();
    }, VIEWER_TTL_MS);

    bridge.localViewerContent()
      .then((content) => {
        if (!active) return;
        setPayload(content);
        setStatus("");
      })
      .catch(() => {
        if (!active) return;
        sessionActive.current = false;
        setRevealed({});
        setStatus("Session viewer indisponible ou expirée. Fermeture sécurisée.");
        window.setTimeout(() => void closeViewerWindow(), 700);
      });

    return () => {
      active = false;
      sessionActive.current = false;
      window.clearTimeout(closeTimer);
    };
  }, [bridge]);

  async function reveal(token) {
    if (!payload || loadingToken) return;
    setLoadingToken(token);
    setStatus("");
    try {
      const value = await bridge.revealLocalToken(token);
      if (!sessionActive.current) return;
      setRevealed((current) => ({ ...current, [token]: value }));
    } catch {
      if (!sessionActive.current) return;
      sessionActive.current = false;
      setRevealed({});
      setStatus("Session viewer expirée ou token indisponible. Fermeture sécurisée.");
      await closeViewerWindow();
    } finally {
      if (sessionActive.current) setLoadingToken("");
    }
  }

  return (
    <main className="shell local-viewer-shell">
      <header className="topbar">
        <div className="brand-mark" aria-hidden="true">H</div>
        <div>
          <p className="eyebrow">VIEWER LOCAL · SESSION 60 S</p>
          <h1>Révélation contrôlée</h1>
        </div>
        <button type="button" className="close-viewer" onClick={() => { sessionActive.current = false; setRevealed({}); void closeViewerWindow(); }}>Fermer</button>
      </header>
      {status && <p className="status viewer-status" role="status">{status}</p>}
      {payload && (
        <section className="workspace" aria-label="Contenu tokenisé dans le viewer local">
          <h2>Contenu tokenisé</h2>
          <TokenizedChunk chunk={payload.tokenized} label="Document tokenisé du viewer local" />
          <div className="viewer-references">
            <h3>Tokens révélables un par un</h3>
            {payload.references.map((reference) => (
              <div className="viewer-reference" key={`${reference.token}-${reference.class}`}>
                <span className={`pii-token pii-${normalizePiiClass(reference.class)}`}>
                  <span className="token-value">{reference.token}</span>
                  <span className="token-class-label">{CLASS_LABELS[normalizePiiClass(reference.class)] ?? CLASS_LABELS.sensitive}</span>
                </span>
                <button type="button" disabled={Boolean(loadingToken) || Boolean(revealed[reference.token])} onClick={() => void reveal(reference.token)}>
                  {loadingToken === reference.token ? "Révélation…" : revealed[reference.token] ? "Révélé" : "Révéler"}
                </button>
                {revealed[reference.token] && <p className="revealed-value" role="status">{revealed[reference.token]}</p>}
              </div>
            ))}
          </div>
          <p className="viewer-disclosure">Chaque clic demande un reveal local audité à usage unique. Cette fenêtre ne transmet jamais le plaintext au fournisseur.</p>
        </section>
      )}
    </main>
  );
}

function buildLocalViewerPayload(chunks) {
  const text = chunks.map((chunk) => chunk.text).join("\n");
  const references = new Map();
  for (const chunk of chunks) {
    for (const reference of chunk.references) {
      if (text.includes(reference.token)) {
        references.set(`${reference.token}\u0000${reference.class}`, reference);
      }
    }
  }
  const tokenReferences = [...references.values()];
  return { tokenized: { text, references: tokenReferences }, references: tokenReferences };
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
  const [hasSelection, setHasSelection] = useState(false);
  const [manualCategory, setManualCategory] = useState("person");
  const [piiStatus, setPiiStatus] = useState(null);
  const [openingViewer, setOpeningViewer] = useState(false);
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
    onSelectionUpdate({ editor: currentEditor }) {
      const { from, to, empty } = currentEditor.state.selection;
      setHasSelection(!empty && currentEditor.state.doc.textBetween(from, to, "\n").trim().length > 0);
    },
  });
  const providerReady = canSendToProvider(piiStatus);

  useEffect(() => {
    let active = true;
    bridge.localPiiStatus()
      .then((status) => {
        if (active) setPiiStatus(status);
      })
      .catch(() => {
        if (active) setPiiStatus(null);
      });
    return () => {
      active = false;
    };
  }, [bridge]);

  useEffect(() => {
    editor?.setEditable(!busy);
  }, [busy, editor]);

  async function pseudonymizeSelection() {
    if (!editor || busy) return;
    const { from, to, empty } = editor.state.selection;
    if (empty) return;
    const value = editor.state.doc.textBetween(from, to, "\n");
    if (!value.trim()) return;
    setBusy(true);
    setError("");
    try {
      const tokenized = await bridge.pseudonymizeSelection({ category: manualCategory, value });
      const reference = tokenized.references[0];
      const inserted = editor.commands.insertContentAt({ from, to }, {
        type: "text",
        text: tokenized.text,
        marks: [{
          type: "haciendaSensitive",
          attrs: {
            tokenId: reference.token,
            tokenClass: reference.class,
            mode: "pseudonymize",
            source: "manual",
            disclosure: "viewer-only",
          },
        }],
      });
      if (!inserted || !reference) throw new Error("Local pseudonymization did not produce an insertable token");
      setHasSelection(false);
    } catch {
      setError("Pseudonymisation locale indisponible. La sélection n’a pas été ajoutée à l’historique.");
    } finally {
      setBusy(false);
    }
  }

  async function openLocalViewer(chunks) {
    if (openingViewer) return;
    const payload = buildLocalViewerPayload(chunks);
    if (!payload.references.length) return;
    setOpeningViewer(true);
    setError("");
    try {
      await bridge.openLocalViewer(payload.tokenized, payload.references);
    } catch {
      setError("Le viewer local autorisé n’a pas pu être ouvert. Aucun contenu original n’a été affiché ici.");
    } finally {
      setOpeningViewer(false);
    }
  }

  async function submit(event) {
    event.preventDefault();
    if (!editor || busy) return;
    if (!providerReady) {
      setError("Envoi désactivé : le runtime ne confirme pas un détecteur local complet.");
      return;
    }
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

        <p className={`detector-status ${providerReady ? "ready" : "blocked"}`} role="status" aria-live="polite">
          Détecteur local : {piiStatus?.detector === "local_model" ? "modèle local prêt" : piiStatus?.detector === "regex_only" ? "regex uniquement" : "indisponible"}
          {providerReady ? " · envoi autorisé" : " · envoi bloqué"}
        </p>

        <TokenizedHistory turns={turns} onOpenViewer={openLocalViewer} openingViewer={openingViewer} />

        <form className="composer" onSubmit={submit}>
          <label className="composer-label" htmlFor="message-composer">Votre message</label>
          <div className="editor-frame">
            <EditorContent editor={editor} />
          </div>
          <ManualPseudonymizationPanel
            category={manualCategory}
            onCategoryChange={setManualCategory}
            disabled={!editor || !hasSelection || busy}
            onPseudonymize={() => void pseudonymizeSelection()}
          />
          <div className="composer-footer">
            <p>Le message est envoyé au bridge Tauri pour traitement local avant transport.</p>
            <button type="submit" disabled={!editor || busy || !hasContent || !providerReady}>
              {busy ? "Traitement local…" : "Envoyer"}
            </button>
          </div>
        </form>
        {error && <p className="error-banner" role="alert">{error}</p>}

        <OriginalViewer />
      </section>
      <footer className="footer-note">Les conversations et les messages ne sont pas enregistrés sur cet appareil.</footer>
    </main>
  );
}

const root = createRoot(document.getElementById("app"));
const isLocalViewer = new URLSearchParams(window.location.search).get("viewer") === "local";
root.render(isLocalViewer ? <LocalViewer /> : <App />);
