use super::*;

const DOCUMENT: Root = Root::Element(W, "document");

/// `raw` rebuilt as a `root` part, checked to be a fixed point.
fn rebuilt(raw: &str, root: Root) -> (String, Vec<(HiddenContent, usize)>) {
    let Rebuilt { text, removed } = rebuild(raw, root)
        .expect("rebuild")
        .expect("the rebuild changes the part");
    assert!(
        rebuild(&text, root).expect("rebuild again").is_none(),
        "not a fixed point: {text}"
    );
    (text, removed.into_iter().collect())
}

fn word(body: &str) -> String {
    format!(
        r#"<w:document xmlns:w="{W}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:mc="{MC}" xmlns:v="urn:vendor:secret"><w:body>{body}</w:body></w:document>"#
    )
}

#[test]
fn a_schema_part_passes_unchanged() {
    let raw = word(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Jane Roe</w:t></w:r></w:p><w:sectPr/>"#,
    );
    let out = rebuild(&raw, DOCUMENT).unwrap();
    assert!(out.is_none(), "{out:?}");
}

#[test]
fn what_the_schema_does_not_name_goes() {
    let raw = word(
        r#"<w:p w14:paraId="1A2B" w:secret="x"><v:note>Jane</v:note><w:r><w:t>Hi</w:t><w:bogus><w:t>Roe</w:t></w:bogus></w:r></w:p>"#,
    );
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p><w:r><w:t>Hi</w:t></w:r></w:p>"));
    assert_eq!(removed, [(HiddenContent::ApplicationData, 4)]);
}

/// An element the schema names, but not under this parent, goes too.
#[test]
fn an_element_out_of_place_goes() {
    let raw = word(r#"<w:p><w:body><w:p><w:r><w:t>x</w:t></w:r></w:p></w:body></w:p>"#);
    let (text, _) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p></w:p>"));
}

#[test]
fn stray_text_goes() {
    let raw = word("<w:p>Jane<w:r><w:t>Hi &amp; bye</w:t></w:r>\n</w:p>");
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(
        text,
        word("<w:p><w:r><w:t>Hi &amp; bye</w:t></w:r>\n</w:p>")
    );
    assert_eq!(removed, [(HiddenContent::Undisplayed, 1)]);
}

#[test]
fn alternate_content_resolves_to_its_fallback() {
    let raw = word(
        r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="w14"><w14:secret>Jane</w14:secret></mc:Choice><mc:Fallback><w:t>Roe</w:t></mc:Fallback></mc:AlternateContent></w:r></w:p>"#,
    );
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p><w:r><w:t>Roe</w:t></w:r></w:p>"));
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);
}

#[test]
fn alternate_content_without_a_fallback_goes() {
    let raw = word(
        r#"<w:p><mc:AlternateContent><mc:Choice Requires="w14"><w:r><w:t>Jane</w:t></w:r></mc:Choice></mc:AlternateContent></w:p>"#,
    );
    let (text, _) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p></w:p>"));
}

#[test]
fn ignorable_markers_go() {
    let raw = format!(
        r#"<w:document xmlns:w="{W}" xmlns:mc="{MC}" mc:Ignorable="w14"><w:body/></w:document>"#
    );
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(
        text,
        format!(r#"<w:document xmlns:w="{W}" xmlns:mc="{MC}"><w:body/></w:document>"#)
    );
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);
}

/// An extension's payload is outside the schema, so the extension goes whole.
#[test]
fn an_extension_goes_whole() {
    let raw = format!(
        r#"<a:theme xmlns:a="{A}" name="T"><a:themeElements/><a:extLst><a:ext uri="{{05A4C25C}}"><thm15:themeFamily xmlns:thm15="urn:x" name="Jane"/></a:ext></a:extLst></a:theme>"#
    );
    let (text, removed) = rebuilt(&raw, Root::Element(A, "theme"));
    assert_eq!(
        text,
        format!(
            r#"<a:theme xmlns:a="{A}" name="T"><a:themeElements/><a:extLst></a:extLst></a:theme>"#
        )
    );
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);
}

#[test]
fn a_root_its_content_type_does_not_name_is_refused() {
    let raw = format!(r#"<w:styles xmlns:w="{W}"/>"#);
    assert!(rebuild(&raw, DOCUMENT).is_err());
    assert!(rebuild(r#"<doc/>"#, DOCUMENT).is_err());
}

#[test]
fn a_strict_part_reads_against_the_transitional_schema() {
    let raw = r#"<w:document xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main"><w:body><w:p><w:r><w:t>x</w:t></w:r><w:junk/></w:p></w:body></w:document>"#;
    let (text, _) = rebuilt(raw, DOCUMENT);
    assert!(!text.contains("junk"), "{text}");
    assert!(text.contains("<w:t>x</w:t>"), "{text}");
}

#[test]
fn a_vml_drawing_keeps_its_shapes() {
    let raw = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shape id="s" type="#t"><v:textbox><div style="text-align:left">Jane</div></v:textbox><x:ClientData ObjectType="Note"><x:Row>0</x:Row></x:ClientData></v:shape><s:x xmlns:s="urn:vendor"/></xml>"##;
    let (text, _) = rebuilt(raw, Root::Vml);
    assert!(text.contains("<x:Row>0</x:Row>"), "{text}");
    assert!(text.contains("<o:idmap"), "{text}");
    assert!(!text.contains("Jane"), "{text}");
    assert!(!text.contains("urn:vendor\"/>"), "{text}");
}

#[test]
fn processing_instructions_go() {
    let raw = word("<?app data?><w:p><w:r><w:t>Jane</w:t></w:r></w:p>");
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p><w:r><w:t>Jane</w:t></w:r></w:p>"));
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);
}

/// CDATA where text shows is written as the text it stands for; elsewhere it
/// goes.
#[test]
fn cdata_is_kept_as_text_where_text_shows() {
    let raw = word("<w:p><w:r><w:t><![CDATA[R&D <b>]]></w:t></w:r><![CDATA[Jane]]></w:p>");
    let (text, removed) = rebuilt(&raw, DOCUMENT);
    assert_eq!(
        text,
        word("<w:p><w:r><w:t>R&amp;D &lt;b&gt;</w:t></w:r></w:p>")
    );
    assert_eq!(removed, [(HiddenContent::Undisplayed, 1)]);
}

/// A namespace an unwrapped `mc:Fallback` declares is declared again on the
/// children it held, unless they declare it themselves.
#[test]
fn a_fallback_hands_its_namespaces_to_its_children() {
    let raw = format!(
        r#"<w:document xmlns:w="{W}" xmlns:mc="{MC}"><w:body><w:p><mc:AlternateContent xmlns:v="urn:schemas-microsoft-com:vml"><mc:Fallback xmlns:x="{W}"><x:r><x:t>Roe</x:t></x:r><x:r xmlns:v="urn:other"/></mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#
    );
    let (text, _) = rebuilt(&raw, DOCUMENT);
    assert_eq!(
        text,
        format!(
            r#"<w:document xmlns:w="{W}" xmlns:mc="{MC}"><w:body><w:p><x:r xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="{W}"><x:t>Roe</x:t></x:r><x:r xmlns:x="{W}" xmlns:v="urn:other"/></w:p></w:body></w:document>"#
        )
    );
}

/// `a:graphicData` keeps the payload its `uri` names, with its content, and
/// nothing else.
#[test]
fn a_graphic_keeps_the_payload_its_uri_names() {
    let frame = |uri: &str, payload: &str| {
        format!(
            r#"<p:sld xmlns:p="{P}" xmlns:a="{A}" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="t"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="{uri}">{payload}</a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
        )
    };
    let picture = r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="3" name="logo"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr/></pic:pic>"#;
    let table = r#"<a:tbl><a:tr h="1"><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>Jane</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl>"#;
    let vendor = r#"<s:x xmlns:s="urn:vendor"/>"#;
    let sld = Root::Element(P, "sld");

    let raw = frame(
        "http://schemas.openxmlformats.org/drawingml/2006/picture",
        &format!("{picture}{table}{vendor}"),
    );
    let (text, removed) = rebuilt(&raw, sld);
    assert_eq!(
        text,
        frame(
            "http://schemas.openxmlformats.org/drawingml/2006/picture",
            picture
        )
    );
    assert_eq!(removed, [(HiddenContent::ApplicationData, 2)]);

    // A reference in the `uri` names the same payload.
    let uri = "http://schemas.openxmlformats.org/drawingml/2006/pictur&#101;";
    let (text, removed) = rebuilt(&frame(uri, &format!("{picture}{table}")), sld);
    assert_eq!(text, frame(uri, picture));
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);

    let raw = frame("http://purl.oclc.org/ooxml/drawingml/table", table);
    assert!(rebuild(&raw, sld).unwrap().is_none());

    let raw = frame("urn:vendor", table);
    let (text, _) = rebuilt(&raw, sld);
    assert_eq!(text, frame("urn:vendor", ""));
}

/// A VML drawing's `<xml>` root keeps its namespace declarations alone.
#[test]
fn a_vml_root_keeps_only_its_declarations() {
    let raw =
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" extra="Jane"><v:shape id="s"/></xml>"#;
    let (text, removed) = rebuilt(raw, Root::Vml);
    assert_eq!(
        text,
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="s"/></xml>"#
    );
    assert_eq!(removed, [(HiddenContent::ApplicationData, 1)]);
}

/// A cut never joins text into a `]]>`, which XML forbids in text.
#[test]
fn text_stays_well_formed_across_a_cut() {
    let raw = word("<w:p><w:r><w:t>]]<?app x?>></w:t></w:r></w:p>");
    let (text, _) = rebuilt(&raw, DOCUMENT);
    assert_eq!(text, word("<w:p><w:r><w:t>]]&gt;</w:t></w:r></w:p>"));
}

/// Excel writes `xml:space` on a string's text, which its schema leaves off.
#[test]
fn spreadsheet_strings_keep_their_spacing() {
    let raw = format!(
        r#"<sst xmlns="{X}"><si><r><t xml:space="preserve">Jane </t></r><t xml:space="preserve"> Roe</t></si></sst>"#
    );
    assert!(rebuild(&raw, Root::Element(X, "sst")).unwrap().is_none());
}

#[test]
fn every_kept_content_type_names_a_root_the_schema_defines() {
    for content_type in [
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
        "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        "application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.volatileDependencies+xml",
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
        "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml",
        "application/vnd.openxmlformats-officedocument.theme+xml",
        "application/vnd.openxmlformats-officedocument.drawing+xml",
        "application/vnd.ms-word.commentsExtended+xml",
        "application/vnd.ms-word.commentsIds+xml",
        "application/vnd.ms-word.commentsExtensible+xml",
        "application/vnd.ms-word.people+xml",
    ] {
        let Some(Root::Element(namespace, local)) = root(content_type) else {
            panic!("{content_type} names no root");
        };
        let index = namespace_index(Some(namespace)).expect(content_type);
        assert!(
            ROOTS
                .binary_search_by(|Child(n, l, _)| (*n, *l).cmp(&(index, local)))
                .is_ok(),
            "{content_type}: {local}"
        );
    }
}
