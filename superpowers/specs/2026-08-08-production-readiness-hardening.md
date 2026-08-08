# Durcissement de la préparation production — API, déploiement, audit Postgres, observabilité

**Date :** 2026-08-08
**Statut :** Proposé
**Piste :** Durcissement opérationnel (hors découpage P/S/E du programme — ne livre aucune
capacité nouvelle, ferme des écarts entre ce qui est documenté/déployé et ce que le code fait
réellement)
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` (complémentaire, ne s'y
substitue pas)
**Dépend de :** aucune spec — chaque écart ci-dessous est indépendant des autres
**Utilisée par :** quiconque déploie `hacienda serve` en dehors d'un poste de développement

---

## 1. Problème

Une revue de préparation production de l'API et des SDK a trouvé le cœur (auth par capacités,
chaîne d'audit hachée, garde anti-SSRF sur l'entrée document) solide, mais quatre écarts
concrets qui empêchent de qualifier le système de prêt pour la production. Aucun n'est une
lacune de conception — chacun est un endroit où le code, la configuration livrée, ou la CI ont
divergé silencieusement, sans qu'aucun test ne le retienne.

## 2. Écart 1 — `hacienda serve` casse le mode pseudonymize

`run_serve` (`hacienda-cli/src/commands.rs:849-890`) construit la façade avec
`HaciendaFacade::new(config)` directement, alors qu'un assistant `build_facade` existe déjà
(`commands.rs:34-44`) et câble conditionnellement un `EnvKeyResolver` quand
`config.pii.redaction.mode == Pseudonymize` — exactement ce que `run_extract`/`run_scan`
utilisent déjà. `CHANGELOG.md` (~ligne 169) documente cet écart comme connu et non corrigé
depuis le correctif CLI équivalent.

**Conséquence.** Un opérateur qui configure `[pii.redaction] mode = "pseudonymize"` et lance
`hacienda serve` obtient un serveur qui démarre normalement, puis échoue à la première requête
de rédaction ou de révélation en mode pseudonymize — l'échec n'est visible qu'à l'usage, pas au
démarrage.

**Décision D-1 — corriger par réutilisation, pas par duplication.** `build_facade` existe déjà,
est déjà testé indirectement via `run_extract`/`run_scan`, et encode déjà la bonne règle
(conditionner sur le mode, jamais construire le résolveur sans condition — un hôte sans
`HACIENDA_PSEUDONYM_ACTIVE_KEY` échouerait sinon sur tout mode, pas seulement pseudonymize).
`run_serve` doit l'appeler, point final.

## 3. Écart 2 — le chemin de déploiement Docker ne fonctionne pas tel que commité

Trois divergences indépendantes, toutes dans `docker/Dockerfile` et son entourage :

1. `CMD ["serve", "http", "--config", "/app/config/production.toml"]` — `http` n'est un
   sous-argument d'aucune commande : `ServeArgs` (`hacienda-cli/src/cli.rs:113-125`) n'a qu'un
   champ `--bind`. Le parsing clap échoue au démarrage du conteneur.
2. `HEALTHCHECK ... CMD ["/app/hacienda", "health-check"]` — aucune sous-commande
   `health-check` n'existe dans `Command` (`cli.rs:38-55` : `Extract`, `Scan`, `Config`,
   `Serve`, `Pii`, `Audit`). Le health check échoue systématiquement.
3. `COPY config/production.toml /app/config/production.toml` — ce fichier n'existe nulle part
   dans le dépôt. Le **build** de l'image échoue, avant même d'atteindre les deux problèmes
   précédents.

Le filet de sécurité censé attraper ceci ne peut pas le faire : `.github/workflows/ci-docker.yaml`
exécute `docker buildx build --check -f "$f" . 2>&1 || true` — un `--check` (lint, pas de build
réel) dont l'échec est explicitly avalé par `|| true`. Ce job ne peut jamais passer au rouge.

En complément, `docs/configuration.md` documente un schéma de configuration (`[server]`,
`[security]`, `[observability]`) qui ne correspond à aucune structure réelle — `HaciendaConfig`
(`hacienda-core/src/config.rs:28-47`), `AuthConfig` (`hacienda-core/src/auth/authn.rs:280-289`)
et `AuditConfig` (`hacienda-core/src/audit/mod.rs:70-76`) portent toutes
`#[serde(deny_unknown_fields)]` — un fichier écrit d'après cette documentation serait rejeté
au chargement.

**Décision D-2 — pas de `HEALTHCHECK` Docker.** L'image runtime est `distroless/cc-debian12`
(sans shell, sans curl) : il n'existe aucune commande à exécuter *depuis l'intérieur* du
conteneur pour sonder son propre port sans embarquer un binaire dédié rien que pour ça. `GET
/health` existe déjà et fonctionne (`hacienda-api/src/handlers/info.rs`, route publique) — la
sonde revient à l'orchestrateur (Kubernetes `httpGet`, ECS, etc.), pas à Docker.

## 4. Écart 3 — cinq tests du store d'audit Postgres restent ignorés en CI

`hacienda-core/src/store/postgres/audit.rs` : cinq tests `#[ignore]` échouent tous sur le
*même* couple segment/hash corrompu, quelle que soit l'assertion propre à chacun — signature
d'un problème d'isolation de test, pas d'un bug de logique de scellement. Confirmé : deux tests
(`should_detect_a_tampered_entry_in_a_sealed_segment`,
`should_report_a_missing_entry_as_a_count_mismatch`) mutent directement, par SQL brut, la base
Postgres *partagée* par tout le module de test, sans jamais restaurer l'état. L'ordre
alphabétique de `libtest` les exécute avant trois autres tests
(`should_serialise_concurrent_appends_without_breaking_the_chain`,
`should_survive_a_process_restart_against_the_same_database`,
`should_verify_after_a_rotation`) qui appellent `verify()`/`entries()` — lesquelles balaient
**toute** la base, y compris les segments corrompus par les deux premiers.

La logique de scellement elle-même (`hacienda-core/src/audit/segment.rs`,
`compute_seal_hash`/`check_seal_integrity`/`verify_seal_chain`) est saine : le backend fichier
(`hacienda-core/src/audit/store_file.rs`) exerce le même chemin `SegmentIntegrity` dans son
propre test équivalent et passe, parce que chaque test fichier reçoit son `TempDir` neuf.

**Décision D-3 — isoler les tests, ne pas changer la portée des requêtes en production.**
`entries()`/`verify()` interrogent délibérément toute la base — c'est le modèle mono-écrivain
documenté en commentaire. La correction porte sur le test (restaurer l'état après mutation, ou
isoler chaque test sur sa propre base/schéma si le harnais le permet déjà ailleurs), jamais sur
un `WHERE` ajouté pour satisfaire un test au prix de la sémantique de production.

## 5. Écart 4 — aucune télémétrie réelle, alors que `docker-compose.yml` en promet

Aucune couche `tower_http` (`TraceLayer`, `TimeoutLayer`, `CorsLayer`) n'existe dans
`hacienda-api` — seuls `DefaultBodyLimit` et le middleware d'auth sont posés
(`hacienda-api/src/routes.rs:363-364`). `docker-compose.yml` démarre pourtant Prometheus,
Grafana et Alertmanager, et `monitoring/alerts.yml` référence des métriques
(`pii_slo_availability_ratio`, `pii_pipeline_duration_seconds`,
`pii_model_inference_duration_seconds`, jauges mémoire/disque) qu'aucun code n'émet nulle part.
`monitoring/prometheus.yml` cible `hacienda:9090` — un port sur lequel rien n'écoute. Ce même
écart est déjà noté dans `docs/architecture/2026-07-31-analyse-architecture-et-pistes-produit.md`
(registre de risques).

**Décision D-4 — instrumentation minimale réelle, pas la parité complète.** Ajouter
`TraceLayer`/`TimeoutLayer`, une route `GET /metrics` réelle (compteur de requêtes, histogramme
de latence par route/statut), et réduire `monitoring/alerts.yml` aux métriques qui existent
vraiment. La parité SLO complète (métriques spécifiques au pipeline PII) reste un chantier
séparé, marqué explicitement comme non fait plutôt que silencieusement absent.

## 6. Hors périmètre

Durabilité des jobs asynchrones (déjà documentée comme limitation connue, `hacienda-api/src/lib.rs`
— Phase 6), bindings FFI natifs (déjà documentés comme non commencés dans le README), activation
de la publication SDK (`publish-sdk.yaml` est correct mais bloqué sur une configuration
« trusted publishing » côté organisation PyPI/npm — action externe, pas un correctif de code).

## 7. Tests

| Test | Assertion |
| --- | --- |
| `run_serve` construit la façade via `build_facade` | Un `serve` en mode pseudonymize répond à `/v1/pii/redact` puis `/v1/pii/reveal` sans erreur |
| `docker build -f docker/Dockerfile .` | Le build réussit ; l'image démarre et répond 200 sur `/health` |
| `ci-docker.yaml` avec l'ancien Dockerfile | Échoue rouge (le garde-fou gate réellement) |
| Les 5 tests Postgres audit précédemment `#[ignore]` | Passent, isolément et dans la suite complète, à trois exécutions de suite |
| `GET /metrics` après quelques requêtes | Rend un corps au format d'exposition Prometheus avec au moins une série de comptage et une série de latence |

## 8. Critères de sortie

- `hacienda serve` en mode pseudonymize fonctionne de bout en bout sur HTTP.
- `docker build` et `docker run` fonctionnent tels que documentés dans le README.
- `ci-docker.yaml` échoue réellement quand le Dockerfile est cassé.
- Les cinq tests Postgres audit passent en CI, non-`#[ignore]`.
- `GET /metrics` existe et le `docker-compose.yml` fourni scrape avec succès une cible réelle.
