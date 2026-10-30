//! The built-in legal-form walk: company names ending in a legal form.
//!
//! Most cases follow stella/anonymize's legal-form tests.

mod walks;

use elide_core::modality::text::TextData;
use elide_core::primitive::{Confidence, ConfidenceThreshold, LanguageClaim, LanguageTag};
use elide_core::recognition::{Scope, Subject};
use elide_pattern::walks::legal_form;

async fn found(text: &str, languages: &[&str]) -> Vec<(String, Confidence)> {
    walks::found(legal_form(), text, languages).await
}

async fn names(text: &str) -> Vec<String> {
    walks::names(legal_form(), text).await
}

async fn names_in(text: &str, languages: &[&str]) -> Vec<String> {
    found(text, languages)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[tokio::test]
async fn finds_names_across_jurisdictions() {
    for (text, name) in [
        ("Invoice from Müller Bau GmbH, Berlin", "Müller Bau GmbH"),
        ("Signed by Acme Holdings Ltd on Monday", "Acme Holdings Ltd"),
        ("Tesco PLC reported results", "Tesco PLC"),
        ("Bank of America Corp. said", "Bank of America Corp."),
        ("Acme, Inc. and its affiliates", "Acme, Inc."),
        ("Dupont Frères SARL à Lyon", "Dupont Frères SARL"),
        ("Società Rossi S.p.A. di Milano", "Società Rossi S.p.A."),
        (
            "Sold to Česká spořitelna, a.s. today.",
            "Česká spořitelna, a.s.",
        ),
        ("Barclays Bank PLC", "Barclays Bank PLC"),
        (
            "JPMorgan Chase Bank, N.A. as administrative agent",
            "JPMorgan Chase Bank, N.A.",
        ),
    ] {
        assert_eq!(names(text).await, [name], "in {text:?}");
    }
}

#[tokio::test]
async fn keeps_the_inner_words_of_a_firm_name() {
    for (text, name) in [
        ("Goldman Sachs & Co. LLC", "Goldman Sachs & Co. LLC"),
        ("J.P. Morgan Securities LLC", "J.P. Morgan Securities LLC"),
        ("J. P. Morgan Securities LLC", "J. P. Morgan Securities LLC"),
        ("Acme GmbH & Co. KG hat", "Acme GmbH & Co. KG"),
        (
            "Served on Arkansas Corporation Services, Inc. today",
            "Arkansas Corporation Services, Inc.",
        ),
        (
            "with a copy to:\nSkadden, Arps, Slate, Meagher & Flom LLP\n525 University Ave",
            "Skadden, Arps, Slate, Meagher & Flom LLP",
        ),
        (
            "Skadden, Arps, Slate, Meagher & Flom (UK) LLP",
            "Skadden, Arps, Slate, Meagher & Flom (UK) LLP",
        ),
        (
            "The Space In Between LLC operates here.",
            "The Space In Between LLC",
        ),
        (
            "Everything Is Better Between Friends LLC operates here.",
            "Everything Is Better Between Friends LLC",
        ),
        ("McDonald's Corp. said", "McDonald's Corp."),
        ("Joe's Crab Shack LLC opened", "Joe's Crab Shack LLC"),
    ] {
        assert_eq!(names(text).await, [name], "in {text:?}");
    }
}

#[tokio::test]
async fn a_name_ends_at_an_earlier_name() {
    assert_eq!(
        names("Acme GmbH and Beta GmbH signed").await,
        ["Acme GmbH", "Beta GmbH"]
    );
    assert_eq!(
        names("Advised by Banco de Sabadell S.A. and Bank of America Corp.").await,
        ["Banco de Sabadell S.A.", "Bank of America Corp."]
    );
    assert_eq!(
        names("Küstenwerke AG, Société générale SA, and Řeka SE filed jointly.").await,
        ["Küstenwerke AG", "Société générale SA", "Řeka SE"]
    );
}

#[tokio::test]
async fn a_party_connector_after_a_comma_or_a_role_ends_the_name() {
    assert_eq!(
        names(
            "represented by its Chief Executive Officer Priya Ramanathan, and Northwind \
             Capital Partners LLC, represented by Jonathan H. Whitaker."
        )
        .await,
        ["Northwind Capital Partners LLC"]
    );
    assert_eq!(
        names("between the Company and Barclays Bank PLC, (ii) the").await,
        ["Barclays Bank PLC"]
    );
    assert_eq!(
        names("Acme Widgets and Bar, Inc. signed.").await,
        ["Acme Widgets and Bar, Inc."]
    );
}

#[tokio::test]
async fn and_after_a_person_with_a_middle_initial_starts_a_new_party() {
    assert_eq!(
        names("Signed by John A. Smith and Acme LLC today.").await,
        ["Acme LLC"]
    );
    assert_eq!(
        names("John A. Smith and Sons Ltd was founded in 1901.").await,
        ["John A. Smith and Sons Ltd"]
    );
    assert_eq!(
        names("Procter and Gamble Ltd announced results.").await,
        ["Procter and Gamble Ltd"]
    );
}

#[tokio::test]
async fn english_prose_between_names_ends_them() {
    for (text, expected) in [
        (
            "Northwind Ventures LLC invested in Acme Holdings Ltd.",
            ["Northwind Ventures LLC", "Acme Holdings Ltd."],
        ),
        (
            "Northwind Ventures LLC invested heavily in Acme Holdings Ltd.",
            ["Northwind Ventures LLC", "Acme Holdings Ltd."],
        ),
        (
            "Northwind Ventures LLC led the Series B round of Acme Holdings Ltd.",
            ["Northwind Ventures LLC", "Acme Holdings Ltd."],
        ),
        (
            "Acme Holdings Ltd. sold its Prague subsidiary to Northwind Ventures LLC.",
            ["Acme Holdings Ltd.", "Northwind Ventures LLC"],
        ),
    ] {
        assert_eq!(names_in(text, &["en"]).await, expected, "in {text:?}");
    }
}

#[tokio::test]
async fn an_open_language_lets_a_name_hold_lowercase_words() {
    let name = "Základní škola a Mateřská škola Brno, s.r.o.";
    assert_eq!(names_in(&format!("Nájemce: {name}"), &["cs"]).await, [name]);
    // Without a Czech language, a lowercase word after the first capitalized
    // one ends the name.
    assert_eq!(names(&format!("Nájemce: {name}")).await, ["Brno, s.r.o."]);
}

#[tokio::test]
async fn a_detected_language_stands_in_for_an_asserted_one() {
    let text = "Nájemce: Základní škola a Mateřská škola Brno, s.r.o.";
    let detected = || {
        let mut subject = Subject::new(TextData::new(text.to_owned()));
        subject.detect_language(LanguageClaim::detected(
            "cs".parse().expect("language tag"),
            Confidence::clamped(0.9),
        ));
        subject
    };
    let names = |found: Vec<(String, Confidence)>| -> Vec<String> {
        found.into_iter().map(|(name, _)| name).collect()
    };

    assert_eq!(
        names(walks::found_in(legal_form(), detected(), &Scope::new()).await),
        ["Základní škola a Mateřská škola Brno, s.r.o."]
    );
    // An asserted language wins over a detected one.
    let english = Scope::new().with_language("en".parse::<LanguageTag>().expect("language tag"));
    assert_eq!(
        names(walks::found_in(legal_form(), detected(), &english).await),
        ["Brno, s.r.o."]
    );
}

#[tokio::test]
async fn the_lead_in_clause_is_not_part_of_the_name() {
    for text in [
        "This Agreement is entered into between Acme Inc.",
        "This Agreement, entered into between Acme Inc.",
    ] {
        assert_eq!(names(text).await, ["Acme Inc."], "in {text:?}");
    }
    assert_eq!(
        names("Investment Agreement, dated as of March 9, 2020, among Twitter, Inc.").await,
        ["Twitter, Inc."]
    );
    assert_eq!(
        names_in("Der Vertrag wird mit Müller Bau GmbH geschlossen.", &["de"]).await,
        ["Müller Bau GmbH"]
    );
}

#[tokio::test]
async fn an_amount_is_not_part_of_a_name() {
    for text in [
        "Northwind invested USD 45,000,000 in Acme Holdings Ltd.",
        "USD 45,000,000 Acme Holdings Ltd.",
        "Northwind invested EUR 45.000.000 in Acme Holdings Ltd.",
        "Paid 45,000,000 to Acme Holdings Ltd. at closing.",
    ] {
        assert_eq!(names(text).await, ["Acme Holdings Ltd."], "in {text:?}");
    }
}

#[tokio::test]
async fn a_name_does_not_cross_a_line_sentence_or_quote() {
    assert_eq!(names("Signed\nAcme GmbH").await, ["Acme GmbH"]);
    assert_eq!(
        names("We met in Berlin. Acme GmbH signed.").await,
        ["Acme GmbH"]
    );
    assert_eq!(names("Acme Inc. Beta LLC").await, ["Acme Inc.", "Beta LLC"]);
    assert_eq!(names("the “Acme GmbH” deal").await, ["Acme GmbH"]);
    assert_eq!(names("Buyer, Seller,\nAcme LLC").await, ["Acme LLC"]);
    assert_eq!(
        names("and\nSimpson Thacher & Bartlett LLP\n425 Lexington Avenue").await,
        ["Simpson Thacher & Bartlett LLP"]
    );
}

#[tokio::test]
async fn needs_a_capitalized_name_and_a_whole_marker() {
    for text in [
        "See 18 U.S.C. Section 1833(b) for civil immunity.",
        "THIRD PARTIES shall not benefit hereunder.",
        "NEED ADDRESS for notices.",
        "Os usuários do Minha AGÊNCIA aceitam estes termos.",
        "Bonus para o PLANO DE SAÚDE empresarial.",
        "a small gmbh in town",
        "Acme GmbHx",
        "GmbH",
        "Schedule A LLC Agreement",
        "Schedule (A) LLC Agreement",
        "Zapsaná v obchodním rejstříku vedeném u KS Brno.",
    ] {
        assert!(names(text).await.is_empty(), "in {text:?}");
    }
    assert_eq!(
        names("Beta LLC. The INITIAL TERM described in 5.1, this AGREEMENT shall apply.").await,
        ["Beta LLC"]
    );
}

#[tokio::test]
async fn an_ambiguous_form_needs_context() {
    let hits = found("Apple AG filed jointly.", &[]).await;
    assert_eq!(hits.len(), 1);
    assert!(!ConfidenceThreshold::BASELINE.passes(hits[0].1));

    let hits = found("The company Apple AG filed jointly.", &[]).await;
    assert!(ConfidenceThreshold::BASELINE.passes(hits[0].1));

    let hits = found("Siemens GmbH filed.", &[]).await;
    assert!(ConfidenceThreshold::BASELINE.passes(hits[0].1));
}

#[tokio::test]
async fn a_weaker_longer_name_keeps_the_stronger_one_inside_it() {
    let hits = found("Acme GmbH AG filed.", &[]).await;
    assert!(
        hits.iter().any(
            |(name, score)| name == "Acme GmbH" && ConfidenceThreshold::BASELINE.passes(*score)
        ),
        "{hits:?}"
    );
}
