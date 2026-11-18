use elide_core::redaction::Disposition;

use super::*;

fn html(policy: HiddenContentPolicy) -> MarkupSanitizer {
    MarkupSanitizer::new(Dialect::Html, policy)
}

fn xml(policy: HiddenContentPolicy) -> MarkupSanitizer {
    MarkupSanitizer::new(Dialect::Xml, policy)
}

/// `raw` rebuilt by `sanitizer`, checked to be a fixed point.
fn sanitize(sanitizer: MarkupSanitizer, raw: &str) -> String {
    let out = sanitizer.sanitize(raw.to_owned()).expect("sanitize").0;
    if let Err(error) = sanitizer.check(&out) {
        panic!("rebuild of {raw:?} is not a fixed point: {out:?}: {error}");
    }
    out
}

fn keep_comments() -> HiddenContentPolicy {
    HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent)
}

#[test]
fn removes_scripts_plugins_and_event_handlers() {
    let raw = r#"<p onclick="track('jane')">Hi</p><script>var u = {"email":"jane@example.com"};</script><embed src="x.swf"><object data="y"><param name="a">fallback</object><a href=" JavaScript:steal()">x</a><a href="/ok">y</a>"#;
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(out, r#"<p>Hi</p><a>x</a><a href="/ok">y</a>"#);
}

#[test]
fn a_handler_after_a_slash_or_unquoted_is_removed() {
    // A browser reads a `/` between attributes like whitespace, so each of
    // these is an `a` with an `onclick` handler.
    let raw = r#"<a/onclick=steal()>x</a><a title="t"/onclick=steal()>y</a><a href=javascript:steal()>z</a>"#;
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(out, r#"<a>x</a><a title="t">y</a><a>z</a>"#);
    // Attributes are written apart, so two around a removed one never join.
    let raw = r#"<img src="https://x.test/p.png"alt="Jane">"#;
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(out, r#"<img alt="Jane">"#);
}

#[test]
fn a_script_url_spelled_with_html_references_is_removed() {
    // A browser decodes HTML's named references and numeric ones without
    // a `;` before it reads the scheme.
    let raw = r#"<a href="javascript&colon;steal()">x</a><a href="jav&Tab;ascript:steal()">y</a><a href="javascript&#58steal()">z</a>"#;
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(out, "<a>x</a><a>y</a><a>z</a>");
}

#[test]
fn a_construct_a_browser_ends_early_is_refused() {
    // A browser reads each `<img>` here as an element, quick-xml as the
    // text of a comment, processing instruction, CDATA section, or
    // document type.
    for raw in [
        "<p><!--><img src=x onerror=steal()>--></p>",
        "<p><!---><img src=x onerror=steal()>--></p>",
        "<p><!-- a --!><img src=x onerror=steal()>--></p>",
        "<p><?x><img src=x onerror=steal()>?></p>",
        "<p><![CDATA[><img src=x onerror=steal()>]]></p>",
        "<!DOCTYPE html [<!-- x -->]><p>t</p>",
    ] {
        let error = html(keep_comments())
            .sanitize(raw.to_owned())
            .expect_err(raw);
        assert!(error.to_string().contains("earlier `>`"), "{raw}: {error}");
    }
}

/// A browser reads a processing instruction or CDATA section in HTML as a
/// comment, which never shows; an ordinary comment and the document type
/// stay.
#[test]
fn what_html_reads_as_a_comment_goes() {
    let raw = "<!DOCTYPE html><?xml version=\"1.0\"?><p><!-- note --><![CDATA[Jane]]><?pi x?></p>";
    assert_eq!(
        sanitize(html(keep_comments()), raw),
        "<!DOCTYPE html><p><!-- note --></p>"
    );
    // In SVG, a CDATA section is one.
    let raw = "<svg><text><![CDATA[Jane]]></text></svg>";
    assert_eq!(sanitize(html(keep_comments()), raw), raw);
}

#[test]
fn an_unclosed_script_runs_to_the_end() {
    let out = sanitize(
        html(HiddenContentPolicy::default()),
        "<p>a</p><script>var x = 1;",
    );
    assert_eq!(out, "<p>a</p>");
}

#[test]
fn comments_follow_the_collaboration_action() {
    let raw = "<p>a<!-- ask Jane --></p>";
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        "<p>a</p>"
    );
    assert_eq!(sanitize(html(keep_comments()), raw), raw);
}

#[test]
fn meta_fields_follow_the_metadata_disposition_but_render_tags_stay() {
    let raw = r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta name="author" content="Jane Roe"><meta http-equiv="content-type" content="text/html"></head>"#;
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(
        out,
        r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta http-equiv="content-type" content="text/html"></head>"#
    );
    let redact = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    assert_eq!(sanitize(html(redact), raw), raw);
}

/// A redirect or a cookie acts when the page opens, so it goes whatever the
/// metadata disposition, and a `javascript:` redirect with it.
#[test]
fn acting_meta_directives_go_under_every_policy() {
    let raw = r#"<head><meta http-equiv="refresh" content="0;url=javascript:steal()"><meta http-equiv="Set-Cookie" content="id=1"></head>"#;
    let redact = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    assert_eq!(sanitize(html(redact), raw), "<head></head>");
}

/// A browser decodes character references in a directive's name, so a
/// spelled-out redirect still redirects and is removed with the others.
#[test]
fn an_encoded_acting_directive_is_still_caught() {
    let raw =
        r#"<head><meta http-equiv="re&#102;resh" content="0;url=https://example.com/"></head>"#;
    let redact = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    assert_eq!(sanitize(html(redact), raw), "<head></head>");
}

/// A tag that names a field is metadata, even when it also sets something
/// that renders the page.
#[test]
fn a_rendering_attribute_does_not_keep_a_field() {
    for raw in [
        r#"<head><meta charset="utf-8" name="author" content="Jane Roe"></head>"#,
        r#"<head><meta http-equiv="content-type" name="author" content="Jane Roe"></head>"#,
    ] {
        let out = sanitize(html(HiddenContentPolicy::default()), raw);
        assert_eq!(out, "<head></head>");
    }
}

/// A browser decodes character references and drops tabs, line breaks, and
/// leading control characters before it reads a URL's scheme, so the check
/// does too.
#[test]
fn an_encoded_or_split_script_url_is_still_caught() {
    let policy = HiddenContentPolicy::default();
    for href in [
        "&#106;avascript:alert(1)",
        "java&#x09;script:alert(1)",
        "java\tscript:alert(1)",
        "&#x01; javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,<script>alert(1)</script>",
    ] {
        let raw = format!(r#"<a href="{href}">x</a>"#);
        assert_eq!(sanitize(html(policy), &raw), "<a>x</a>", "{href}");
    }
}

#[test]
fn frames_go_with_their_inline_or_remote_page() {
    let raw = r#"<p>a</p><iframe srcdoc="<script>s()</script>"></iframe><iframe src="https://x"/><frameset><frame src="b.html"></frameset>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        "<p>a</p>"
    );
}

/// What the page loads from a server when opened goes; a hyperlink a
/// reader follows, and a resource next to the page, stay.
#[test]
fn remote_loads_go_and_hyperlinks_stay() {
    let raw = r#"<img src="https://t/pixel.gif" alt="logo"><img src="logo.png"><img srcset="a.png 1x, https://t/b.png 2x"><a href="https://site" ping="https://t/ping">site</a><link rel="stylesheet" href="//cdn/x.css"><link rel="stylesheet" href="local.css"><base href="https://t/"><div style="background:url('https://t/bg')">d</div><style>@import "https://t/x.css";</style><style>p{color:red}</style>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        r#"<img alt="logo"><img src="logo.png"><img><a href="https://site">site</a><link rel="stylesheet" href="local.css"><div>d</div><style>p{color:red}</style>"#
    );
}

#[test]
fn undisplayed_elements_go_and_findable_ones_stay() {
    let raw = r#"<template><p>Jane</p></template><div hidden><p>Roe</p></div><img hidden src="a.png"><p hidden="until-found">found</p><input type="hidden" value="4111"><dialog><p>closed</p></dialog><dialog open>shown</dialog>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        r#"<p hidden="until-found">found</p><dialog open>shown</dialog>"#
    );
}

/// A conditional comment is markup an old browser runs, so it goes even
/// when the policy keeps comments.
#[test]
fn a_conditional_comment_goes_under_every_policy() {
    let raw = "<!--[if IE]><script>s()</script><![endif]--><!-- kept note -->";
    assert_eq!(sanitize(html(keep_comments()), raw), "<!-- kept note -->");
}

/// An element or attribute the allowlist does not name is left out, with
/// what it holds, rather than passed through.
#[test]
fn what_the_allowlist_does_not_name_goes() {
    let raw = r#"<p data-user="jane@example.com" itemprop="author" class="c">a</p><x-card><span>Jane</span></x-card><xmp><b>raw</b></xmp><p>b</p>"#;
    let (out, stripped) = html(HiddenContentPolicy::default())
        .sanitize(raw.to_owned())
        .expect("sanitize");
    assert_eq!(out, r#"<p class="c">a</p><p>b</p>"#);
    let entries: Vec<(HiddenContent, usize)> = stripped
        .iter()
        .map(|entry| (entry.content, entry.count))
        .collect();
    assert_eq!(entries, [(HiddenContent::ApplicationData, 4)]);
}

#[test]
fn a_meta_tag_repeating_an_attribute_goes_under_every_policy() {
    let raw = r#"<head><meta name="viewport" name="author" content="Jane Roe"></head>"#;
    let keep = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    assert_eq!(sanitize(html(keep), raw), "<head></head>");
}

#[test]
fn a_repeated_attribute_is_written_once() {
    let raw = r#"<p title="a" TITLE="Jane Roe">x</p>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        r#"<p title="a">x</p>"#
    );
}

#[test]
fn accessible_names_stay() {
    let raw = r#"<button aria-label="Close" role="button">x</button>"#;
    assert_eq!(sanitize(html(HiddenContentPolicy::default()), raw), raw);
}

#[test]
fn svg_draws_and_its_active_parts_go() {
    let raw = r##"<svg viewBox="0 0 10 10" data-x="1"><script>s()</script><a href="https://site"><text x="1">Jane</text></a><set attributeName="href" to="javascript:s()"/><use href="#i"/><image href="https://t/p.png"/><foreignObject><p>Roe</p></foreignObject><g display="none"><text>hidden</text></g><metadata>by Jane</metadata></svg>"##;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        r##"<svg viewBox="0 0 10 10"><a href="https://site"><text x="1">Jane</text></a><use href="#i"/><image/></svg>"##
    );
}

/// An HTML element a browser does not allow inside SVG ends it, and what
/// follows is read as HTML again.
#[test]
fn an_html_element_ends_svg() {
    let raw = r#"<svg><p onclick="x()">a</p><circle r="1"/></svg>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        "<svg><p>a</p></svg>"
    );
}

#[test]
fn mathml_renders_and_annotations_go() {
    let raw = r#"<math><semantics><mi>x</mi><annotation encoding="TeX">Jane</annotation></semantics><mphantom><mi>y</mi></mphantom></math>"#;
    assert_eq!(
        sanitize(html(HiddenContentPolicy::default()), raw),
        "<math><semantics><mi>x</mi></semantics></math>"
    );
}

#[test]
fn a_self_closing_flag_survives_an_unquoted_value() {
    // In `title=x/>` the `/` is the value's, so the tag is not self-closing.
    let raw = "<svg><rect width=2 /><rect width=3/></svg>";
    let out = sanitize(html(HiddenContentPolicy::default()), raw);
    assert_eq!(out, raw);
}

#[test]
fn xml_drops_instructions_doctype_and_includes() {
    let raw = r#"<?xml version="1.0"?><?xml-stylesheet href="https://t/s.xsl"?><?app data?><!DOCTYPE doc><doc xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="https://t/part.xml"><xi:fallback>f</xi:fallback></xi:include><p>a</p></doc>"#;
    let (out, stripped) = xml(HiddenContentPolicy::default())
        .sanitize(raw.to_owned())
        .expect("sanitize");
    assert_eq!(
        out,
        r#"<?xml version="1.0"?><doc xmlns:xi="http://www.w3.org/2001/XInclude"><p>a</p></doc>"#
    );
    let entries: Vec<(HiddenContent, usize)> = stripped
        .iter()
        .map(|entry| (entry.content, entry.count))
        .collect();
    assert_eq!(
        entries,
        [
            (HiddenContent::Link, 2),
            (HiddenContent::ApplicationData, 2)
        ]
    );
    let external = r#"<!DOCTYPE doc SYSTEM "https://t/doc.dtd"><doc/>"#;
    assert!(
        xml(HiddenContentPolicy::default())
            .sanitize(external.to_owned())
            .is_err()
    );
    // HTML's document type is never fetched, so it stays.
    let xhtml = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"><p>a</p>"#;
    assert_eq!(sanitize(html(HiddenContentPolicy::default()), xhtml), xhtml);
}

#[test]
fn the_record_names_each_removal_by_what_it_is() {
    let raw = r#"<head><meta name="author" content="Jane"></head><!-- note --><p onclick="x()">a</p><script>s()</script><object>o</object>"#;
    let (_, stripped) = html(HiddenContentPolicy::default())
        .sanitize(raw.to_owned())
        .expect("sanitize");
    let counts: Vec<(HiddenContent, usize)> = stripped
        .iter()
        .map(|entry| (entry.content, entry.count))
        .collect();
    assert_eq!(
        counts,
        [
            (HiddenContent::Properties, 1),
            (HiddenContent::Comment, 1),
            (HiddenContent::Script, 2),
            (HiddenContent::Control, 1),
        ]
    );
}

#[test]
fn xml_keeps_its_elements_as_parsed() {
    let raw = r#"<doc onload="x"><!-- note --><script>kept</script><![CDATA[a<b]]></doc>"#;
    assert_eq!(
        sanitize(xml(HiddenContentPolicy::default()), raw),
        r#"<doc onload="x"><script>kept</script><![CDATA[a<b]]></doc>"#
    );
}

#[test]
fn a_document_type_declaring_entities_is_refused() {
    let raw = r#"<!DOCTYPE d [<!ENTITY who "Jane Roe">]><d>&who;</d>"#;
    assert!(
        xml(HiddenContentPolicy::default())
            .sanitize(raw.to_owned())
            .is_err()
    );
}

#[test]
fn check_refuses_what_a_rebuild_would_change() {
    let sanitizer = html(HiddenContentPolicy::default());
    assert!(sanitizer.check("<p>clean</p>").is_ok());
    assert!(sanitizer.check("<p>a<!-- x --></p>").is_err());
    assert!(sanitizer.check(r#"<p data-x="1">a</p>"#).is_err());
}

#[test]
fn a_rebuild_keeps_text_and_references_as_written() {
    let raw = "\u{feff}<p title=\"a&amp;b\">Jane&nbsp;Roe &lt; x</p><br><br/><img alt=x>";
    assert_eq!(sanitize(html(HiddenContentPolicy::default()), raw), raw);
}
