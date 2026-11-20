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

/// `raw` rebuilt under the default policy, checked to be a fixed point.
fn rebuilt(raw: &str) -> String {
    sanitize(html(HiddenContentPolicy::default()), raw)
}

/// Assert that `raw` rebuilds to the paragraph `Visible` alone.
fn only_visible(raw: &str) {
    assert_eq!(rebuilt(raw), "<p>Visible</p>", "{raw}");
}

/// Whether `sanitizer` refuses `raw`.
fn refused(sanitizer: MarkupSanitizer, raw: &str) -> bool {
    sanitizer.sanitize(raw.to_owned()).is_err()
}

/// What `stripped` records, as `(content, count)` in order.
fn removals(stripped: &StrippedContent) -> Vec<(HiddenContent, usize)> {
    stripped
        .iter()
        .map(|entry| (entry.content, entry.count))
        .collect()
}

fn keep_comments() -> HiddenContentPolicy {
    HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent)
}

#[test]
fn removes_scripts_plugins_and_event_handlers() {
    let raw = r#"<p onclick="track('jane')">Hi</p><script>var u = {"email":"jane@example.com"};</script><embed src="x.swf"><object data="y"><param name="a">fallback</object><a href=" JavaScript:steal()">x</a><a href="/ok">y</a>"#;
    let out = rebuilt(raw);
    assert_eq!(out, r#"<p>Hi</p><a>x</a><a href="/ok">y</a>"#);
}

#[test]
fn a_handler_after_a_slash_or_unquoted_is_removed() {
    // A browser reads a `/` between attributes like whitespace, so each of
    // these is an `a` with an `onclick` handler.
    let raw = r#"<a/onclick=steal()>x</a><a title="t"/onclick=steal()>y</a><a href=javascript:steal()>z</a>"#;
    let out = rebuilt(raw);
    assert_eq!(out, r#"<a>x</a><a title="t">y</a><a>z</a>"#);
    // Attributes are written apart, so two around a removed one never join.
    let raw = r#"<img src="https://x.test/p.png"alt="Jane">"#;
    let out = rebuilt(raw);
    assert_eq!(out, r#"<img alt="Jane">"#);
}

// HTML's named references are decoded only with the `html` feature.
#[cfg(feature = "html")]
#[test]
fn a_script_url_spelled_with_html_references_is_removed() {
    // A browser decodes HTML's named references and numeric ones without
    // a `;` before it reads the scheme.
    let raw = r#"<a href="javascript&colon;steal()">x</a><a href="jav&Tab;ascript:steal()">y</a><a href="javascript&#58steal()">z</a>"#;
    let out = rebuilt(raw);
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
    let out = rebuilt("<p>a</p><script>var x = 1;");
    assert_eq!(out, "<p>a</p>");
}

#[test]
fn comments_follow_the_collaboration_action() {
    let raw = "<p>a<!-- ask Jane --></p>";
    assert_eq!(rebuilt(raw), "<p>a</p>");
    assert_eq!(sanitize(html(keep_comments()), raw), raw);
}

#[test]
fn meta_fields_follow_the_metadata_disposition_but_render_tags_stay() {
    let raw = r#"<head><meta charset="utf-8"><meta name="viewport" content="w"><meta name="author" content="Jane Roe"><meta http-equiv="content-type" content="text/html"></head>"#;
    let out = rebuilt(raw);
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
        let out = rebuilt(raw);
        assert_eq!(out, "<head></head>");
    }
}

/// A browser decodes character references and drops tabs, line breaks, and
/// leading control characters before it reads a URL's scheme, so the check
/// does too.
#[test]
fn an_encoded_or_split_script_url_is_still_caught() {
    for href in [
        "&#106;avascript:alert(1)",
        "java&#x09;script:alert(1)",
        "java\tscript:alert(1)",
        "&#x01; javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html,<script>alert(1)</script>",
    ] {
        let raw = format!(r#"<a href="{href}">x</a>"#);
        assert_eq!(rebuilt(&raw), "<a>x</a>", "{href}");
    }
}

#[test]
fn frames_go_with_their_inline_or_remote_page() {
    let raw = r#"<p>a</p><iframe srcdoc="<script>s()</script>"></iframe><iframe src="https://x"/><frameset><frame src="b.html"></frameset>"#;
    assert_eq!(rebuilt(raw), "<p>a</p>");
}

/// What the page loads from a server when opened goes; a hyperlink a
/// reader follows, and a resource next to the page, stay.
#[test]
fn remote_loads_go_and_hyperlinks_stay() {
    let raw = r#"<img src="https://t/pixel.gif" alt="logo"><img src="logo.png"><img srcset="a.png 1x, https://t/b.png 2x"><a href="https://site" ping="https://t/ping">site</a><link rel="stylesheet" href="//cdn/x.css"><link rel="stylesheet" href="local.css"><base href="https://t/"><div style="background:url('https://t/bg')">d</div><style>@import "https://t/x.css";</style><style>p{color:red}</style>"#;
    assert_eq!(
        rebuilt(raw),
        r#"<img alt="logo"><img src="logo.png"><img><a href="https://site">site</a><link rel="stylesheet" href="local.css"><div>d</div><style>p{color:red}</style>"#
    );
}

#[test]
fn undisplayed_elements_go_and_findable_ones_stay() {
    let raw = r#"<template><p>Jane</p></template><div hidden><p>Roe</p></div><img hidden src="a.png"><p hidden="until-found">found</p><input type="hidden" value="4111"><dialog><p>closed</p></dialog><dialog open>shown</dialog>"#;
    assert_eq!(
        rebuilt(raw),
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
    assert_eq!(removals(&stripped), [(HiddenContent::ApplicationData, 4)]);
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
    assert_eq!(rebuilt(raw), r#"<p title="a">x</p>"#);
}

#[test]
fn accessible_names_stay() {
    let raw = r#"<button aria-label="Close" role="button">x</button>"#;
    assert_eq!(rebuilt(raw), raw);
}

#[test]
fn svg_draws_and_its_active_parts_go() {
    let raw = r##"<svg viewBox="0 0 10 10" data-x="1"><script>s()</script><a href="https://site"><text x="1">Jane</text></a><set attributeName="href" to="javascript:s()"/><use href="#i"/><image href="https://t/p.png"/><foreignObject><p>Roe</p></foreignObject><g display="none"><text>hidden</text></g><metadata>by Jane</metadata></svg>"##;
    assert_eq!(
        rebuilt(raw),
        r##"<svg viewBox="0 0 10 10"><a href="https://site"><text x="1">Jane</text></a><use href="#i"/><image/></svg>"##
    );
}

/// An HTML element a browser does not allow inside SVG ends it, and what
/// follows is read as HTML again: there a `<circle/>` is an unknown element
/// the self-closing flag does not close, holding the rest.
#[test]
fn an_html_element_ends_svg() {
    let raw = r#"<svg><p onclick="x()">a</p><circle r="1"/></svg>b"#;
    assert_eq!(rebuilt(raw), "<svg><p>a</p>");
}

#[test]
fn mathml_renders_and_annotations_go() {
    let raw = r#"<math><semantics><mi>x</mi><annotation encoding="TeX">Jane</annotation></semantics><mphantom><mi>y</mi></mphantom></math>"#;
    assert_eq!(
        rebuilt(raw),
        "<math><semantics><mi>x</mi></semantics></math>"
    );
}

#[test]
fn a_self_closing_flag_survives_an_unquoted_value() {
    // In `title=x/>` the `/` is the value's, so the tag is not self-closing.
    let raw = "<svg><rect width=2 /><rect width=3/></svg>";
    let out = rebuilt(raw);
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
    assert_eq!(
        removals(&stripped),
        [
            (HiddenContent::Link, 2),
            (HiddenContent::ApplicationData, 2)
        ]
    );
    let external = r#"<!DOCTYPE doc SYSTEM "https://t/doc.dtd"><doc/>"#;
    assert!(refused(xml(HiddenContentPolicy::default()), external));
    // HTML's document type is never fetched, so it stays.
    let xhtml = r#"<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Strict//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-strict.dtd"><p>a</p>"#;
    assert_eq!(rebuilt(xhtml), xhtml);
}

#[test]
fn the_record_names_each_removal_by_what_it_is() {
    let raw = r#"<head><meta name="author" content="Jane"></head><!-- note --><p onclick="x()">a</p><script>s()</script><object>o</object>"#;
    let (_, stripped) = html(HiddenContentPolicy::default())
        .sanitize(raw.to_owned())
        .expect("sanitize");
    assert_eq!(
        removals(&stripped),
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
    assert!(refused(xml(HiddenContentPolicy::default()), raw));
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
    assert_eq!(rebuilt(raw), raw);
}

/// An enumerated keyword is compared untrimmed: `" hidden "` is no input type
/// and no `hidden` value a browser knows, so the first input shows and the
/// paragraph is hidden outright.
#[test]
fn keywords_are_compared_as_written() {
    let raw = r#"<input type=" hidden " value="Draft"><input type="HIDDEN" value="x"><p hidden=" until-found ">Jane</p>"#;
    assert_eq!(rebuilt(raw), r#"<input type=" hidden " value="Draft">"#);
}

#[test]
fn an_undisplayed_svg_root_goes() {
    let raw = r#"<svg display=" none "><text>Jane</text></svg><p>Roe</p>"#;
    assert_eq!(rebuilt(raw), "<p>Roe</p>");
}

/// A reference standing for the comma between candidates still separates them.
#[test]
fn a_remote_srcset_candidate_behind_a_reference_goes() {
    let raw =
        r#"<img srcset="a.png&#44;https://t/p.png 2x" alt="x"><img srcset="a.png 1x, b.png 2x">"#;
    assert_eq!(
        rebuilt(raw),
        r#"<img alt="x"><img srcset="a.png 1x, b.png 2x">"#
    );
}

#[test]
fn markup_inside_a_textarea_or_title_is_refused() {
    let sanitizer = html(HiddenContentPolicy::default());
    for raw in [
        "<textarea><x-note>Draft</x-note></textarea>",
        "<title>a<b>b</b></title>",
        "<textarea/><i>Draft</i></textarea>",
    ] {
        assert!(refused(sanitizer, raw), "{raw}");
    }
    let raw = "<title>Jane &amp; Roe</title><textarea>notes</textarea>";
    assert_eq!(sanitize(sanitizer, raw), raw);
}

/// A removal never joins text into a `]]>`, which XML forbids in text.
#[test]
fn xml_text_stays_well_formed_across_a_removal() {
    let raw = "<doc>]]<!-- c -->></doc>";
    assert_eq!(
        sanitize(xml(HiddenContentPolicy::default()), raw),
        "<doc>]]&gt;</doc>"
    );
}

#[test]
fn an_xinclude_namespace_spelled_with_references_is_still_caught() {
    let raw = r#"<doc xmlns:xi="&#x68;ttp://www.w3.org/2001/XInclude"><xi:include href="secret.xml"/><p>x</p></doc>"#;
    assert_eq!(
        sanitize(xml(HiddenContentPolicy::default()), raw),
        r#"<doc xmlns:xi="&#x68;ttp://www.w3.org/2001/XInclude"><p>x</p></doc>"#
    );
}

#[test]
fn an_xinclude_root_is_refused() {
    let raw = r#"<xi:include xmlns:xi="http://www.w3.org/2001/XInclude" href="secret.xml"/>"#;
    assert!(refused(xml(HiddenContentPolicy::default()), raw));
}

/// A hidden element ends where a browser ends it, not only at its end tag;
/// and a self-closing flag does not end a non-void one.
#[test]
fn a_hidden_element_ends_where_a_browser_ends_it() {
    assert_eq!(
        rebuilt("<p hidden>Draft<p>Published</p>"),
        "<p>Published</p>"
    );
    assert_eq!(
        rebuilt("<ul><li hidden>Jane<li>Roe</ul>"),
        "<ul><li>Roe</ul>"
    );
    assert_eq!(rebuilt("<div hidden/>Jane</div>Roe"), "Roe");
    assert_eq!(
        rebuilt(r#"<svg><g display="none"><text>Jane</text><p>Roe</p>"#),
        "<svg><p>Roe</p>"
    );
}

/// When the element that leaves SVG is left out, the output still leaves SVG
/// there, so a rebuild of it reads what follows as HTML too.
#[test]
fn leaving_svg_through_a_removed_element_is_kept() {
    let raw = r#"<svg><p hidden></p><a title="Label">x</a></svg>"#;
    assert_eq!(
        rebuilt(raw),
        r#"<svg><span></span><a title="Label">x</a></svg>"#
    );
}

/// An SVG style sheet a breakout ends is closed there, so what follows stays.
#[test]
fn a_breakout_closes_an_svg_style_sheet() {
    let raw = "<svg><style>@import url(https://t/a.css);<p>Roe</p>";
    assert_eq!(rebuilt(raw), "<svg><p>Roe</p>");
}

/// HTML inside an SVG `desc` stays in SVG, which resumes after it.
#[test]
fn html_inside_an_integration_point_stays_in_svg() {
    let raw = r#"<svg><desc><span>Note</span></desc><circle r="5"/></svg><math><mi><b>x</b></mi><mn>1</mn></math>"#;
    assert_eq!(rebuilt(raw), raw);
}

/// A browser reads the first of a repeated attribute, so a removed first one
/// does not hand its place to the next.
#[test]
fn a_removed_first_attribute_is_not_replaced_by_a_repeat() {
    let raw = r#"<img src="https://t/p.png" src="/local.png" alt="x">"#;
    assert_eq!(rebuilt(raw), r#"<img alt="x">"#);
}

/// A reference in an SVG style sheet is decoded before the remote-load check,
/// as a browser decodes it; one in an HTML style sheet is read as written.
#[test]
fn a_remote_import_spelled_with_references_in_svg_goes() {
    for raw in [
        "<svg><style>@import url(https&#58;//t/a.css);</style></svg>",
        r#"<svg><style>@import "https&#x3a;//t/a.css";</style></svg>"#,
    ] {
        assert_eq!(rebuilt(raw), "<svg></svg>", "{raw}");
    }
    for raw in [
        "<style>p { content: 'https&#58;//t' }</style>",
        r#"<style>p { background: url("https&#58;//local.png") }</style>"#,
    ] {
        assert_eq!(rebuilt(raw), raw, "{raw}");
    }
    // A comment or a tag in an HTML style sheet is text the CSS reads.
    for raw in [
        "<style><!-- @import url(https://t/a.css); --></style><p>a</p>",
        "<style><b>@import url(https://t/a.css);</b></style><p>a</p>",
    ] {
        assert_eq!(rebuilt(raw), "<p>a</p>", "{raw}");
    }
    let raw = "<style><!-- p { color: red } --></style>";
    assert_eq!(rebuilt(raw), raw);
}

/// The root of SVG or MathML honours its self-closing flag, even removed.
#[test]
fn a_removed_self_closing_svg_root_holds_nothing() {
    let raw = "<svg hidden/><math hidden/><p>Visible</p>";
    only_visible(raw);
    // The `/` belongs to the unquoted value, so the root holds what follows,
    // up to its end tag or an HTML element that breaks out of it.
    for raw in [
        r#"<svg display="none" id=x/>Secret</svg><p>Visible</p>"#,
        r#"<svg display="none" id=x/>Secret<p>Visible</p>"#,
        // The `/` belongs to `id`, so `desc` stays open and holds the `p`.
        r#"<svg hidden><desc id=x/><p>Secret</p></desc></svg><p>Visible</p>"#,
    ] {
        only_visible(raw);
    }
    // An SVG `mi` is not a MathML integration point, so the `p` breaks out;
    // a browser ignores the `</svg>` left after it.
    assert_eq!(
        rebuilt("<svg hidden><mi>Secret<p>Visible</p></svg>"),
        "<p>Visible</p></svg>"
    );
}

/// A text element inside a removed element reads its body as text up to its
/// own end tag, so an end tag in that text does not end the removal.
#[test]
fn a_text_element_in_a_removed_element_holds_its_end_tags() {
    for raw in [
        "<svg hidden><desc><textarea></svg>Secret</textarea></desc></svg><p>Visible</p>",
        "<p hidden><script></p>Secret</script></p><p>Visible</p>",
        "<script></p>Secret</script><p>Visible</p>",
    ] {
        only_visible(raw);
    }
}

/// Inside a removed element, each open element reads what follows in its own
/// vocabulary: an SVG `title` in an `svg` inside `desc` is SVG, not HTML
/// text, while a `title` in an HTML `div` inside `desc` is HTML text.
#[test]
fn a_removed_region_reads_each_element_in_its_own_vocabulary() {
    for raw in [
        "<svg hidden><desc><svg><title></svg></desc></svg><p>Visible</p>",
        "<svg hidden><desc><div><title></svg></title></div></desc></svg><p>Visible</p>",
        "<svg hidden><desc><textarea></svg></textarea></desc></svg><p>Visible</p>",
        // The breakout `p` closes the SVG, then the hidden paragraph.
        "<p hidden>Secret<svg><p>Visible</p>",
    ] {
        only_visible(raw);
    }
}

/// End tags naming nothing open are turned away without a scan of what is
/// open, so many of them under many open elements stay linear.
#[test]
fn unmatched_end_tags_under_deep_nesting_stay_linear() {
    let n = 50_000;
    let raw = format!(
        "<math><mi>{}{}</mi></math><div hidden>{}{}</div><p>Visible</p>",
        "<span>".repeat(n),
        "</b>".repeat(n),
        "<span>".repeat(n),
        "</b>".repeat(n),
    );
    let started = std::time::Instant::now();
    let out = rebuilt(&raw);
    assert!(
        out.ends_with("<p>Visible</p>"),
        "{}",
        &out[out.len().saturating_sub(80)..]
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

/// A void element in a removed region holds nothing, and closes what it
/// implies the end of, self-closed or not.
#[test]
fn a_void_element_in_a_removed_region_opens_nothing() {
    for raw in [
        "<math hidden><mi><p>x<hr><mglyph><title></math><p>Visible</p>",
        "<math hidden><mi><p>x<hr/><mglyph><title></math><p>Visible</p>",
        "<div hidden><br>Secret</div><p>Visible</p>",
    ] {
        only_visible(raw);
    }
}

/// A start tag that closes what is open in the removed element, and then the
/// removed element itself, ends the removal.
#[test]
fn an_implied_close_through_a_removed_element_ends_it() {
    let raw = r#"<select><optgroup hidden><option>Secret<optgroup label="Public"><option>Visible</select><p>After</p>"#;
    let out = rebuilt(raw);
    assert!(!out.contains("Secret"), "{out}");
    assert!(
        out.contains("Visible") && out.ends_with("<p>After</p>"),
        "{out}"
    );
}

/// A browser reads a carriage return as a line feed, so one ends an unquoted
/// value and starts another attribute.
#[test]
fn a_carriage_return_separates_attributes() {
    for raw in [
        "<a title=x\ronclick=alert(1)>a</a>",
        "<a title=\"x\"\ronclick=alert(1)>a</a>",
        "<a\ronclick=alert(1)>a</a>",
    ] {
        let out = rebuilt(raw);
        assert!(!out.contains("onclick"), "{raw:?} -> {out:?}");
    }
}

/// An `mglyph` or `malignmark` in a MathML token element stays MathML, so a
/// `title` in it is not HTML text and `</math>` still ends the math.
#[test]
fn mglyph_and_malignmark_stay_mathml() {
    for raw in [
        "<math hidden><mi><mglyph><title></math><p>Visible</p>",
        "<math hidden><mi><malignmark><title></math><p>Visible</p>",
    ] {
        only_visible(raw);
    }
    // Other children of `mi` are still HTML: a `title` there is text.
    let raw = "<math hidden><mi><b><title></math></title></b></mi></math><p>Visible</p>";
    only_visible(raw);
    // Under an HTML element in `mi`, an `mglyph` is HTML: its self-closing
    // flag is ignored and it holds what follows.
    let raw = "<math><mi><span><mglyph hidden/>Secret</mglyph></span></mi></math>";
    let out = rebuilt(raw);
    assert!(!out.contains("Secret"), "{out}");
    // A second `<p>` closes the first, so after its end the `mglyph` is
    // right in `mi` again and its self-closing flag holds.
    let raw = "<math><mi><p>a<p>b</p><mglyph/>Visible</mi></math><p>After</p>";
    let out = rebuilt(raw);
    assert!(
        out.contains("Visible") && out.ends_with("<p>After</p>"),
        "{out}"
    );
    // While the `span` is open, the `p` it holds does not end it.
    let raw = "<math><mi><span><p>a<p>b</p><mglyph hidden/>Secret</mglyph></span></mi></math>";
    let out = rebuilt(raw);
    assert!(!out.contains("Secret"), "{out}");
    // A kept `mi` holding a self-closed `mglyph` does not hold what follows.
    let out = rebuilt("<math><mi><mglyph/>x</mi></math><p>Visible</p>");
    assert!(out.ends_with("<p>Visible</p>"), "{out}");
}

/// `raw` rebuilt well within a few seconds, ending in `Visible`.
fn rebuilds_quickly(raw: &str) {
    let started = std::time::Instant::now();
    let out = rebuilt(raw);
    assert!(out.contains("Visible"), "{}", &out[..80.min(out.len())]);
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

/// Many distinct attributes, written `n` times.
fn distinct_attributes(n: usize) -> String {
    (0..n).map(|i| format!(" data-a{i}=\"1\"")).collect()
}

/// A `<meta>` of many distinct attributes is read in linear time, and one
/// repeating an attribute still goes whole.
#[test]
fn a_meta_tag_of_many_attributes_stays_linear() {
    rebuilds_quickly(&format!(
        "<meta{}><p>Visible</p>",
        distinct_attributes(50_000)
    ));
    only_visible(r#"<meta name="a" NAME="b" content="c"><p>Visible</p>"#);
}

/// A tag of many distinct attributes is read in linear time, and a removed
/// first copy of an attribute still leaves its repeat out.
#[test]
fn a_tag_of_many_attributes_stays_linear() {
    rebuilds_quickly(&format!(
        "<div{}>Visible</div>",
        distinct_attributes(50_000)
    ));
    let out = rebuilt(r#"<a onclick="x()" ONCLICK="y()">a</a>"#);
    assert_eq!(out, "<a>a</a>");
}

/// An SVG presentation attribute is CSS, whose `url()` may load from a
/// server; a reference into the document stays.
#[test]
fn svg_paint_that_loads_from_a_server_is_removed() {
    for key in [
        "fill",
        "stroke",
        "filter",
        "clip-path",
        "mask",
        "marker-start",
        "marker-mid",
        "marker-end",
    ] {
        let raw = format!(
            r#"<svg><rect width="1" height="1" {key}="url(https://example.test/p.svg#g)"/></svg>"#
        );
        let out = rebuilt(&raw);
        assert!(!out.contains("example.test"), "{out}");
        let raw = format!(r#"<svg><rect {key}="URL( '&#x2f;/example.test/p')"/></svg>"#);
        let out = rebuilt(&raw);
        assert!(!out.contains("example.test"), "{out}");
        let raw = format!(r##"<svg><rect {key}="url(#g)"/></svg>"##);
        assert_eq!(rebuilt(&raw), raw);
    }
}

/// A browser ends an HTML `<style>` at the first `</style`, wherever the XML
/// reader sees a comment, a section or a tag around it.
#[test]
fn a_style_end_hidden_in_a_construct_is_refused() {
    let sanitizer = html(HiddenContentPolicy::default());
    for raw in [
        "<style><!--</style><img src=x onerror=alert(1)>--></style>",
        "<style><!--</STYLE ><img src=x onerror=alert(1)>--></style>",
        "<style><![CDATA[</style><img src=x onerror=alert(1)>]]></style>",
        "<style><?x </style><img src=x onerror=alert(1)>?></style>",
        r#"<style><x a="</style><img src=x onerror=alert(1)>"></style>"#,
        "<style><!DOCTYPE </style/><img src=x onerror=alert(1)>></style>",
    ] {
        assert!(refused(sanitizer, raw), "{raw}");
        assert!(sanitizer.check(raw).is_err(), "{raw}");
    }
    // An ordinary comment in a style sheet stays.
    let raw = "<style><!-- p { color: red } --></style><p>a</p>";
    assert_eq!(rebuilt(raw), raw);
}

/// A style sheet nested in an SVG one leaves the outer one still checked.
#[test]
fn a_nested_svg_style_sheet_is_refused() {
    let sanitizer = html(HiddenContentPolicy::default());
    for raw in [
        r#"<svg><style>@import "https://example.test/a.css";<style></style></style></svg>"#,
        r#"<svg><style><style></style>@import "https://example.test/a.css";</style></svg>"#,
    ] {
        let result = sanitizer.sanitize(raw.to_owned());
        assert!(
            result
                .as_ref()
                .is_err_and(|_| sanitizer.check(raw).is_err()),
            "{result:?}"
        );
    }
    let raw = "<svg><style>rect { fill: red }</style></svg>";
    assert_eq!(rebuilt(raw), raw);
}

/// With script on, a browser reads a `<noscript>` body as text up to the
/// first `</noscript`, wherever the XML reader sees it.
#[test]
fn a_noscript_end_hidden_in_a_construct_is_refused() {
    for raw in [
        r#"<noscript><p title="</noscript><img src=x onerror=alert(1)>"></p></noscript>"#,
        "<noscript><style></noscript><img src=x onerror=alert(1)></style></noscript>",
        "<noscript><!--</NOSCRIPT><img src=x onerror=alert(1)>--></noscript>",
    ] {
        for sanitizer in [html(HiddenContentPolicy::default()), html(keep_comments())] {
            assert!(refused(sanitizer, raw), "{raw}");
        }
    }
    let raw = r#"<noscript><p title="a">Enable script</p></noscript><p>b</p>"#;
    assert_eq!(rebuilt(raw), raw);
}

/// CSS reads its escapes and comments before its tokens, so a remote load
/// spelled with them is still caught, in a style attribute, a style sheet
/// and an SVG presentation attribute.
#[test]
fn a_remote_css_load_spelled_with_escapes_or_comments_goes() {
    for css in [
        r"background: url(htt\70s://example.test/a.png)",
        r"background: url(htt\000070 s://example.test/a.png)",
        r"background: u\72l(https://example.test/a.png)",
        r"background: \75 rl(https://example.test/a.png)",
        r"background: url('ht\tps://example.test/a.png')",
        "background: url('ht\\\ntps://example.test/a.png')",
        r"background: url(\2f\2f example.test/a.png)",
        r"background: image-set('https://example.test/a.png' 1x)",
        r"background: -webkit-image-set('https://example.test/a.png' 1x)",
        r"background:\55rl(https://example.test/a.png)",
        r"background: url(\48ttps://example.test/a.png)",
        r"background: url(\48TTPS://example.test/a.png)",
        r"background: url(/**/https://example.test/a.png)",
        r"background: ur/**/l(https://example.test/a.png)",
    ] {
        let raw = format!(r#"<p style="{css}">a</p>"#);
        let out = rebuilt(&raw);
        assert_eq!(out, "<p>a</p>", "{raw}");
        let raw = format!("<style>p {{ {css} }}</style><p>a</p>");
        assert_eq!(rebuilt(&raw), "<p>a</p>", "{raw}");
    }
    for css in [
        r#"@im\70ort "https://example.test/a.css";"#,
        r#"@import/**/"https://example.test/a.css";"#,
        r#"@\69mport 'https://example.test/a.css';"#,
        r#"@\49mport 'https://example.test/a.css';"#,
        r#"@import '\48ttps://example.test/a.css';"#,
        r#"@font-face { src: src("https://example.test/f.woff") }"#,
    ] {
        let raw = format!("<style>{css}</style><p>a</p>");
        assert_eq!(rebuilt(&raw), "<p>a</p>", "{raw}");
    }
    let raw = r#"<svg><rect fill="url(htt\70s://example.test/p#g)"/></svg>"#;
    assert!(!rebuilt(raw).contains("example.test"));
    // A local reference and an escape that loads nothing stay.
    let raw = r#"<p style="background: url(\2f a.png); content: '\201C'">a</p>"#;
    assert_eq!(rebuilt(raw), raw);
}

/// An SVG attribute read as text keeps a value that only names a URL.
#[test]
fn svg_text_attributes_naming_a_url_stay() {
    for raw in [
        r#"<svg aria-label="Example: url(https://example.test/a.png)"></svg>"#,
        r#"<svg><rect aria-describedby="x" class="url(https://example.test/a.png)"/></svg>"#,
    ] {
        assert_eq!(rebuilt(raw), raw);
    }
}

/// CSS reads a backslash before a line break as a line continuation, so the
/// break is not dropped as a URL parser would drop it before reading CSS.
#[test]
fn a_css_line_continuation_does_not_hide_a_remote_load() {
    let escaped = "url('\\20\\\n\\68ttps://example.test/p.svg#g')";
    for raw in [
        format!(r#"<svg><rect fill="{escaped}"/></svg>"#),
        format!(r#"<svg><rect style="fill: {escaped}"/></svg>"#),
        format!(r#"<p style="background: {escaped}">a</p>"#),
        format!("<style>p {{ background: {escaped} }}</style><p>a</p>"),
        format!("<svg><style>rect {{ fill: {escaped} }}</style></svg>"),
    ] {
        let out = rebuilt(&raw);
        assert!(!out.contains("example.test"), "{raw}: {out}");
    }
}

/// Only an end tag of the element's own name ends a style sheet or a
/// `<noscript>`, as a browser reads it.
#[test]
fn a_longer_end_tag_name_does_not_end_a_text_body() {
    for raw in [
        r#"<style>p::after{content:"</stylesheet>"}</style><p>a</p>"#,
        r#"<noscript><p title="</noscripts>">a</p></noscript>"#,
    ] {
        assert_eq!(rebuilt(raw), raw);
    }
    let sanitizer = html(HiddenContentPolicy::default());
    for raw in [
        "<style><!--</style\t><img src=x onerror=alert(1)>--></style>",
        "<style><!--</style/><img src=x onerror=alert(1)>--></style>",
        "<style><!--</style\n--></style>",
        r#"<noscript><p title="</noscript/x"></p></noscript>"#,
    ] {
        assert!(refused(sanitizer, raw), "{raw:?}");
    }
}

#[test]
fn a_removal_never_joins_text_into_a_reference() {
    // The text reads `&amp;` as written; joined, it would read `&`.
    assert_eq!(rebuilt("<p>&am<!-- note -->p;</p>"), "<p>&amp;amp;</p>");
    assert_eq!(rebuilt("<p>&am<b hidden>x</b>p;</p>"), "<p>&amp;amp;</p>");
    // A number without its `;` reads as its character either way.
    assert_eq!(rebuilt("<p>&#38<!-- note -->4;</p>"), "<p>&amp;4;</p>");
    // A `#` continues a reference too, and a name a browser reads without
    // its `;` is written as what it reads.
    assert_eq!(
        rebuilt("<p>&<b hidden>x</b>#60;script&gt;</p>"),
        "<p>&amp;#60;script&gt;</p>"
    );
    assert_eq!(rebuilt("<p>&am<b hidden>x</b>px</p>"), "<p>&amp;ampx</p>");
    assert_eq!(rebuilt("<p>a&amp<b hidden>x</b>;</p>"), "<p>a&amp;;</p>");
    // A reference already whole on either side is left as written.
    assert_eq!(rebuilt("<p>&amp;<!-- note -->p;</p>"), "<p>&amp;p;</p>");
    assert_eq!(rebuilt("<p>a&<!-- note --> b</p>"), "<p>a& b</p>");
}

#[test]
fn a_cdata_section_in_svg_or_mathml_holds_a_gt() {
    for raw in [
        "<svg><text><![CDATA[a > b]]></text></svg>",
        "<math><mtext><![CDATA[a > b]]></mtext></math>",
    ] {
        assert_eq!(sanitize(html(keep_comments()), raw), raw);
    }
    // In HTML read inside them, a browser reads it as a comment ended at
    // its first `>`.
    for raw in [
        "<math><mtext><b><![CDATA[a > b]]></b></mtext></math>",
        "<svg><desc><p><![CDATA[a > b]]></p></desc></svg>",
    ] {
        assert!(refused(html(keep_comments()), raw), "{raw}");
    }
}

#[test]
fn a_cdata_section_right_in_a_mathml_token_element_stays() {
    let raw = "<math><mtext><![CDATA[Note]]></mtext></math>";
    assert_eq!(rebuilt(raw), raw);
    // In an HTML element inside it, a browser reads it as a comment.
    assert_eq!(
        rebuilt("<math><mtext><b><![CDATA[Note]]></b></mtext></math>"),
        "<math><mtext><b></b></mtext></math>"
    );
    assert_eq!(
        rebuilt("<math><mtext><b>x</b><![CDATA[Note]]></mtext></math>"),
        "<math><mtext><b>x</b><![CDATA[Note]]></mtext></math>"
    );
    // Past misnested tags a browser may still have an HTML element open, so
    // the section goes and the `mtext`'s end is refused.
    let raw = "<math><mtext><b><i>x</b><![CDATA[Note]]></mtext></math>";
    assert!(refused(html(keep_comments()), raw), "{raw}");
}

#[test]
fn an_unquoted_value_ending_in_a_quote_keeps_its_space_before_the_slash() {
    let raw = "<svg><circle id=x'' /><rect/></svg>";
    assert_eq!(rebuilt(raw), raw);
}

/// A browser ends a list item at its list's end tag, which this reader does
/// not follow outside the item, so the rest goes with it.
#[test]
fn a_dropped_list_item_goes_with_what_follows() {
    for raw in [
        "<ul><li hidden>Draft</ul><p>Published</p>",
        "<ul><li><div>a<li hidden>secret</div>more secret</ul>",
        "<dl><dd><div>a<dd hidden>secret</div>more secret</dl>",
        "<button><div>a<button hidden>secret</div>more secret</button>",
    ] {
        let out = rebuilt(raw);
        assert!(!out.contains("secret") && !out.contains("Draft"), "{out}");
    }
}

/// An end tag a browser ignores, because an element open inside the one
/// being left out keeps it from closing it, does not end the removal.
#[test]
fn an_end_tag_held_open_by_an_element_inside_does_not_end_a_removal() {
    for raw in [
        "<div hidden><table><tr><td>a</div>more secret</td></tr></table></div>",
        "<div hidden><svg><foreignObject><p>a</div>more secret",
        "<span hidden><div>a</span>more secret",
        "<form hidden><div>a</form>more secret",
        "<li hidden><ul><li>a</li></ul></ol>more secret",
    ] {
        let out = rebuilt(raw);
        assert!(!out.contains("secret"), "{raw}: {out}");
    }
    // One that closes everything open inside it does.
    assert_eq!(
        rebuilt("<div hidden><p>a<span>b</div><p>Visible</p>"),
        "<p>Visible</p>"
    );
    assert_eq!(
        rebuilt("<math hidden><mi><mglyph><title></math><p>Visible</p>"),
        "<p>Visible</p>"
    );
}

/// In SVG or MathML, even an element reading HTML, an end tag closes the
/// innermost element of its name with everything inside it.
#[test]
fn an_end_tag_in_svg_or_mathml_closes_its_ancestor() {
    // After `</math>`, the CDATA section is an HTML comment.
    for raw in [
        "<math><mtext></math><![CDATA[><img src=x onerror=alert(1)>]]>",
        "<svg><desc></svg><![CDATA[><img src=x onerror=alert(1)>]]>",
    ] {
        assert!(refused(html(keep_comments()), raw), "{raw}");
    }
    // After `</mrow>`, the `math` is open, where it is a CDATA section.
    let raw = "<math><mrow><mi></mrow><![CDATA[a > b]]></math>";
    assert_eq!(sanitize(html(keep_comments()), raw), raw);
    assert_eq!(
        rebuilt("<math><mtext></math><![CDATA[Note]]><p>a</p>"),
        "<math><mtext></math><p>a</p>"
    );
    // An end tag that may close the HTML around them is refused.
    let raw = "<div><svg><g></div><![CDATA[><img src=x onerror=alert(1)>]]>";
    assert!(refused(html(keep_comments()), raw), "{raw}");
    // Under an HTML element, HTML's rules stop at the element reading HTML.
    let raw = "<math><mtext><b></math></b><![CDATA[Note]]></mtext></math>";
    assert_eq!(rebuilt(raw), raw);
}

#[test]
fn a_mixed_slash_prefix_reaches_a_server() {
    assert_eq!(
        rebuilt(
            r#"<img src="/\example.test/p.png"><img src="\/example.test/p.png"><img src="/p.png" alt="ok">"#
        ),
        r#"<img><img><img src="/p.png" alt="ok">"#
    );
}

#[test]
fn every_image_set_candidate_is_checked() {
    for css in [
        "background:image-set(linear-gradient(red,blue) 1x, 'https://example.test/p.png' 2x)",
        "background:image-set('a)b.png' 1x, 'https://example.test/p.png' 2x)",
        r"background:image-set('a\'b)' 1x, 'https://example.test/p.png' 2x)",
        r#"background:image-set("a\"b)" 1x, "https://example.test/p.png" 2x)"#,
        r"background:image-set(a\)b 1x, 'https://example.test/p.png' 2x)",
    ] {
        assert!(css_loads_remote(css), "{css}");
    }
    assert!(!css_loads_remote(
        "background:image-set(linear-gradient(red,blue) 1x, 'p.png' 2x)"
    ));
}

/// A `&` left dangling in a removed style sheet does not dangle after it,
/// however long the markup written after the sheet grows the output.
#[test]
fn a_dangling_reference_goes_with_a_removed_style_sheet() {
    for lead in ["", "a"] {
        for pad in 0..80 {
            let title = format!("{lead}{}", "é".repeat(pad));
            let raw = format!(
                "<svg><style>@import \"https://x.test/a.css\";é&am</style><p title=\"{title}\">p;</p>"
            );
            let out = rebuilt(&raw);
            assert!(
                out.ends_with(&format!("<p title=\"{title}\">p;</p>")),
                "{out}"
            );
        }
    }
}

/// Text joined across many removals is copied in linear time.
#[test]
fn text_joined_across_many_removals_stays_linear() {
    let raw = format!("<p>&{}</p>", "ab<!---->".repeat(100_000));
    let started = std::time::Instant::now();
    let out = rebuilt(&raw);
    assert!(out.starts_with("<p>&amp;abab"), "{}", &out[..40]);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

/// In SVG or MathML, `</br>` and `</p>` leave them, as an HTML start tag
/// does, so what follows is HTML.
#[test]
fn a_br_or_p_end_tag_leaves_svg_and_mathml() {
    for raw in [
        "<svg></br><![CDATA[><img src=x onerror=alert(1)>]]>",
        "<math></br><![CDATA[><img src=x onerror=alert(1)>]]>",
        "<svg><g></p><![CDATA[><img src=x onerror=alert(1)>]]>",
    ] {
        assert!(refused(html(keep_comments()), raw), "{raw}");
    }
    assert_eq!(
        rebuilt("<svg><g></p><![CDATA[Jane]]><p>a</p>"),
        "<svg><g></p><p>a</p>"
    );
    // Right in an element reading HTML, they stay there.
    let raw = "<math><mtext></p><![CDATA[a > b]]></mtext></math>";
    assert_eq!(sanitize(html(keep_comments()), raw), raw);
}

/// An end tag for an element open in the one being left out closes it only
/// where a browser does: not past an element that keeps it open.
#[test]
fn an_inner_end_tag_held_open_does_not_close_its_element() {
    for raw in [
        "<div hidden><div><table><tr><td>a</div></div>secret",
        "<span hidden><span><div>a</span></span>secret",
        "<svg hidden><g><foreignObject><b></g></svg>secret",
    ] {
        let out = rebuilt(raw);
        assert!(!out.contains("secret"), "{raw}: {out}");
    }
    assert_eq!(
        rebuilt("<div hidden><div><p>a</div></div><p>Visible</p>"),
        "<p>Visible</p>"
    );
}

/// SVG and MathML elements are no scope boundary: an HTML end tag closes
/// them with what holds them.
#[test]
fn an_html_end_tag_closes_svg_in_a_removal() {
    assert_eq!(
        rebuilt("<div hidden><svg><g></div><p>Visible</p>"),
        "<p>Visible</p>"
    );
    // An element reading HTML inside them still is.
    let out = rebuilt("<div hidden><svg><desc></div>secret");
    assert!(!out.contains("secret"), "{out}");
}
