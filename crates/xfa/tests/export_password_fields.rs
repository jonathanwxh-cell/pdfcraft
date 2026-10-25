use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, Object, PdfString, SaveOptions, Stream, write_full};
use pdfcraft_xfa::{export_data, render_into};

fn xfa_shell(xdp: &str) -> Vec<u8> {
    let mut doc = Document::new_empty();
    let xfa_stream = doc.add(Object::Stream(Stream::from_raw(Dict::new(), xdp.as_bytes().to_vec())));
    let pages_root = doc.add(Object::Null);
    let page = doc.add(Object::Dict(
        [
            (b"Type".to_vec(), Object::name("Page")),
            (b"Parent".to_vec(), Object::Ref(pages_root)),
            (b"MediaBox".to_vec(), Object::Array([0.0, 0.0, 612.0, 792.0].into_iter().map(Object::Real).collect())),
        ]
        .into_iter()
        .collect(),
    ));
    doc.set(
        pages_root,
        Object::Dict(
            [
                (b"Type".to_vec(), Object::name("Pages")),
                (b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)])),
                (b"Count".to_vec(), Object::Int(1)),
            ]
            .into_iter()
            .collect(),
        ),
    );
    let acro: Dict = [(b"Fields".to_vec(), Object::Array(Vec::new())), (b"XFA".to_vec(), Object::Ref(xfa_stream))].into_iter().collect();
    let cat = doc.add(Object::Dict(
        [(b"Type".to_vec(), Object::name("Catalog")), (b"Pages".to_vec(), Object::Ref(pages_root)), (b"AcroForm".to_vec(), Object::Dict(acro))]
            .into_iter()
            .collect(),
    ));
    doc.trailer_mut().set(b"Root".to_vec(), Object::Ref(cat));
    write_full(&doc, &SaveOptions::default()).unwrap()
}

/// #3945: XFA XML and XDP data exports do not write typed `<passwordEdit/>` values in plain text;
/// fields with a template default export that default and fields without one export empty.
#[test]
fn xfa_export_data_omits_typed_password_edit_values() {
    let xdp = r#"<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/">
<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form" layout="tb">
 <pageSet><pageArea name="front"><contentArea x="0.5in" y="0.5in" w="7.5in" h="10in"/><medium short="8.5in" long="11in"/></pageArea></pageSet>
 <subform name="page1" layout="tb">
  <field name="user" w="2in" h="0.3in"><ui><textEdit/></ui></field>
  <field name="pin" w="2in" h="0.3in"><ui><passwordEdit/></ui></field>
  <field name="preset" w="2in" h="0.3in"><ui><passwordEdit/></ui><value><text>0000</text></value></field>
 </subform>
</subform>
</template>
<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/">
<xfa:data><form><page1><user>Ada</user><pin>hunter2</pin><preset>9999</preset></page1></form></xfa:data>
</xfa:datasets>
</xdp:xdp>"#;
    let mut doc = Document::open(Arc::new(xfa_shell(xdp))).unwrap();
    render_into(&mut doc).unwrap();

    for is_xdp in [false, true] {
        let exported = export_data(&doc, is_xdp, "form.pdf").unwrap();
        assert!(exported.contains("<user>Ada</user>"), "is_xdp={is_xdp}: {exported}");
        assert!(exported.contains("<preset>0000</preset>"), "is_xdp={is_xdp}: {exported}");
        assert!(!exported.contains("hunter2") && !exported.contains("9999"), "is_xdp={is_xdp} leaked password: {exported}");
    }
}

fn password_export_document(pin: &str, preset: &str) -> String {
    format!(
        r#"<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/">
<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
 <subform name="form"><field name="pin"><ui><passwordEdit/></ui></field>
 <field name="preset"><ui><passwordEdit/></ui><value><text>0000</text></value></field>
 <field name="user"><ui><textEdit/></ui></field></subform>
</template>
<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/">
 <xfa:data><form><pin>{pin}</pin><preset>{preset}</preset><user>Ada</user></form></xfa:data>
</xfa:datasets></xdp:xdp>"#
    )
}

#[test]
fn xfa_export_data_scans_the_same_wrapped_packet_it_exports() {
    let text = format!("<config/>{}", password_export_document("leaf-secret", "other-secret"));
    assert!(pdfcraft_xfa::data::parse_datasets(&text).is_none(), "the unwrapped scan cannot read this packet");
    let doc = Document::open(Arc::new(xfa_shell(&text))).unwrap();
    for xdp in [false, true] {
        let exported = export_data(&doc, xdp, "form.pdf").unwrap();
        assert!(!exported.contains("leaf-secret") && !exported.contains("other-secret"), "{exported}");
        assert!(exported.contains("<user>Ada</user>") && exported.contains("<preset>0000</preset>"), "{exported}");
        assert!(roxmltree::Document::parse(&exported).is_ok());
    }
    assert!(pdfcraft_xfa::read_packets(&doc).unwrap().unwrap().xdp.contains("leaf-secret"), "export does not mutate the open form");

    // Wrapping itself is not an error, including when no password field needs clearing.
    let ordinary = text.replace("passwordEdit", "textEdit");
    let doc = Document::open(Arc::new(xfa_shell(&ordinary))).unwrap();
    for xdp in [false, true] {
        assert!(export_data(&doc, xdp, "form.pdf").unwrap().contains("<pin>leaf-secret</pin>"));
    }
}

#[test]
fn xfa_export_data_rejects_partial_password_redaction() {
    for structured in ["<value>structured-secret</value>", "<other>structured-secret</other>", "<value>0000</value>"] {
        let text = password_export_document("leaf-secret", structured);
        let doc = Document::open(Arc::new(xfa_shell(&text))).unwrap();
        for xdp in [false, true] {
            assert!(export_data(&doc, xdp, "form.pdf").is_err(), "structured password must not survive a partial export");
        }
        let original = pdfcraft_xfa::read_packets(&doc).unwrap().unwrap().xdp;
        assert!(original.contains("leaf-secret") && original.contains(structured), "a refused export leaves the document unchanged");
    }
}

#[test]
fn xfa_export_data_rejects_unreadable_password_data() {
    let text = password_export_document("leaf-secret", "other-secret").replace("</xfa:data>", "</broken>");
    let doc = Document::open(Arc::new(xfa_shell(&text))).unwrap();
    for xdp in [false, true] {
        assert!(export_data(&doc, xdp, "form.pdf").is_err());
    }
}

#[test]
fn xfa_export_data_rejects_an_unreadable_password_template() {
    let text = password_export_document("leaf-secret", "other-secret")
        .replace("<subform name=\"form\">", "<area name=\"form\">")
        .replace("</subform>", "</area>");
    let doc = Document::open(Arc::new(xfa_shell(&text))).unwrap();
    for xdp in [false, true] {
        assert!(export_data(&doc, xdp, "form.pdf").is_err());
    }
}

#[test]
fn xfa_export_data_scans_wrapped_array_packets() {
    let text = password_export_document("leaf-secret", "other-secret");
    let parsed = roxmltree::Document::parse(&text).unwrap();
    let template = parsed.descendants().find(|n| n.has_tag_name(("http://www.xfa.org/schema/xfa-template/3.3/", "template"))).unwrap();
    let datasets = parsed.descendants().find(|n| n.tag_name().name() == "datasets").unwrap();
    let mut doc = Document::open(Arc::new(xfa_shell(&text))).unwrap();
    let template = doc.add(Object::Stream(Stream::from_raw(Dict::new(), text[template.range()].as_bytes().to_vec())));
    let data = format!("<config/>{}", &text[datasets.range()]);
    let data = doc.add(Object::Stream(Stream::from_raw(Dict::new(), data.into_bytes())));
    let root = doc.root().unwrap();
    let mut catalog = doc.get(root).as_dict().unwrap().clone();
    let mut acro = catalog.get(b"AcroForm").unwrap().as_dict().unwrap().clone();
    acro.set(
        b"XFA".to_vec(),
        Object::Array(vec![
            Object::String(PdfString::text("template")),
            Object::Ref(template),
            Object::String(PdfString::text("datasets")),
            Object::Ref(data),
        ]),
    );
    catalog.set(b"AcroForm".to_vec(), Object::Dict(acro));
    doc.set(root, Object::Dict(catalog));
    for xdp in [false, true] {
        let exported = export_data(&doc, xdp, "form.pdf").unwrap();
        assert!(!exported.contains("leaf-secret") && !exported.contains("other-secret"), "{exported}");
        assert!(exported.contains("<user>Ada</user>") && exported.contains("<preset>0000</preset>"), "{exported}");
    }
}
