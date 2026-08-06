# P5 — Artefacts de conformité

**Date :** 2026-08-01
**Statut :** Proposé
**Piste :** P (couche de preuve) · **Vague :** 0
**Programme :** `2026-08-01-hacienda-platform-parity-program.md` §5.5
**Dépend de :** rien · **Enrichie par :** V4 (provenance verticale dans la model card)

---

## 1. Problème

`ComplianceGenerator` produit `ComplianceReport { dpia, model_card, dora, checklist,
generated_at }`. Les générateurs existent, sont testés, et sont pilotés par `ComplianceConfig
{ model_name, enabled_reports }`.

Deux défauts, dont le second est le vrai sujet :

1. **Aucune route HTTP.** Comme P2 et P4 : du métier écrit et invisible.
2. **Les artefacts sont trop peu liés à la réalité du déploiement.** `ComplianceConfig` ne porte
   que le nom du modèle et la liste des rapports. Une DPIA qui ne cite ni le profil de rédaction
   appliqué, ni le modèle et l'adaptateur réellement actifs, ni la politique de rétention, ni la
   chaîne d'audit qui l'atteste, est un gabarit — et un régulateur le lit comme tel.

Xberg Enterprise n'a rien ici : zéro occurrence de `gdpr` dans sa spec (analyse §9.12.3). C'est
le seul module du programme dont l'acheteur est le DPO et non la DSI.

## 2. Surface

| Route | Capacité | Objet |
| --- | --- | --- |
| `GET /v1/compliance/report` | `audit:read` | Rapport unifié |
| `GET /v1/compliance/dpia` | `audit:read` | RGPD Art. 35 |
| `GET /v1/compliance/model-card` | `audit:read` | AI Act Art. 11 |
| `GET /v1/compliance/dora` | `audit:read` | DORA Art. 11 |
| `GET /v1/compliance/checklist` | `audit:read` | Liste de contrôle |

Négociation de contenu : `application/json`, `text/markdown`, `application/pdf`.

**Décision D-P5-1 — capacité `audit:read`, pas une nouvelle.** Un artefact de conformité décrit
le même système que la chaîne d'audit et cite ses empreintes. Qui peut lire l'audit peut lire la
DPIA ; l'inverse serait incohérent.

## 3. Le cœur de la spec : ancrer les artefacts dans le déploiement réel

`ComplianceConfig` s'étend pour que chaque artefact généré cite ce qui a réellement produit les
données :

```text
profil de rédaction actif      (RedactionConfig : mode, profil PCI/HIPAA/GDPR, seuils)
modèle et adaptateur actifs    (ModelConfig + registre V2, avec empreintes)
config_hash effectif           (celui que la chaîne d'audit inscrit déjà)
tip de chaîne au moment de la génération
politique de rétention et de rotation de clés
verticales déployées           (→ V1, V4)
```

**Décision D-P5-2 — un artefact cite le `config_hash` et le `tip` de chaîne.** C'est ce qui le
rend vérifiable : un auditeur peut demander la chaîne, la vérifier via P2, et constater qu'elle
correspond à l'empreinte que la DPIA revendique. Un document non ancré n'est qu'une assertion.

**Décision D-P5-3 — changer la configuration change le document.** Test obligatoire : deux
générations sous deux profils de rédaction produisent deux DPIA différentes. Si elles sont
identiques, l'artefact est un gabarit et la spec a échoué.

**Décision D-P5-4 — aucun champ n'est inventé.** Un générateur qui ne dispose pas d'une
information la marque `non renseigné` avec la raison, plutôt que d'émettre une valeur
plausible. Un régulateur qui découvre un champ inventé cesse de croire au document entier — et
il a raison.

## 4. Ce que la model card doit dire, et que V4 fournira

L'AI Act Art. 11 exige que la documentation technique décrive **le modèle qui a réellement
produit la sortie**. Avec un routage d'adaptateurs par requête (V2), cela ne peut venir que de
l'entrée d'audit.

Tant que V4 n'est pas livré, la model card énumère le modèle de base et déclare explicitement
que le routage par verticale n'est pas encore tracé. **Déclarer une limite est conforme ;
l'omettre ne l'est pas.**

## 5. Rendu PDF

**Décision D-P5-5 — le PDF est un rendu du Markdown, pas un second générateur.** Deux chemins de
génération divergeraient, et le document remis au régulateur serait celui qui a divergé. Un
générateur, deux rendus.

## 6. Tests

| Test | Assertion |
| --- | --- |
| `changing_redaction_profile_changes_the_dpia` | D-P5-3. Le test qui distingue artefact et gabarit. |
| `dpia_cites_the_effective_config_hash` | Et il correspond à celui de la chaîne. |
| `dpia_cites_a_chain_tip_that_verifies` | Bout en bout avec P2. |
| `unavailable_fields_are_marked_not_invented` | D-P5-4. |
| `pdf_and_markdown_agree` | Comparaison du contenu textuel. |
| `model_card_declares_untracked_vertical_routing` | Tant que V4 n'est pas là. |
| `artefacts_are_scoped_to_the_calling_tenant` | Avec S1. |

## 7. Critères de sortie

- Une DPIA générée cite l'empreinte de configuration effective **et** un tip de chaîne qui
  vérifie via P2.
- Changer le profil de rédaction change le document.
- Aucun champ n'est renseigné par défaut plausible.
- PDF et Markdown proviennent du même générateur.
