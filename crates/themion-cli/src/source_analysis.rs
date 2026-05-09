use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;
use themion_core::tools::{SourceExtractSymbolsResult, SourceExtractedSymbol, SourceSymbolSpan};
use tree_sitter_language_pack::{
    detect_language_from_extension, detect_language_from_path, ProcessConfig, StructureItem,
    StructureKind,
};

pub(crate) fn handle_source_analysis_request(
    project_dir: &Path,
    action: &str,
    args: Value,
) -> Result<String> {
    match action {
        "source_extract_symbols" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let result = source_extract_symbols(project_dir, path)?;
            Ok(serde_json::to_string(&result)?)
        }
        other => Err(anyhow::anyhow!("unknown source analysis action: {other}")),
    }
}

fn source_extract_symbols(
    project_dir: &Path,
    path_arg: &str,
) -> Result<SourceExtractSymbolsResult> {
    let path = project_dir.join(path_arg);
    let source = fs::read_to_string(&path)
        .with_context(|| format!("read source file {}", path.display()))?;
    let detected_language = detect_language_from_path(path_arg)
        .or_else(|| {
            Path::new(path_arg)
                .extension()
                .and_then(|ext| ext.to_str())
                .and_then(detect_language_from_extension)
        })
        .ok_or_else(|| anyhow::anyhow!("could not detect language from path: {path_arg}"))?;

    let config = ProcessConfig::new(detected_language);
    let processed = tree_sitter_language_pack::process(&source, &config).map_err(|err| {
        anyhow::anyhow!("process {} with tree-sitter-language-pack: {err}", path_arg)
    })?;

    let mut symbols = Vec::new();
    flatten_structure_items(&processed.structure, None, &mut symbols);

    let parse_error = if processed.metrics.error_count > 0 {
        Some(format!(
            "parse reported {} error(s)",
            processed.metrics.error_count
        ))
    } else {
        None
    };

    Ok(SourceExtractSymbolsResult {
        language: processed.language,
        path: path_arg.to_string(),
        symbols,
        parse_error,
    })
}

fn flatten_structure_items(
    items: &[StructureItem],
    parent_name: Option<&str>,
    out: &mut Vec<SourceExtractedSymbol>,
) {
    for item in items {
        if let Some(name_ref) = item.name.as_ref() {
            let name = name_ref.clone();
            out.push(SourceExtractedSymbol {
                name,
                kind: normalize_kind(&item.kind),
                parent_name: parent_name.map(str::to_string),
                span: SourceSymbolSpan {
                    start_line: item.span.start_line,
                    start_byte: item.span.start_byte,
                    end_line: item.span.end_line,
                    end_byte: item.span.end_byte,
                },
            });
        }
        flatten_structure_items(&item.children, item.name.as_deref().or(parent_name), out);
    }
}

fn normalize_kind(kind: &StructureKind) -> String {
    match kind {
        StructureKind::Function => "function",
        StructureKind::Method => "method",
        StructureKind::Class => "class",
        StructureKind::Struct => "struct",
        StructureKind::Interface => "interface",
        StructureKind::Enum => "enum",
        StructureKind::Module => "module",
        StructureKind::Trait => "trait",
        StructureKind::Impl => "impl",
        StructureKind::Namespace => "module",
        StructureKind::Other(value) => value.as_str(),
    }
    .to_string()
}
