use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use themion_core::tools::{
    SourceExtractSymbolsResult, SourceExtractedSymbol, SourceOutlineEdge, SourceOutlineFile,
    SourceOutlineImport, SourceOutlineResult, SourceOutlineSymbol, SourceSymbolSpan,
};
use tree_sitter_language_pack::{
    detect_language_from_extension, detect_language_from_path, ImportInfo, ProcessConfig,
    StructureItem, StructureKind,
};

const MAX_SYMBOLS: usize = 500;
const MAX_IMPORTS: usize = 200;
const MAX_EDGES: usize = 1000;

pub(crate) fn handle_source_analysis_request(
    project_dir: &Path,
    action: &str,
    args: Value,
) -> Result<String> {
    match action {
        "source_outline" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing path"))?;
            let result = source_outline(project_dir, path)?;
            Ok(serde_json::to_string(&result)?)
        }
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
    let outline = source_outline(project_dir, path_arg)?;
    let symbols = outline
        .symbols
        .into_iter()
        .map(|symbol| SourceExtractedSymbol {
            name: symbol.name,
            kind: symbol.kind,
            parent_name: symbol.parent_name,
            span: symbol.span,
        })
        .collect();

    Ok(SourceExtractSymbolsResult {
        language: outline.language,
        path: outline.path,
        symbols,
        parse_error: outline.parse_error,
    })
}

fn source_outline(project_dir: &Path, path_arg: &str) -> Result<SourceOutlineResult> {
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

    let file = SourceOutlineFile {
        id: format!("file:{path_arg}"),
        kind: "file".to_string(),
        path: path_arg.to_string(),
    };
    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    flatten_structure_items(
        path_arg,
        &processed.structure,
        None,
        None,
        file.id.clone(),
        &mut symbols,
        &mut edges,
    );

    let mut imports = processed
        .imports
        .iter()
        .map(|import| outline_import(path_arg, import))
        .collect::<Vec<_>>();
    for import in &imports {
        edges.push(SourceOutlineEdge {
            from: file.id.clone(),
            to: import.id.clone(),
            relation: "imports".to_string(),
            confidence: "extracted".to_string(),
        });
    }

    let mut warnings = Vec::new();
    truncate_vec(&mut symbols, MAX_SYMBOLS, "symbols", &mut warnings);
    truncate_vec(&mut imports, MAX_IMPORTS, "imports", &mut warnings);
    let valid_ids = valid_node_ids(&file, &symbols, &imports);
    edges.retain(|edge| valid_ids.contains(&edge.from) && valid_ids.contains(&edge.to));
    truncate_vec(&mut edges, MAX_EDGES, "edges", &mut warnings);

    let parse_error = if processed.metrics.error_count > 0 {
        Some(format!(
            "parse reported {} error(s)",
            processed.metrics.error_count
        ))
    } else {
        None
    };

    Ok(SourceOutlineResult {
        language: processed.language,
        path: path_arg.to_string(),
        file,
        symbols,
        imports,
        edges,
        parse_error,
        warnings,
    })
}

fn flatten_structure_items(
    path: &str,
    items: &[StructureItem],
    parent_name: Option<String>,
    parent_id: Option<String>,
    container_id: String,
    symbols: &mut Vec<SourceOutlineSymbol>,
    edges: &mut Vec<SourceOutlineEdge>,
) {
    for item in items {
        let mut child_parent_name = parent_name.clone();
        let mut child_parent_id = parent_id.clone();
        let mut child_container_id = container_id.clone();

        if let Some(name_ref) = item.name.as_ref() {
            let name = name_ref.clone();
            let kind = normalize_kind(&item.kind);
            let span = symbol_span(&item.span);
            let id = symbol_id(path, &kind, &name, &span);
            symbols.push(SourceOutlineSymbol {
                id: id.clone(),
                name: name.clone(),
                kind,
                parent_id: parent_id.clone(),
                parent_name: parent_name.clone(),
                span,
            });
            edges.push(SourceOutlineEdge {
                from: container_id.clone(),
                to: id.clone(),
                relation: "contains".to_string(),
                confidence: "extracted".to_string(),
            });
            child_parent_name = Some(name_ref.clone());
            child_parent_id = Some(id.clone());
            child_container_id = id;
        }

        flatten_structure_items(
            path,
            &item.children,
            child_parent_name,
            child_parent_id,
            child_container_id,
            symbols,
            edges,
        );
    }
}

fn outline_import(path: &str, import: &ImportInfo) -> SourceOutlineImport {
    let span = symbol_span(&import.span);
    SourceOutlineImport {
        id: format!("import:{path}:{}:{}", span.start_line, span.start_byte),
        module: import.source.clone(),
        items: import.items.clone(),
        alias: import.alias.clone(),
        is_wildcard: import.is_wildcard,
        span,
        resolved: false,
        resolved_path: None,
    }
}

fn symbol_span(span: &tree_sitter_language_pack::Span) -> SourceSymbolSpan {
    SourceSymbolSpan {
        start_line: span.start_line,
        start_byte: span.start_byte,
        end_line: span.end_line,
        end_byte: span.end_byte,
    }
}

fn symbol_id(path: &str, kind: &str, name: &str, span: &SourceSymbolSpan) -> String {
    format!(
        "symbol:{path}:{kind}:{}:{}:{}",
        normalize_id_part(name),
        span.start_line,
        span.start_byte
    )
}

fn normalize_id_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            ':' | '\n' | '\r' | '\t' => '_',
            _ => ch,
        })
        .collect()
}

fn valid_node_ids(
    file: &SourceOutlineFile,
    symbols: &[SourceOutlineSymbol],
    imports: &[SourceOutlineImport],
) -> HashSet<String> {
    let mut ids = HashSet::new();
    ids.insert(file.id.clone());
    ids.extend(symbols.iter().map(|symbol| symbol.id.clone()));
    ids.extend(imports.iter().map(|import| import.id.clone()));
    ids
}

fn truncate_vec<T>(items: &mut Vec<T>, max: usize, label: &str, warnings: &mut Vec<String>) {
    if items.len() > max {
        items.truncate(max);
        warnings.push(format!("{label} truncated at {max}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempProject {
        path: PathBuf,
    }

    impl TempProject {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_temp_source(contents: &str) -> (TempProject, PathBuf) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "themion-source-outline-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&dir).expect("create temp dir");
        let path = dir.join("sample.rs");
        fs::write(&path, contents).expect("write temp source");
        (TempProject { path: dir }, PathBuf::from("sample.rs"))
    }

    #[test]
    fn outline_contains_file_symbols_imports_and_edges() {
        let (dir_guard, relative) = write_temp_source(
            r#"
use std::fs;

mod inner {
    pub fn nested() {}
}

fn top() {}
"#,
        );
        let project_dir = dir_guard.path();
        let outline = source_outline(project_dir, relative.to_str().unwrap()).expect("outline source");

        assert_eq!(outline.language, "rust");
        assert_eq!(outline.file.id, "file:sample.rs");
        assert!(outline.symbols.iter().any(|symbol| symbol.name == "top"));
        assert!(outline.imports.iter().any(|import| import.module.contains("std::fs")));

        let ids = valid_node_ids(&outline.file, &outline.symbols, &outline.imports);
        assert!(outline
            .edges
            .iter()
            .all(|edge| ids.contains(&edge.from) && ids.contains(&edge.to)));
    }

    #[test]
    fn legacy_symbols_project_from_outline() {
        let (dir_guard, relative) = write_temp_source("fn top() {}\n");
        let result = source_extract_symbols(dir_guard.path(), relative.to_str().unwrap())
            .expect("extract symbols");

        assert_eq!(result.language, "rust");
        assert!(result.symbols.iter().any(|symbol| symbol.name == "top"));
    }

    #[test]
    fn source_outline_omits_absent_optional_fields() {
        let (dir_guard, relative) = write_temp_source("fn top() {}\n");
        let outline = source_outline(dir_guard.path(), relative.to_str().unwrap())
            .expect("outline source");
        let value = serde_json::to_value(outline).expect("serialize outline");

        assert!(value.get("parse_error").is_none());
        let symbol = value["symbols"].as_array().unwrap().first().unwrap();
        assert!(symbol.get("parent_id").is_none());
        assert!(symbol.get("parent_name").is_none());
    }
}
