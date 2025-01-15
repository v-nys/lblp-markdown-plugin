use anyhow;
use base64::encode;
use comrak::{nodes::NodeValue, parse_document, Arena, ComrakOptions};
use extism_pdk::*;
use logic_based_learning_paths::domain_without_loading::{
    BoolPayload, ClusterProcessingPayload, ClusterProcessingResult, DirectoryStructurePayload,
    DummyPayload, FileEntry, FileReadBase64OperationInPayload, FileReadBase64OperationOutPayload,
    FileReadOperationInPayload, FileReadOperationOutPayload, FileWriteOperationPayload,
    ParamsSchema, SystemTimePayload,
};
use regex::Regex;
use scraper::{ElementRef, Html, Node};
use serde_json;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;

#[host_fn]
extern "ExtismHost" {
    fn get_system_time() -> SystemTimePayload;
    fn get_last_modification_time(relative_path: String) -> SystemTimePayload;
    fn write_text_file(payload: FileWriteOperationPayload) -> ();
    fn read_text_file(payload: FileReadOperationInPayload) -> FileReadOperationOutPayload;
    fn read_binary_file_base64(
        payload: FileReadBase64OperationInPayload,
    ) -> FileReadBase64OperationOutPayload;
    fn get_cluster_structure(payload: DummyPayload) -> DirectoryStructurePayload;
}

#[plugin_fn]
pub fn get_params_schema(_: ()) -> FnResult<ParamsSchema> {
    let mut parameters = HashMap::new();
    let string_schema = schemars::schema_for!(String);
    let value = serde_json::to_value(string_schema).expect("Should be convertible.");
    parameters.insert("input_extension".into(), (true, value.clone()));
    parameters.insert("output_extension".into(), (true, value));
    let boolean_schema = schemars::schema_for!(bool);
    let value = serde_json::to_value(boolean_schema).expect("Should be convertible.");
    parameters.insert("include_artifact_mapping".into(), (true, value));
    Ok(ParamsSchema { schema: parameters })
}

#[plugin_fn]
pub fn process_cluster(cpp: ClusterProcessingPayload) -> FnResult<ClusterProcessingResult> {
    let artifacts = HashSet::new();
    let input_extension = cpp
        .parameter_values
        .get("input_extension")
        .expect("Missing expected argument for parameter input_extension.")
        .as_str()
        .expect("Should be a string, as specified by the schema.");
    let output_extension = cpp
        .parameter_values
        .get("output_extension")
        .expect("Missing expected argument for parameter input_extension.")
        .as_str()
        .expect("Should be a string, as specified by the schema.");
    let include_artifact_mapping = cpp
        .parameter_values
        .get("include_artifact_mapping")
        .expect("Missing expected argument for parameter include_artifact_mapping.")
        .as_bool()
        .expect("Should be a bool, as specified by the schema.");
    let DirectoryStructurePayload { entries } =
        (unsafe { get_cluster_structure(DummyPayload {}) }).expect("Thought this would be fine.");
    entries.iter().for_each(|e| {
        let e_path = PathBuf::from_str(&e.relative_path)
            .expect("This was originally a path, so should be able to convert back.");
        if !e.is_dir && e.relative_path.ends_with(input_extension) {
            // TODO only do this if the existing generated file is older than the source file
            let string_rep = read_markdown_to_html_with_inlined_images(
                &PathBuf::from_str(&e.relative_path)
                    .expect("Building a PathBuf from str should work here."),
            );
            let payload = FileWriteOperationPayload {
                relative_path: e_path
                    .with_extension(output_extension)
                    .to_string_lossy()
                    .to_string(),
                // FIXME: don't expect
                contents: string_rep.expect("Failed to convert Markdown."),
            };
            unsafe { write_text_file(payload) }.expect("Invoking this host method should be fine.");
        }
    });
    // should include mapping for converted files iff this plugin is meant as "terminator"
    // i.e. if further processing of HTML is expected, don't include
    Ok(ClusterProcessingResult {
        hash_set: if include_artifact_mapping {
            artifacts
        } else {
            HashSet::new()
        },
    })
}

fn normalize_whitespace(text: &str) -> String {
    let re = Regex::new(r"\s+").unwrap();
    re.replace_all(text, " ").to_string()
}

fn recurse(node: ego_tree::NodeRef<Node>, new_html: &mut String) {
    match node.value() {
        Node::Doctype(doctype) => {
            new_html.push_str(&format!("<!doctype {}>", doctype.name()));
        }
        Node::Comment(_) => {}
        Node::Element(elem) => {
            let elem_ref = ElementRef::wrap(node).expect("Specifically works in this case.");
            let tag = elem_ref.value().name();
            match tag {
                "pre" | "code" | "textarea" | "svg" => new_html.push_str(&elem_ref.html()),
                _ => {
                    new_html.push_str(&format!("<{}", tag));
                    // attributes include classes!
                    for (attr_name, attr_value) in elem.attrs() {
                        new_html.push_str(&format!(" {attr_name}=\"{attr_value}\""));
                    }
                    new_html.push_str(">");
                    for node in elem_ref.children() {
                        recurse(node, new_html);
                    }
                    new_html.push_str(&format!("</{}>", tag));
                }
            }
        }
        Node::Text(text) => {
            new_html.push_str(&normalize_whitespace(&text.to_string()));
        }
        Node::Document => {}
        Node::Fragment | Node::ProcessingInstruction(_) => {
            unimplemented!("these nodes are not supported");
        }
    }
}

fn read_markdown_to_html_with_inlined_images(md_path: &PathBuf) -> anyhow::Result<String> {
    let protocol_re = regex::Regex::new(r#"[A-Za-z]+://.+"#)
        .expect("This regex has been tested. It won't fail to compile.");
    let FileReadOperationOutPayload { contents: markdown } = unsafe {
        read_text_file(FileReadOperationInPayload {
            relative_path: md_path.to_string_lossy().to_string(),
        })
    }?;
    let arena = Arena::new();
    let mut comrak_options = ComrakOptions::default();
    comrak_options.extension.table = true;
    let root = parse_document(&arena, &markdown, &comrak_options);
    let mut scrubbed = vec![];
    for node in root.descendants() {
        // in this case, img tag will have to be removed entirely
        // that entails some extra work later
        let mut is_relative_svg = false;
        // this needs to be in this scope because it is also needed for svg
        let mut img_path = PathBuf::new();
        if let NodeValue::Image(ref mut link) = node.data.borrow_mut().value {
            // see https://docs.rs/comrak/0.26.0/comrak/nodes/struct.NodeLink.html
            let existing_url = &link.url.clone();
            if !protocol_re.is_match(existing_url) {
                if existing_url.contains("\\") {
                    Err(anyhow::anyhow!(format!(
                        "Path {} contains backslash. Use forward slash, even on Windows.",
                        existing_url
                    )))?
                } else {
                    let url_path = std::path::PathBuf::from_str(existing_url)?;
                    if url_path.is_absolute() {
                        Err(anyhow::anyhow!(format!(
                            "Path {} is absolute. For portability reasons, this is not allowed.",
                            existing_url
                        )))?
                    } else {
                        img_path = md_path.with_file_name(&url_path);
                        let ext = img_path
                            .extension()
                            .and_then(std::ffi::OsStr::to_str)
                            .ok_or(anyhow::anyhow!(
                                "Image lacks an extension: {}",
                                img_path.to_string_lossy()
                            ))?;
                        if ext == "svg" {
                            is_relative_svg = true;
                        } else {
                            let mime_type = match ext {
                                "jpg" | "jpeg" => "image/jpeg",
                                "gif" => "image/gif",
                                "png" => "image/png",
                                "webp" => "image/webp",
                                _ => Err(anyhow::anyhow!(
                                    "Unsupported extension for {}",
                                    img_path.to_string_lossy()
                                ))?,
                            };
                            let FileReadBase64OperationOutPayload { contents: base64 } = unsafe {
                                read_binary_file_base64(FileReadBase64OperationInPayload {
                                    relative_path: img_path.to_string_lossy().to_string(),
                                })
                            }?;
                            link.url = format!(r#"data:{};base64,{}"#, mime_type, base64)
                        }
                    }
                }
            }
        }
        if is_relative_svg {
            node.children().for_each(|child| {
                // remove "alt text" (which would just appear inline)
                // should not invoke child.detach() here
                // think this messes up traversal order
                // tests show that second SVG is not inlined if we do this
                scrubbed.push(child);
            });
            let mut to_be_replaced = node.data.borrow_mut();
            let FileReadOperationOutPayload {
                contents: svg_contents,
            } = unsafe {
                read_text_file(FileReadOperationInPayload {
                    relative_path: img_path.to_string_lossy().to_string(),
                })
            }?;
            let actual_svg_start = svg_contents
                .find("<svg")
                .ok_or(anyhow::anyhow!("Could not find svg tag in svg file."))?;
            let (_doctypestuff, actual_svg) = svg_contents.split_at(actual_svg_start);
            to_be_replaced.value = NodeValue::HtmlInline(actual_svg.into());
        }
    }
    scrubbed.into_iter().for_each(|scrubbed| {
        scrubbed.detach();
    });
    let mut html = vec![];
    let mut render_options = comrak::Options::default();
    // needed to render inline SVGs, as there is no element for that
    render_options.render.unsafe_ = true;
    render_options.extension.table = true;
    comrak::format_html(root, &render_options, &mut html)?;
    String::from_utf8(html)
        .map(|s| {
            let document = Html::parse_document(&s);
            let mut new_html = String::new();
            for node in document.tree.root().children() {
                recurse(node, &mut new_html);
            }
            new_html
        })
        .map_err(|_| anyhow::anyhow!("Encoding error".to_owned()))
}
