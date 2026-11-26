//! Suite de non-régression française pour la détection déterministe.
//!
//! Deux familles de cas, toutes tirées de documents français réels ou de
//! leurs pièges les plus fréquents :
//!
//! * **faux positifs** — un montant, un numéro de commande ou un SIRET ne
//!   doivent jamais être étiquetés SIREN, carte bancaire ou téléphone ;
//! * **détection** — NIR corse, SIRET sans libellé mais groupé, EUID, La
//!   Poste, TVA intracommunautaire, téléphone national et variations OCR.
//!
//! Les affirmations portent sur la catégorie *et* sur la confiance : `1.0`
//! n'est permis que lorsqu'une somme de contrôle a réellement été exécutée.

use hacienda_core::pii::{PiiCategory, PiiPipeline, PipelineConfig, RegexEngine};

fn categories(texte: &str) -> Vec<PiiCategory> {
    RegexEngine::new()
        .expect("les motifs intégrés doivent compiler")
        .find_all(texte)
        .into_iter()
        .map(|entite| entite.category)
        .collect()
}

fn confiance(texte: &str, categorie: PiiCategory) -> Option<f32> {
    RegexEngine::new()
        .expect("les motifs intégrés doivent compiler")
        .find_all(texte)
        .into_iter()
        .find(|entite| entite.category == categorie)
        .map(|entite| entite.confidence)
}

mod faux_positifs {
    use super::*;

    #[test]
    fn un_montant_en_millions_nest_pas_un_siren() {
        // « 732 829 320 » passe Luhn et se présente exactement comme des
        // millions en typographie française. Sans libellé, ce doit être un
        // montant, pas un numéro d'entreprise.
        assert!(!categories("montant : 732 829 320 EUR").contains(&PiiCategory::FrSiren));
        assert!(!categories("Total TTC 732 829 320,00 €").contains(&PiiCategory::FrSiren));
    }

    #[test]
    fn un_numero_de_commande_luhn_valide_nest_pas_un_siret() {
        // 14 chiffres, Luhn valide, mais aucun libellé et aucun regroupement
        // canonique 3-3-3-5 : une référence, pas un établissement.
        assert!(!categories("référence 73282932000074 enregistrée").contains(&PiiCategory::FrSiret));
        assert!(!categories("commande n° 94468163400019 suivie").contains(&PiiCategory::FrSiret));
    }

    #[test]
    fn un_siret_nest_pas_une_carte_bancaire() {
        // Le défaut historique : Luhn seul laissait passer chaque SIRET des
        // factures françaises comme CREDIT_CARD à confiance 1.0.
        for texte in [
            "SIRET 732 829 320 00074",
            "73282932000074",
            "944 681 634 00019",
        ] {
            assert!(
                !categories(texte).contains(&PiiCategory::CreditCard),
                "un SIRET ne doit pas être une carte : {texte:?}"
            );
            assert!(
                !categories(texte).contains(&PiiCategory::Ssn),
                "un SIRET ne doit pas être un SSN : {texte:?}"
            );
        }
    }

    #[test]
    fn une_suite_de_chiffres_sans_separateur_nest_pas_un_telephone() {
        // libphonenumber accepte « 0156359180 », mais une suite compacte est
        // trop ambiguë avec un compte ou une référence : politique délibérée.
        for texte in ["référence 0156359180 classée", "compte 2125550143 ouvert"] {
            assert!(
                !categories(texte).contains(&PiiCategory::PhoneNumber),
                "suite compacte acceptée à tort : {texte:?}"
            );
        }
    }

    #[test]
    fn un_siret_groupe_nest_pas_un_telephone() {
        // Sur le document complet, le matcher libphonenumber extrairait le
        // « 00074 » final. La validation sur l'embranchement entier le refuse.
        let trouvees = RegexEngine::new()
            .unwrap()
            .find_all("SIRET 732 829 320 00074 mentionné sur la facture");
        assert!(
            !trouvees
                .iter()
                .any(|e| e.category == PiiCategory::PhoneNumber),
            "leSIRET ne doit pas produire de téléphone : {trouvees:?}"
        );
    }

    #[test]
    fn une_tva_avec_une_mauvaise_cle_est_rejetee() {
        // Une clé erronée est une preuve plus forte qu'une clé non vérifiée :
        // la correspondance disparaît, elle ne passe pas à 0.8.
        for texte in [
            "TVA FR41303265045 incorrecte",
            "BE0776091952 sur la facture",
            "NL123456783B90 invalide",
            "LU12345614 erroné",
        ] {
            assert!(
                !categories(texte).contains(&PiiCategory::EuVat),
                "TVA invalide détectée : {texte:?}"
            );
        }
    }

    #[test]
    fn un_nir_avec_une_mauvaise_cle_est_rejete() {
        for texte in [
            "numéro 291059933807693 invalide",
            "carte Vitale 185017512345608 non reconnue",
        ] {
            assert!(
                !categories(texte).contains(&PiiCategory::FrNir),
                "NIR invalide détecté : {texte:?}"
            );
        }
    }

    #[test]
    fn un_iban_francais_nest_pas_une_tva() {
        // Le préfixe FR est commun aux deux ; la longueur et la frontière
        // de mot empêchent la TVA de capturer l'IBAN.
        let trouvees = RegexEngine::new()
            .unwrap()
            .find_all("IBAN FR7630006000011234567890189 à créditer");
        assert!(
            !trouvees.iter().any(|e| e.category == PiiCategory::EuVat),
            "l'IBAN ne doit pas devenir une TVA : {trouvees:?}"
        );
        assert!(trouvees.iter().any(|e| e.category == PiiCategory::Iban));
    }

    #[test]
    fn un_label_ne_qualifie_pas_un_montant_de_la_phrase_suivante() {
        let texte = "SIREN 732829320. Total : 944 681 634 EUR";
        let montant = texte.find("944 681 634").unwrap() as u32;
        let trouvees = RegexEngine::new().unwrap().find_all(texte);
        assert!(
            !trouvees.iter().any(|e| {
                e.start == montant
                    && (e.category == PiiCategory::FrSiren || e.category == PiiCategory::FrSiret)
            }),
            "le montant ne doit pas devenir un SIREN : {trouvees:?}"
        );
        // Le vrai SIREN, lui, reste détecté.
        assert!(trouvees.iter().any(|e| e.category == PiiCategory::FrSiren));
    }

    #[test]
    fn une_tva_a_cle_valide_mais_siren_invalide_est_rejetee() {
        // FR15000000001 satisfait la formule de clé, mais le SIREN
        // 000000001 échoue à Luhn : ce n'est pas une TVA vérifiée.
        assert!(!categories("TVA FR15000000001 sur la facture").contains(&PiiCategory::EuVat));
    }

    #[test]
    fn une_adresse_ip_nest_pas_un_telephone() {
        assert!(!categories("host 203.0.113.42 responded").contains(&PiiCategory::PhoneNumber));
    }
}

mod detection {
    use super::*;

    #[test]
    fn doit_trouver_un_nir_compact_espacé_et_pointé() {
        for texte in [
            "NIR 185017512345609 déclaré",
            "numéro 1 85 01 75 123 456 09 déclaré",
            "numéro 2.96.01.12.345.678.59 déclaré",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::FrNir),
                "NIR valide manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn doit_trouver_un_nir_corse() {
        // Le département 2A est remplacé par 19 avant le contrôle mod-97.
        assert!(categories("assuré corse 185032A03411208").contains(&PiiCategory::FrNir));
    }

    #[test]
    fn doit_trouver_un_siret_libelle_ou_canoniquement_groupe() {
        for texte in [
            "SIRET 732 829 320 00074 sur la facture",
            "N° SIRET : 73282932000074",
            "75008 PARIS 944 681 634 00019 66198",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::FrSiret),
                "SIRET valide manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn doit_trouver_un_siret_de_la_poste() {
        // La Poste : le SIREN passe Luhn, mais les établissements se
        // valident par une somme des chiffres divisible par 5.
        assert!(categories("SIRET 35600000009075").contains(&PiiCategory::FrSiret));
    }

    #[test]
    fn doit_accepter_les_codes_administratifs_insee() {
        // 20-99 : mois de naissance inconnu ; 99 : né à l'étranger.
        assert!(categories("NIR 291205933807654 déclaré").contains(&PiiCategory::FrNir));
        assert!(categories("NIR 185019912345640 déclaré").contains(&PiiCategory::FrNir));
    }

    #[test]
    fn doit_rejeter_un_nir_avec_un_chiffre_non_ascii() {
        assert!(!categories("NIR 18\u{0665}017512345609 déclaré").contains(&PiiCategory::FrNir));
    }

    #[test]
    fn doit_garder_le_siren_devant_un_code_postal() {
        // Les cinq chiffres qui suivent ne forment pas un SIRET avec ce SIREN.
        let trouvees = RegexEngine::new()
            .unwrap()
            .find_all("SIREN 732 829 320 75008 Paris");
        assert!(trouvees.iter().any(|e| e.category == PiiCategory::FrSiren));
    }

    #[test]
    fn doit_trouver_un_siren_via_rcs_et_euid() {
        for texte in [
            "SIREN 732829320 déclaré",
            "RCS Paris 732 829 320",
            "Européen - EUID FR7501.944681634",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::FrSiren),
                "SIREN valide manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn doit_trouver_les_tva_checksummees_et_leurs_variantes() {
        for texte in [
            "TVA FR40303265045 intracommunautaire",
            "fr 40303265045",
            "TVA\u{a0}FR40303265045",
            "BE0776091951",
            "NL123456782B90",
            "LU12345613",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::EuVat),
                "TVA valide manquée : {texte:?}"
            );
        }
    }

    #[test]
    fn doit_trouver_une_tva_sans_algorithme_public() {
        // L'Allemagne ne publie pas de clé de contrôle : la forme suffit,
        // mais elle ne doit jamais prétendre à 1.0.
        assert_eq!(
            confiance("TVA DE123456789 sur la facture", PiiCategory::EuVat),
            Some(0.8)
        );
    }

    #[test]
    fn doit_trouver_les_telephones_nationaux_internationaux_et_ocr() {
        for texte in [
            "Appelez le 01 56 35 91 80",
            "téléphonez au : 0156 35 9180",
            "Appel international : +33 6 12 34 56 78",
            "US : (212) 555-0143",
            "Standard : 212-555-0143",
            "BV : 020 7183 8750",
        ] {
            assert!(
                categories(texte).contains(&PiiCategory::PhoneNumber),
                "téléphone valide manqué : {texte:?}"
            );
        }
    }

    #[test]
    fn doit_trouver_les_cartes_avec_leur_issuer_prefix() {
        for texte in ["carte 4111 1111 1111 1111", "amex 3782-822463-10005"] {
            assert!(
                categories(texte).contains(&PiiCategory::CreditCard),
                "carte valide manquée : {texte:?}"
            );
        }
    }
}

mod confiance {
    use super::*;

    #[test]
    fn la_confiance_pleine_est_reservee_a_une_somme_de_controle() {
        assert_eq!(
            confiance("NIR 185017512345609", PiiCategory::FrNir),
            Some(1.0)
        );
        assert_eq!(
            confiance("SIRET 73282932000074", PiiCategory::FrSiret),
            Some(1.0)
        );
        assert_eq!(
            confiance("SIREN 732829320", PiiCategory::FrSiren),
            Some(1.0)
        );
        assert_eq!(
            confiance("TVA FR40303265045", PiiCategory::EuVat),
            Some(1.0)
        );
        assert_eq!(
            confiance("carte 4111111111111111", PiiCategory::CreditCard),
            Some(1.0)
        );
    }

    #[test]
    fn les_detecteurs_sans_somme_de_controle_restent_en_dessous() {
        assert_eq!(
            confiance("Appelez le 01 56 35 91 80", PiiCategory::PhoneNumber),
            Some(0.75)
        );
        assert_eq!(confiance("TVA DE123456789", PiiCategory::EuVat), Some(0.8));
        assert_eq!(confiance("SSN 123-45-6789", PiiCategory::Ssn), Some(0.85));
        assert_eq!(confiance("jane@example.com", PiiCategory::Email), Some(0.9));
    }

    #[test]
    fn un_mot_de_contexte_ne_fabrique_pas_une_certitude() {
        // « TVA » à côté d'un numéro allemand ne doit pas transformer 0.8
        // en 1.0 : aucun chiffre de contrôle n'a été vérifié.
        assert_eq!(
            confiance("TVA DE123456789 sur la facture", PiiCategory::EuVat),
            Some(0.8)
        );
        assert_eq!(
            confiance("téléphone : 01 56 35 91 80", PiiCategory::PhoneNumber),
            Some(0.75)
        );
    }

    #[tokio::test]
    async fn la_confusion_apres_pipeline_respecte_les_sommes_de_controle() {
        // `context::enhance` s'exécute dans `PiiPipeline::detect`, pas dans
        // `RegexEngine::find_all` : on vérifie donc la valeur fusionnée.
        let pipeline = PiiPipeline::with_detector(PipelineConfig::default(), None).unwrap();
        for (texte, categorie, attendu) in [
            ("TVA DE123456789 sur la facture", PiiCategory::EuVat, 0.8),
            ("téléphone : 01 56 35 91 80", PiiCategory::PhoneNumber, 0.75),
        ] {
            let resultat = pipeline.scan(texte).await.unwrap();
            let confiance = resultat
                .entities
                .iter()
                .find(|e| e.category == categorie)
                .map(|e| e.confidence);
            assert_eq!(confiance, Some(attendu), "après pipeline : {texte:?}");
        }
        // Un e-mail reste à 0.9 même avec un libellé « email » : la forme
        // seule ne justifie pas une confiance pleine sans somme de contrôle.
        let resultat = pipeline.scan("email jane@example.com").await.unwrap();
        let email = resultat
            .entities
            .iter()
            .find(|e| e.category == PiiCategory::Email)
            .unwrap();
        assert_eq!(email.confidence, 0.9);
    }
}

mod decalages_octets {
    use super::*;

    #[test]
    fn les_empreintes_sont_des_octets_memes_avec_des_accents() {
        let texte = "mon numéro de sécurité sociale est 1 85 01 75 123 456 09";
        let entites = RegexEngine::new().unwrap().find_all(texte);
        let nir = entites
            .iter()
            .find(|e| e.category == PiiCategory::FrNir)
            .expect("le NIR doit être détecté");
        // « numéro » et « sécurité » contiennent trois caractères à deux
        // octets : l'embranchement commence à l'octet 38, pas au caractère 38.
        assert_eq!(nir.start, 38);
        assert_eq!(
            &texte[nir.start as usize..nir.end as usize],
            "1 85 01 75 123 456 09"
        );
    }

    #[test]
    fn une_tva_avec_espace_insécable_garde_son_embranchement() {
        let texte = "TVA FR\u{a0}40303265045 intracommunautaire";
        let entites = RegexEngine::new().unwrap().find_all(texte);
        let tva = entites
            .iter()
            .find(|e| e.category == PiiCategory::EuVat)
            .expect("la TVA doit être détectée");
        assert_eq!(
            &texte[tva.start as usize..tva.end as usize],
            "FR\u{a0}40303265045"
        );
    }
}
