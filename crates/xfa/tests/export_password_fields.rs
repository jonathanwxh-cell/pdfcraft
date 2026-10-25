use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, Object, SaveOptions, Stream, write_full};
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
