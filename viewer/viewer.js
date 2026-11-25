// Codex local viewer — Hacienda gateway.
//
// Sécurité / contrat :
//  - Servi en boucle locale uniquement (127.0.0.1 / localhost / ::1), par le même
//    serveur que le gateway : aucune requête cross-origin, aucun CORS à autoriser.
//  - Aucune requête externe : le seul flux est un EventSource GET relatif vers
//    /viewer/events, authentifié par un jeton de session éphémère lié à un cookie
//    HttpOnly. Le secret local du gateway n'entre jamais dans le navigateur.
//  - Les données de rendu restent en mémoire (carte de réhydratation + tampon de rendu),
//    jamais écrites sur le disque ni dans le stockage persistant du navigateur.
//  - L'audit des révélations ne contient aucun contenu en clair : action,
//    horodatage et référence de jeton (pseudonyme) uniquement.
(() => {
  'use strict';

  // Configuration injectée par le serveur dans la page (même origine).
  const config = (() => {
    try {
      return JSON.parse(document.body.dataset.viewerConfig);
    } catch (e) {
      return null;
    }
  })();

  const els = {
    connectBtn: document.getElementById('connectBtn'),
    disconnectBtn: document.getElementById('disconnectBtn'),
    clearBtn: document.getElementById('clearBtn'),
    status: document.getElementById('status'),
    output: document.getElementById('output'),
    audit: document.getElementById('audit'),
  };

  // Carte de réhydratation en mémoire : jeton pseudonymisé -> valeur originale.
  // Jamais persistée ; vidée à la fermeture de l'onglet.
  const rehydrationMap = new Map();
  // Tampon de rendu en mémoire : liste des items rendus (texte tokenisé + HTML).
  const renderBuffer = [];
  // Journal d'audit des révélations, contenu libre (sans contenu en clair).
  const auditLog = [];

  let eventSource = null;

  // Le serveur n'injecte que des URL relatives : une origine externe est refusée.
  function isLocalEventsUrl(url) {
    return typeof url === 'string' && url.startsWith('/') && !url.startsWith('//');
  }

  // ─── Échappement HTML ────────────────────────────────────────────────────

  function escapeHtml(text) {
    return String(text)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  // ─── Markdown minimal (aucune dépendance externe) ────────────────────────

  // Tokens Hacienda : [CATEGORIE:clef:chiffrement]
  const TOKEN_RE = /\[([A-Z][A-Z0-9_]*):([A-Za-z0-9_-]+):([A-Za-z0-9_-]*)\]/g;

  function renderInline(text) {
    let html = escapeHtml(text);
    // Code en ligne d'abord.
    html = html.replace(/`([^`]+)`/g, '<code>$1</code>');
    // Gras puis italique.
    html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    html = html.replace(/\*([^*]+)\*/g, '<em>$1</em>');
    // Liens.
    html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" rel="nofollow noreferrer" target="_blank">$1</a>');
    // Tokens : entité locale en gris si réhydratable en mémoire, sinon jeton seul.
    html = html.replace(TOKEN_RE, (match, category, keyId, cipher) => {
      const token = `[${category}:${keyId}:${cipher}]`;
      const original = rehydrationMap.get(token);
      if (original !== undefined) {
        return `<span class="entity" title="jeton: ${escapeHtml(token)}">${escapeHtml(original)}</span>`;
      }
      return `<span class="token" title="jeton ${escapeHtml(category)}">${escapeHtml(token)}</span>`;
    });
    return html;
  }

  function markdownToHtml(text) {
    const paragraphs = String(text).split(/\n\n+/);
    return paragraphs
      .map((p) => {
        const trimmed = p.trim();
        if (!trimmed) return '';
        const heading = trimmed.match(/^(#{1,6})\s+(.+)$/s);
        if (heading) {
          const level = heading[1].length;
          return `<h${level}>${renderInline(heading[2].trim())}</h${level}>`;
        }
        return `<p>${renderInline(trimmed)}</p>`;
      })
      .join('');
  }

  // ─── Rendu ───────────────────────────────────────────────────────────────

  function appendRendered(text) {
    const html = markdownToHtml(text);
    renderBuffer.push({ text, html, at: new Date().toISOString() });
    // Le tampon reste en mémoire ; on ne le persiste pas.
    redraw();
  }

  function redraw() {
    els.output.innerHTML = renderBuffer.map((item) => item.html).join('\n') || '<p class="hint">Aucun événement reçu pour l’instant.</p>';
  }

  function clearRender() {
    renderBuffer.length = 0;
    redraw();
  }

  // ─── Audit des révélations (sans contenu) ────────────────────────────────

  async function auditReveal(token) {
    const entry = { action: 'reveal', at: new Date().toISOString(), ref: token };
    auditLog.push(entry);
    const div = document.createElement('div');
    div.className = 'entry';
    div.innerHTML = `<span class="action">${escapeHtml(entry.action)}</span> <span class="at">${escapeHtml(entry.at)}</span> <span class="ref">${escapeHtml(entry.ref)}</span>`;
    els.audit.prepend(div);
    // Seule la référence du jeton est envoyée : jamais la valeur réelle, jamais
    // le jeton de session (qui reste dans le cookie HttpOnly et l'en-tête).
    if (!config || !isLocalEventsUrl(config.revealUrl)) return;
    try {
      await fetch(config.revealUrl, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'x-hacienda-viewer-session': config.sessionToken,
        },
        body: JSON.stringify({ token_ref: token }),
      });
    } catch (e) {
      // L'audit local reste disponible même si le serveur est momentanément absent.
    }
  }

  // ─── Carte de réhydratation ──────────────────────────────────────────────

  function setRehydrationEntry(token, original) {
    if (!/^\[[A-Z][A-Z0-9_]*:[A-Za-z0-9_-]+:[A-Za-z0-9_-]*\]$/.test(token)) return false;
    rehydrationMap.set(token, original);
    redraw();
    return true;
  }

  // Expose pour tests / console (lecture seule de la taille, pas du contenu).
  window.__codexViewer__ = {
    mapSize: () => rehydrationMap.size,
    bufferSize: () => renderBuffer.length,
    auditSize: () => auditLog.length,
    setRehydrationEntry,
  };

  // ─── EventSource (SSE) — boucle locale uniquement ────────────────────────

  function setStatus(connected) {
    if (connected) {
      els.status.textContent = 'Connecté (boucle locale)';
      els.status.className = 'status status-connected';
      els.connectBtn.disabled = true;
      els.disconnectBtn.disabled = false;
    } else {
      els.status.textContent = 'Déconnecté';
      els.status.className = 'status status-disconnected';
      els.connectBtn.disabled = false;
      els.disconnectBtn.disabled = true;
    }
  }

  function connect() {
    const eventsUrl = config && config.eventsUrl;
    if (!isLocalEventsUrl(eventsUrl)) {
      setStatus(false);
      return;
    }
    disconnect();
    try {
      eventSource = new EventSource(eventsUrl);
    } catch (e) {
      setStatus(false);
      return;
    }
    eventSource.onopen = () => setStatus(true);
    eventSource.onerror = () => {
      setStatus(false);
      if (eventSource) { eventSource.close(); eventSource = null; }
    };
    eventSource.onmessage = (ev) => {
      let data;
      try { data = JSON.parse(ev.data); } catch (e) { return; }
      if (!data || typeof data !== 'object') return;
      if (data.type === 'mapping' && typeof data.token === 'string' && typeof data.original === 'string') {
        setRehydrationEntry(data.token, data.original);
      } else if (data.type === 'text' && typeof data.text === 'string') {
        appendRendered(data.text);
      }
      // `end` est un marqueur de corrélation : aucun texte supplémentaire.
    };
  }

  function disconnect() {
    if (eventSource) {
      eventSource.close();
      eventSource = null;
    }
    setStatus(false);
  }

  // ─── Liaison DOM ─────────────────────────────────────────────────────────

  els.connectBtn.addEventListener('click', () => connect());
  els.disconnectBtn.addEventListener('click', disconnect);
  els.clearBtn.addEventListener('click', clearRender);

  // Révélation d'une entité : audit sans contenu, affichage facultatif de l'original.
  els.output.addEventListener('click', (ev) => {
    const entity = ev.target.closest('.entity');
    if (!entity) return;
    const token = entity.getAttribute('title') && entity.getAttribute('title').replace(/^jeton: /, '');
    const original = rehydrationMap.get(token || '');
    if (token) {
      auditReveal(token); // contenu libre uniquement : action, heure, référence de jeton
    }
    if (original !== undefined) {
      entity.classList.add('revealed');
      entity.title = `révélé localement (audité, sans contenu journalisé)`;
    }
  });

  // Déconnexion propre à la fermeture de l'onglet : rien n'est persistant.
  window.addEventListener('beforeunload', disconnect);

  // État initial : connexion automatique au canal de la page (même origine).
  setStatus(false);
  redraw();
  connect();
})();
