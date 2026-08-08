# Durcissement préparation production — Plan d'implémentation

> **Pour les agents :** SOUS-COMPÉTENCE REQUISE — utiliser `superpowers:subagent-driven-development`
> (recommandé) ou `superpowers:executing-plans` pour exécuter ce plan tâche par tâche. Les étapes
> utilisent la syntaxe case à cocher (`- [ ]`).

**Objectif :** fermer quatre écarts entre ce que le dépôt documente/déploie et ce que le code
fait réellement, chacun indépendant des autres, chacun sa propre PR.

**Spec :** `superpowers/specs/2026-08-08-production-readiness-hardening.md`.

**Séquencement :** quatre branches indépendantes depuis `main`, ouvertes en PR brouillon dans
l'ordre ci-dessous (aucune ne bloque une autre — fichiers disjoints).

---

## Tâche 1 — `hacienda serve` : mode pseudonymize (branche `fix/serve-pseudonymize-key-resolver`)

- [ ] **Étape 1.1** — Dans `hacienda-cli/src/commands.rs::run_serve`, remplacer
      `let facade = Arc::new(HaciendaFacade::new(config).context("building the facade")?);`
      par un appel à `build_facade(config)` (déjà défini lignes 34-44, déjà utilisé par
      `run_extract`/`run_scan`). Une ligne.
- [ ] **Étape 1.2** — Test de non-régression : configurer `pii.redaction.mode = Pseudonymize`,
      poser `HACIENDA_PSEUDONYM_ACTIVE_KEY`, vérifier que la construction de façade utilisée par
      `run_serve` réussit — avant le correctif, elle construirait une façade sans
      pseudonymiseur, qui échoue au premier usage. Réutiliser les aides de test déjà présentes
      dans `hacienda-api/src/handlers/*.rs` si un aller-retour HTTP complet est peu coûteux à
      monter ; sinon un test unitaire ciblé sur la construction de façade suffit.
- [ ] **Étape 1.3** — `CHANGELOG.md` : transformer la note « known, not yet addressed »
      (~ligne 169) en entrée `### Fixed`, même style explicatif (cause racine + correctif) que
      les entrées existantes.

## Tâche 2 — Déploiement Docker (branche `fix/docker-deploy-mismatch`)

- [ ] **Étape 2.1** — `docker/Dockerfile` : corriger `CMD` en
      `["serve", "--config", "/app/config/production.toml", "--bind", "0.0.0.0:8787"]`.
- [ ] **Étape 2.2** — `docker/Dockerfile` : supprimer l'instruction `HEALTHCHECK` (décision D-2
      de la spec) ; la remplacer par un commentaire pointant vers `GET /health` pour les sondes
      d'orchestrateur.
- [ ] **Étape 2.3** — Créer `config/production.toml`, schéma réel vérifié contre
      `HaciendaConfig`/`AuthConfig`/`AuditConfig` (`hacienda-core/src/config.rs:28-47`,
      `hacienda-core/src/auth/authn.rs:280-289`, `hacienda-core/src/audit/mod.rs:70-76`), avec
      `[auth] enabled = true` (obligatoire : `check_bind_policy`,
      `hacienda-cli/src/commands.rs:828-846`, refuse un bind non loopback sans auth activée) et
      un `[[auth.static_tokens]]` d'exemple documenté « à remplacer avant tout déploiement ».
- [ ] **Étape 2.4** — Réécrire `docs/configuration.md` pour refléter le schéma réel — sections
      actuelles (`[server]`, `[security]`, `[observability]`) inexistantes sur des structures
      `#[serde(deny_unknown_fields)]`.
- [ ] **Étape 2.5** — `.github/workflows/ci-docker.yaml` : remplacer
      `docker buildx build --check -f "$f" . 2>&1 || true` par un vrai build pour
      `docker/Dockerfile` au minimum (`--check` seul peut rester pour les variantes musl/FFI si
      un build complet y est impraticable en CI — le dire en commentaire si c'est le choix fait).
- [ ] **Étape 2.6** — Vérifier que le garde-fou corrigé échoue bien rouge si on lui repasse
      l'ancien Dockerfile (sinon le correctif du job ne gate rien).

## Tâche 3 — Isolation des tests d'audit Postgres (branche `fix/postgres-audit-test-isolation`)

- [ ] **Étape 3.1** — Lire `test_support::block_on_shared` et les modules de test Postgres
      voisins pour vérifier si un patron d'isolation par test existe déjà ailleurs dans le
      dépôt, à réutiliser plutôt qu'à réinventer.
- [ ] **Étape 3.2** — Dans `hacienda-core/src/store/postgres/audit.rs`, corriger
      `should_detect_a_tampered_entry_in_a_sealed_segment` et
      `should_report_a_missing_entry_as_a_count_mismatch` pour qu'ils n'affectent plus l'état vu
      par les tests suivants — restaurer les lignes mutées en fin de test, ou isoler
      complètement (base/schéma dédié) si le patron de l'étape 3.1 le permet à moindre coût. Ne
      pas changer la portée des requêtes de production (`entries()`/`verify()` restent
      volontairement globales à la base — décision D-3 de la spec).
- [ ] **Étape 3.3** — Retirer `#[ignore]` des cinq tests
      (`should_detect_a_tampered_entry_in_a_sealed_segment`,
      `should_report_a_missing_entry_as_a_count_mismatch`,
      `should_serialise_concurrent_appends_without_breaking_the_chain`,
      `should_survive_a_process_restart_against_the_same_database`,
      `should_verify_after_a_rotation`), les faire tourner 3 fois de suite contre un Postgres
      réel, isolément et dans la suite complète, pour écarter toute fragilité résiduelle liée à
      l'ordre.
- [ ] **Étape 3.4** — `.github/workflows/ci-postgres.yaml` : retirer les `--skip` du job
      `postgres-store-tests`, remplacer le commentaire explicatif par une note historique courte
      (patron déjà établi dans `CHANGELOG.md`).
- [ ] **Étape 3.5** — `CHANGELOG.md` : entrée documentant la cause racine (pollution
      inter-tests, pas un bug de chaîne de scellement) et le correctif.

## Tâche 4 — Observabilité minimale réelle (branche `feat/observability-metrics`)

- [ ] **Étape 4.1** — Ajouter `tower-http` (features `trace`, `timeout`) comme dépendance
      **directe** de `hacienda-api/Cargo.toml` (présente aujourd'hui seulement en transitif dans
      `Cargo.lock`).
- [ ] **Étape 4.2** — Dans `hacienda-api/src/routes.rs::build_router`, poser
      `TraceLayer::new_for_http()` et un `TimeoutLayer` raisonnable, à côté des `.layer(...)`
      existants (`DefaultBodyLimit`, `auth_middleware`, lignes 363-364). Ordre : trace/timeout
      doivent envelopper l'auth, pas l'inverse, pour tracer et borner aussi un échec d'auth lent.
- [ ] **Étape 4.3** — Ajouter une dépendance minimale de métriques (`metrics` +
      `metrics-exporter-prometheus`, ou équivalent — vérifier qu'aucun choix existant du
      workspace ne convient déjà mieux avant d'ajouter une nouvelle dépendance) et instrumenter
      compteur de requêtes + histogramme de latence, étiquetés par route et code de statut.
- [ ] **Étape 4.4** — Ajouter `GET /metrics` à `ROUTE_TABLE` (`hacienda-api/src/routes.rs`) en
      `Access::Public` — même raisonnement que `/health` : le corps ne porte aucun contenu
      document.
- [ ] **Étape 4.5** — Réduire `monitoring/prometheus.yml` et `monitoring/alerts.yml` aux
      métriques réellement émises ; déplacer les alertes SLO PII retirées
      (`pii_slo_availability_ratio`, `pii_pipeline_duration_seconds`,
      `pii_model_inference_duration_seconds`, jauges mémoire/disque) dans un bloc clairement
      marqué « non instrumenté — backlog » plutôt que de les supprimer silencieusement.
- [ ] **Étape 4.6** — Ajuster `docker-compose.yml` si le port de scrape doit changer pour
      correspondre à l'endroit réel où `/metrics` est servi.

---

## Critères de sortie

- [ ] `hacienda serve` en mode pseudonymize répond correctement sur `/v1/pii/redact` et
      `/v1/pii/reveal`.
- [ ] `docker build -f docker/Dockerfile .` réussit ; le conteneur démarre et répond 200 sur
      `/health`.
- [ ] `ci-docker.yaml` échoue rouge contre un Dockerfile cassé (vérifié, pas supposé).
- [ ] Les cinq tests Postgres audit passent, non-`#[ignore]`, stables sur plusieurs exécutions.
- [ ] `GET /metrics` rend un corps Prometheus valide ; `docker-compose up` fait passer la cible
      `hacienda` à `UP` dans Prometheus.
- [ ] `cargo clippy --all-targets --all-features -D warnings` et `cargo fmt --check` propres sur
      chaque branche.

## Hors périmètre

Parité SLO complète des métriques PII, durabilité des jobs asynchrones, bindings FFI natifs,
activation de la publication SDK (bloquée sur une configuration externe côté PyPI/npm) — voir
§6 de la spec.
