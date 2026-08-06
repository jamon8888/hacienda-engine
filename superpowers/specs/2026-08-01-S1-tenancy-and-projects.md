# S1 — Tenants, projets et autorisation

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** S (socle) · **Vague :** 1 · **Chemin critique :** oui
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §4
**Bloque :** P3, S2, S3, E2, E3, E4, E5, V1, V2

---

## 1. Problème

L'isolation actuelle se limite à un champ : `Job.owner` porte l'identifiant de principal, et
`hacienda-api/src/handlers/jobs.rs` renvoie 404 — pas 403 — quand un principal demande le job
d'un autre. C'est une bonne défense IDOR, et c'est tout ce qui existe.

Trois conséquences, dont une grave :

1. **L'espace de tokens de pseudonymisation est global au processus.** `HACIENDA_PSEUDONYM_ACTIVE_KEY`
   nomme une clé unique. Deux clients dont les corpus contiennent la même valeur — une adresse,
   un IBAN — obtiennent **le même token**. Un client qui observe ses propres tokens peut donc
   tester l'appartenance d'une valeur au corpus d'un autre. C'est une fuite par corrélation, et
   elle disqualifie l'offre multi-client.
2. Une chaîne d'audit par nœud, pas par tenant : un export destiné à un client contient les
   entrées de tous.
3. Aucun quota, aucune limite, aucune configuration par client.

Le `NodeId` des segments (`audit/segment.rs:43`) est déjà documenté comme servant « les
déploiements multi-tenant » : l'intention existe, l'implémentation non.

**Cette spec doit être livrée avant toute mise en production multi-client.** Rétro-ajouter un
tenant après coup impose de migrer les chaînes d'audit *et* de re-dériver chaque token jamais
émis — les tokens étant déterministes, changer leur espace de clés change leur valeur, donc
casse toute co-référence déjà publiée.

## 2. Objectifs / Non-objectifs

**Objectifs**

- Un `TenantCtx` porté par `Caller` et accepté en paramètre de scope par tous les traits de store.
- Un espace de clés de pseudonymisation **par tenant**.
- Des segments d'audit nommés `(tenant, node)`.
- Des quotas et limites par tenant, applicables au transport.

**Non-objectifs**

| Différé | Raison |
| --- | --- |
| SSO / SAML / OIDC | Offre entreprise, hors socle |
| Facturation | → E5, qui dérive ses compteurs de l'audit |
| Console d'administration | Interface, pas socle |
| Fédération inter-tenants | Aucun cas d'usage identifié |

## 3. Modèle

```rust
/// Identité de cloisonnement. Immuable pour la durée d'une requête.
pub struct TenantCtx {
    pub tenant: TenantId,
    pub actor: ActorId,
    /// Sous-division optionnelle, pour la parité avec le `project` d'Enterprise.
    pub project: Option<ProjectId>,
}
```

**Décision D-S1-1 — le contexte est un paramètre, jamais un champ implicite.** Chaque méthode
de store le reçoit :

```rust
async fn entries(&self, ctx: &TenantCtx) -> Result<Vec<AuditEntry>, AuditError>;
```

plutôt que d'être capturé à la construction du store. Un store construit *pour* un tenant
invite à instancier un store par tenant, ce qui multiplie les pools de connexions et déplace
le cloisonnement vers la fabrique — où il s'oublie. Le paramètre rend l'omission
impossible à compiler.

**Décision D-S1-2 — `project` est optionnel et n'est pas une frontière de sécurité.** Il
organise ; il ne cloisonne pas. Seul `tenant` cloisonne. Un `project` traité comme frontière
créerait deux niveaux d'autorisation dont le second serait plus faible et invisible.

## 4. Espace de clés par tenant

`KeyResolver` gagne le tenant :

```rust
fn resolve(&self, ctx: &TenantCtx, id: &KeyId) -> Result<PseudonymKey, PseudonymError>;
fn active(&self, ctx: &TenantCtx) -> Result<KeyId, PseudonymError>;
```

**Décision D-S1-3 — pas de dérivation d'une clé maître par tenant.** Il serait tentant de
dériver `key_tenant = KDF(master, tenant_id)`. Refusé : la compromission de la maîtresse
compromettrait tous les tenants d'un coup, et la rotation par tenant deviendrait impossible
sans re-dériver les autres. Chaque tenant possède un matériel indépendant, résolu par
l'implémentation (env, Vault, KMS — → S2).

**Décision D-S1-4 — échec à la construction, jamais à l'usage.** Comme
`HaciendaFacade::with_key_resolver` aujourd'hui : un tenant dont la clé configurée est absente
fait échouer l'admission de ce tenant, pas la première requête qui le touche. Découvrir qu'un
corpus est irréversible au moment d'un droit d'accès est le mode d'échec à interdire.

## 5. Audit segmenté par tenant

`NodeId` devient `(TenantId, NodeId)`. Les trois vérifications existantes — entrées dans un
segment, chaîne de seals, tip enregistré contre le segment scellé — restent inchangées ; elles
s'appliquent par tenant.

**Décision D-S1-5 — pas de chaîne globale au-dessus des chaînes de tenants.** Elle sérialiserait
tous les écrivains et n'apporterait aucune propriété qu'un auditeur d'un tenant puisse
exploiter : il ne peut pas voir les entrées des autres, donc ne peut rien vérifier au-dessus.

## 6. Autorisation

`Capability` est inchangée. Ce qui change est le sujet auquel elle s'applique : une capacité
est accordée **pour un tenant**, jamais globalement.

**Décision D-S1-6 — l'absence se signale par 404.** Une ressource d'un autre tenant est
*inexistante* du point de vue de l'appelant. Un 403 confirmerait son existence. C'est la règle
déjà appliquée par `handlers/jobs.rs`, généralisée.

## 7. Quotas

Par tenant : documents/mois, octets ingérés, requêtes/minute, taille de corpus vectoriel.
Appliqués au transport, avant décodage du corps — un quota vérifié après décodage a déjà payé
le coût qu'il devait éviter.

Dépassement : `429` avec un en-tête `Retry-After` et un corps décrivant la limite atteinte,
jamais un `500`.

## 8. Migration

Le déploiement mono-client existant devient le tenant `default`. Une migration relit les
segments d'audit existants et les réécrit sous `(default, node)`. Les tokens déjà émis
restent valides : ils portent leur identifiant de clé, et la clé du tenant `default` est celle
qui existait.

**Ce chemin ne fonctionne que pour un seul tenant préexistant.** Il n'y en a qu'un.

## 9. Tests

| Test | Assertion |
| --- | --- |
| `two_tenants_same_value_get_different_tokens` | Le cœur de la spec. La même chaîne sous deux tenants produit deux tokens distincts. |
| `retired_key_of_one_tenant_reveals_nothing_of_another` | La révélation croisée échoue. |
| `cross_tenant_read_is_404_not_403` | Sur chaque endpoint scopé. |
| `audit_export_contains_only_the_calling_tenant` | Aucune fuite d'entrée. |
| `missing_tenant_key_fails_admission_not_first_request` | D-S1-4. |
| `verify_passes_per_tenant_after_replica_failover` | Avec S2. |
| `quota_exceeded_returns_429_before_body_decode` | Le corps n'est pas lu. |

## 10. Critères de sortie

- I3 du programme vérifié sur **chaque** store.
- Deux tenants portant la même valeur PII produisent des tokens différents — vérifié par test,
  pas par revue.
- Aucun matériel de clé ne transite par la configuration ni n'apparaît dans `config show`.
- La migration du tenant `default` est rejouable et idempotente.
