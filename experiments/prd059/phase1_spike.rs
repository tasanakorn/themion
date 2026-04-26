#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! anyhow = "1"
//! fastembed = { version = "5.13.3", default-features = false, features = ["hf-hub-rustls-tls", "ort-download-binaries-rustls-tls"] }
//! rusqlite = { version = "0.31", features = ["bundled"] }
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! ```

use anyhow::{bail, Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const GLOBAL_PROJECT_DIR: &str = "[GLOBAL]";
const DEFAULT_TOP_K: usize = 5;
const DEFAULT_BATCH_SIZE: usize = 32;
const VECTOR_BLOB_DIM_MULTIPLE: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CorpusFile {
    nodes: Vec<CorpusNode>,
    queries: Vec<EvalQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CorpusNode {
    node_id: String,
    project_dir: String,
    node_type: String,
    title: String,
    content: Option<String>,
    hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalQuery {
    query_id: String,
    text: String,
    project_dir: String,
    #[serde(default)]
    filters: QueryFilters,
    #[serde(default, alias = "expected_node_ids")]
    relevant_node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
struct QueryFilters {
    node_type: Option<String>,
    hashtags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyEvalQuery {
    query_id: String,
    text: String,
    project_dir: String,
    #[serde(default, alias = "expected_node_ids")]
    relevant_node_ids: Vec<String>,
    #[serde(default)]
    hashtags: Vec<String>,
    #[serde(default)]
    hashtag_match: Option<String>,
    #[serde(default)]
    node_type: Option<String>,
}

impl<'de> Deserialize<'de> for QueryFilters {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum QueryFiltersWire {
            New {
                #[serde(default)]
                node_type: Option<String>,
                #[serde(default)]
                hashtags: Vec<String>,
            },
            Legacy {
                #[serde(default)]
                hashtags: Vec<String>,
                #[serde(default, rename = "hashtag_match")]
                _hashtag_match: Option<String>,
                #[serde(default)]
                node_type: Option<String>,
            },
        }

        let parsed = QueryFiltersWire::deserialize(deserializer)?;
        Ok(match parsed {
            QueryFiltersWire::New { node_type, hashtags } => Self { node_type, hashtags },
            QueryFiltersWire::Legacy {
                node_type,
                hashtags,
                _hashtag_match: _,
            } => Self { node_type, hashtags },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenarioSummary {
    name: String,
    semantic: StrategySummary,
    exact: StrategySummary,
    sqlite_bytes: u64,
    embedding_dimension: usize,
    node_count: usize,
    query_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrategySummary {
    recall_at_5: f32,
    mrr_at_5: f32,
    avg_query_ms: f32,
}

#[derive(Debug, Clone)]
struct EmbeddingRecord {
    node_id: String,
    vector: Vec<f32>,
}

#[derive(Debug, Clone)]
struct RetrievedNode {
    node_id: String,
    score: f32,
}

#[derive(Debug, Clone)]
struct StrategyMetrics {
    recall_sum: f32,
    reciprocal_rank_sum: f32,
    total_query_time_ms: f64,
    query_count: usize,
}

impl StrategyMetrics {
    fn summary(&self) -> StrategySummary {
        let denom = self.query_count.max(1) as f32;
        StrategySummary {
            recall_at_5: self.recall_sum / denom,
            mrr_at_5: self.reciprocal_rank_sum / denom,
            avg_query_ms: (self.total_query_time_ms as f32) / denom,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1))?;
    let artifact_dir = args
        .artifact_dir
        .unwrap_or_else(|| PathBuf::from("tmp/prd-059-phase1-artifacts"));
    fs::create_dir_all(&artifact_dir)
        .with_context(|| format!("create artifact dir {}", artifact_dir.display()))?;

    let corpus = load_corpus(&args.dataset)?;

    let mut summaries = Vec::new();
    summaries.push(run_scenario("project_only", &corpus, false, &artifact_dir)?);
    summaries.push(run_scenario("project_plus_global", &corpus, true, &artifact_dir)?);

    let report = serde_json::to_string_pretty(&summaries)?;
    let report_path = artifact_dir.join("summary.json");
    fs::write(&report_path, report).with_context(|| format!("write {}", report_path.display()))?;

    println!("wrote {}", report_path.display());
    for summary in summaries {
        println!(
            "scenario={} semantic(recall@5={:.3}, mrr@5={:.3}, avg_ms={:.2}) exact(recall@5={:.3}, mrr@5={:.3}, avg_ms={:.2}) sqlite_bytes={} dim={} nodes={} queries={}",
            summary.name,
            summary.semantic.recall_at_5,
            summary.semantic.mrr_at_5,
            summary.semantic.avg_query_ms,
            summary.exact.recall_at_5,
            summary.exact.mrr_at_5,
            summary.exact.avg_query_ms,
            summary.sqlite_bytes,
            summary.embedding_dimension,
            summary.node_count,
            summary.query_count,
        );
    }

    Ok(())
}

#[derive(Debug)]
struct Args {
    dataset: PathBuf,
    artifact_dir: Option<PathBuf>,
}

impl Args {
    fn parse<I>(mut args: I) -> Result<Self>
    where
        I: Iterator<Item = String>,
    {
        let mut dataset = PathBuf::from("experiments/prd059/fixtures/prd-059-semantic-search-corpus.json");
        let mut artifact_dir = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dataset" => dataset = PathBuf::from(next_arg(&mut args, "--dataset")?),
                "--artifact-dir" => {
                    artifact_dir = Some(PathBuf::from(next_arg(&mut args, "--artifact-dir")?))
                }
                "--help" | "-h" => {
                    println!(
                        "Usage: prd059_phase1_spike.rs [--dataset PATH] [--artifact-dir PATH]"
                    );
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }

        Ok(Self {
            dataset,
            artifact_dir,
        })
    }
}

fn next_arg<I>(args: &mut I, flag: &str) -> Result<String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))
}

fn parse_embedding_model(raw: &str) -> Result<EmbeddingModel> {
    match raw {
        "BGESmallENV15" => Ok(EmbeddingModel::BGESmallENV15),
        "BGESmallENV15Q" => Ok(EmbeddingModel::BGESmallENV15Q),
        "BGEM3" | "BGE-M3" => Ok(EmbeddingModel::BGEM3),
        "BGESmallZHV15" | "BGE-Micro-v2" => Ok(EmbeddingModel::BGESmallZHV15),
        other => bail!("unsupported PRD059_EMBEDDING_MODEL: {other}"),
    }
}

fn load_corpus(path: &Path) -> Result<CorpusFile> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if let Ok(parsed) = serde_json::from_str::<CorpusFile>(&raw) {
        return Ok(parsed);
    }

    #[derive(Deserialize)]
    struct LegacyCorpusFile {
        nodes: Vec<CorpusNode>,
        queries: Vec<LegacyEvalQuery>,
    }

    let legacy: LegacyCorpusFile =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(CorpusFile {
        nodes: legacy.nodes,
        queries: legacy
            .queries
            .into_iter()
            .map(|query| EvalQuery {
                query_id: query.query_id,
                text: query.text,
                project_dir: query.project_dir,
                filters: QueryFilters {
                    node_type: query.node_type,
                    hashtags: query.hashtags,
                },
                relevant_node_ids: query.relevant_node_ids,
            })
            .collect(),
    })
}

fn run_scenario(
    name: &str,
    corpus: &CorpusFile,
    include_global: bool,
    artifact_dir: &Path,
) -> Result<ScenarioSummary> {
    let scenario_dir = artifact_dir.join(name);
    fs::create_dir_all(&scenario_dir)?;
    let db_path = scenario_dir.join("memory.sqlite");
    let cache_dir = scenario_dir.join("model-cache");

    if db_path.exists() {
        fs::remove_file(&db_path).with_context(|| format!("remove {}", db_path.display()))?;
    }
    fs::create_dir_all(&cache_dir).with_context(|| format!("create {}", cache_dir.display()))?;

    let mut conn = Connection::open(&db_path)?;
    create_schema(&conn)?;

    let nodes = select_nodes(corpus, include_global);
    let documents: Vec<String> = nodes.iter().map(node_document).collect();

    let selected_model = std::env::var("PRD059_EMBEDDING_MODEL")
        .ok()
        .as_deref()
        .map(parse_embedding_model)
        .transpose()?
        .unwrap_or(EmbeddingModel::BGESmallENV15);

    let mut model = TextEmbedding::try_new(
        InitOptions::new(selected_model).with_cache_dir(cache_dir.clone()),
    )?;

    let node_embeddings = model.embed(documents, Some(DEFAULT_BATCH_SIZE))?;
    persist_nodes(&mut conn, &nodes, &node_embeddings)?;

    let mut semantic_metrics = StrategyMetrics {
        recall_sum: 0.0,
        reciprocal_rank_sum: 0.0,
        total_query_time_ms: 0.0,
        query_count: 0,
    };
    let mut exact_metrics = semantic_metrics.clone();

    for query in &corpus.queries {
        let relevant_pool = scenario_relevant_ids(query, &nodes);
        if relevant_pool.is_empty() {
            continue;
        }

        let semantic_start = Instant::now();
        let semantic_hits = semantic_search(&conn, &mut model, query, include_global)?;
        semantic_metrics.total_query_time_ms += semantic_start.elapsed().as_secs_f64() * 1000.0;
        update_metrics(&mut semantic_metrics, &semantic_hits, &relevant_pool);

        let exact_start = Instant::now();
        let exact_hits = exact_search(&conn, query, include_global)?;
        exact_metrics.total_query_time_ms += exact_start.elapsed().as_secs_f64() * 1000.0;
        update_metrics(&mut exact_metrics, &exact_hits, &relevant_pool);
    }

    let embedding_dimension = node_embeddings.first().map(|v| v.len()).unwrap_or(0);
    let sqlite_bytes = fs::metadata(&db_path)
        .with_context(|| format!("metadata {}", db_path.display()))?
        .len();

    Ok(ScenarioSummary {
        name: name.to_string(),
        semantic: semantic_metrics.summary(),
        exact: exact_metrics.summary(),
        sqlite_bytes,
        embedding_dimension,
        node_count: nodes.len(),
        query_count: semantic_metrics.query_count,
    })
}

fn select_nodes<'a>(corpus: &'a CorpusFile, include_global: bool) -> Vec<&'a CorpusNode> {
    corpus
        .nodes
        .iter()
        .filter(|node| include_global || node.project_dir != GLOBAL_PROJECT_DIR)
        .collect()
}

fn node_document(node: &&CorpusNode) -> String {
    let hashtags = if node.hashtags.is_empty() {
        String::new()
    } else {
        format!(" hashtags:{}", node.hashtags.join(" "))
    };
    format!(
        "title: {}\ntype: {}\nproject: {}\ncontent: {}{}",
        node.title,
        node.node_type,
        node.project_dir,
        node.content.clone().unwrap_or_default(),
        hashtags
    )
}

fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE memory_nodes (
            node_id TEXT PRIMARY KEY,
            project_dir TEXT NOT NULL,
            node_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT,
            hashtags_json TEXT NOT NULL
        );
        CREATE TABLE memory_node_embeddings (
            node_id TEXT PRIMARY KEY REFERENCES memory_nodes(node_id) ON DELETE CASCADE,
            embedding BLOB NOT NULL,
            dimension INTEGER NOT NULL
        );
        CREATE INDEX idx_memory_nodes_project_dir ON memory_nodes(project_dir);
        CREATE INDEX idx_memory_nodes_node_type ON memory_nodes(node_type);
        "
    )?;
    Ok(())
}

fn persist_nodes(
    conn: &mut Connection,
    nodes: &[&CorpusNode],
    node_embeddings: &[Vec<f32>],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut insert_node = tx.prepare(
            "INSERT INTO memory_nodes (node_id, project_dir, node_type, title, content, hashtags_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut insert_embedding = tx.prepare(
            "INSERT INTO memory_node_embeddings (node_id, embedding, dimension)
             VALUES (?1, ?2, ?3)",
        )?;

        for (node, vector) in nodes.iter().zip(node_embeddings.iter()) {
            insert_node.execute(params![
                node.node_id,
                node.project_dir,
                node.node_type,
                node.title,
                node.content,
                serde_json::to_string(&node.hashtags)?
            ])?;
            insert_embedding.execute(params![
                node.node_id,
                vector_to_blob(vector),
                vector.len() as i64
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * VECTOR_BLOB_DIM_MULTIPLE);
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

fn blob_to_vector(blob: &[u8], dimension: usize) -> Result<Vec<f32>> {
    if blob.len() != dimension * VECTOR_BLOB_DIM_MULTIPLE {
        bail!(
            "embedding blob length {} does not match dimension {}",
            blob.len(),
            dimension
        );
    }
    let mut vector = Vec::with_capacity(dimension);
    for chunk in blob.chunks_exact(VECTOR_BLOB_DIM_MULTIPLE) {
        vector.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(vector)
}

fn semantic_search(
    conn: &Connection,
    model: &mut TextEmbedding,
    query: &EvalQuery,
    include_global: bool,
) -> Result<Vec<RetrievedNode>> {
    let query_embedding = model
        .embed(vec![query.text.clone()], Some(1))?
        .into_iter()
        .next()
        .context("missing query embedding")?;

    let candidates = load_embeddings(conn, query, include_global)?;
    let mut scored = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let score = cosine_similarity(&query_embedding, &candidate.vector);
        scored.push(RetrievedNode {
            node_id: candidate.node_id,
            score,
        });
    }
    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    scored.truncate(DEFAULT_TOP_K);
    Ok(scored)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;

    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }

    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn exact_search(conn: &Connection, query: &EvalQuery, include_global: bool) -> Result<Vec<RetrievedNode>> {
    let mut scored = Vec::new();
    let query_lower = query.text.to_lowercase();
    let candidates = load_text_candidates(conn, query, include_global)?;

    for (node_id, title, content, hashtags) in candidates {
        let mut score = 0.0f32;
        if title.to_lowercase().contains(&query_lower) {
            score += 3.0;
        }
        if content.to_lowercase().contains(&query_lower) {
            score += 2.0;
        }
        for token in query_lower.split_whitespace() {
            if title.to_lowercase().contains(token) {
                score += 0.4;
            }
            if content.to_lowercase().contains(token) {
                score += 0.2;
            }
            if hashtags.iter().any(|tag| tag.to_lowercase() == token) {
                score += 0.6;
            }
        }
        if score > 0.0 {
            scored.push(RetrievedNode { node_id, score });
        }
    }

    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    scored.truncate(DEFAULT_TOP_K);
    Ok(scored)
}

fn update_metrics(metrics: &mut StrategyMetrics, hits: &[RetrievedNode], relevant: &[String]) {
    let recall = hits
        .iter()
        .filter(|hit| relevant.contains(&hit.node_id))
        .count() as f32
        / relevant.len() as f32;
    metrics.recall_sum += recall;

    let rr = hits
        .iter()
        .position(|hit| relevant.contains(&hit.node_id))
        .map(|index| 1.0 / (index as f32 + 1.0))
        .unwrap_or(0.0);
    metrics.reciprocal_rank_sum += rr;
    metrics.query_count += 1;
}

fn scenario_relevant_ids(query: &EvalQuery, nodes: &[&CorpusNode]) -> Vec<String> {
    query
        .relevant_node_ids
        .iter()
        .filter(|node_id| nodes.iter().any(|node| &node.node_id == *node_id))
        .cloned()
        .collect()
}

fn load_embeddings(
    conn: &Connection,
    query: &EvalQuery,
    include_global: bool,
) -> Result<Vec<EmbeddingRecord>> {
    let mut sql = String::from(
        "SELECT n.node_id, e.embedding, e.dimension
         FROM memory_nodes n
         JOIN memory_node_embeddings e USING(node_id)
         WHERE 1=1",
    );

    if include_global {
        sql.push_str(" AND (n.project_dir = ?1 OR n.project_dir = ?2)");
    } else {
        sql.push_str(" AND n.project_dir = ?1");
    }
    if query.filters.node_type.is_some() {
        sql.push_str(" AND n.node_type = ?3");
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, Vec<u8>, i64)> = if include_global {
        if let Some(node_type) = query.filters.node_type.as_deref() {
            stmt.query_map(params![query.project_dir, GLOBAL_PROJECT_DIR, node_type], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![query.project_dir, GLOBAL_PROJECT_DIR], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        }
    } else if let Some(node_type) = query.filters.node_type.as_deref() {
        stmt.query_map(params![query.project_dir, node_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(params![query.project_dir], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut embeddings = Vec::with_capacity(rows.len());
    for (node_id, blob, dimension) in rows {
        embeddings.push(EmbeddingRecord {
            node_id,
            vector: blob_to_vector(&blob, dimension as usize)?,
        });
    }

    if !query.filters.hashtags.is_empty() {
        embeddings.retain(|record| {
            query
                .filters
                .hashtags
                .iter()
                .all(|needle| record.node_id.contains(needle) || needle.starts_with('#'))
        });
    }

    Ok(embeddings)
}

fn load_text_candidates(
    conn: &Connection,
    query: &EvalQuery,
    include_global: bool,
) -> Result<Vec<(String, String, String, Vec<String>)>> {
    let mut sql = String::from(
        "SELECT node_id, title, COALESCE(content, ''), hashtags_json
         FROM memory_nodes
         WHERE 1=1",
    );

    if include_global {
        sql.push_str(" AND (project_dir = ?1 OR project_dir = ?2)");
    } else {
        sql.push_str(" AND project_dir = ?1");
    }
    if query.filters.node_type.is_some() {
        sql.push_str(" AND node_type = ?3");
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, String, String, String)> = if include_global {
        if let Some(node_type) = query.filters.node_type.as_deref() {
            stmt.query_map(params![query.project_dir, GLOBAL_PROJECT_DIR, node_type], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![query.project_dir, GLOBAL_PROJECT_DIR], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        }
    } else if let Some(node_type) = query.filters.node_type.as_deref() {
        stmt.query_map(params![query.project_dir, node_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(params![query.project_dir], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut candidates = Vec::new();
    for (node_id, title, content, hashtags_json) in rows {
        let hashtags: Vec<String> = serde_json::from_str(&hashtags_json)?;
        if query.filters.hashtags.is_empty()
            || query
                .filters
                .hashtags
                .iter()
                .all(|needle| hashtags.iter().any(|tag| tag == needle))
        {
            candidates.push((node_id, title, content, hashtags));
        }
    }
    Ok(candidates)
}
