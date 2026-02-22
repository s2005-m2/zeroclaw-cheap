#![cfg(feature = "memory-lancedb")]

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use chrono::Utc;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Table};
use uuid::Uuid;

use super::embeddings::EmbeddingProvider;
use super::traits::{Memory, MemoryCategory, MemoryEntry};

pub struct LanceDbMemory {
    table: Table,
    embedder: Arc<dyn EmbeddingProvider>,
    vector_weight: f32,
    keyword_weight: f32,
    dims: usize,
}

impl LanceDbMemory {
    pub async fn new(
        workspace_dir: &Path,
        embedder: Arc<dyn EmbeddingProvider>,
        vector_weight: f32,
        keyword_weight: f32,
    ) -> Result<Self> {
        let db_path = workspace_dir.join(".zeroclaw").join("lancedb");
        let db_path_str = db_path.to_string_lossy();
        let conn = connect(&*db_path_str).execute().await?;

        let dims = embedder.dimensions();
        let table = Self::open_or_create_table(&conn, dims).await?;

        Ok(Self {
            table,
            embedder,
            vector_weight,
            keyword_weight,
            dims,
        })
    }

    async fn open_or_create_table(conn: &lancedb::Connection, dims: usize) -> Result<Table> {
        let table_name = "memories";
        let names = conn.table_names().execute().await?;
        if names.contains(&table_name.to_string()) {
            return Ok(conn.open_table(table_name).execute().await?);
        }

        let schema = Arc::new(build_schema(dims));
        let empty: Vec<RecordBatch> = vec![];
        let reader = RecordBatchIterator::new(empty.into_iter().map(Ok), schema);
        Ok(conn
            .create_table(table_name, Box::new(reader))
            .execute()
            .await?)
    }
}

fn build_schema(dims: usize) -> Schema {
    let mut fields = vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("key", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("timestamp", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, true),
    ];
    if dims > 0 {
        let vec_field = Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dims as i32,
            ),
            true,
        );
        fields.push(vec_field);
    }
    Schema::new(fields)
}

fn batch_from_entry(
    id: &str,
    key: &str,
    content: &str,
    category: &str,
    timestamp: &str,
    session_id: Option<&str>,
    vector: Option<&[f32]>,
    dims: usize,
) -> Result<RecordBatch> {
    let schema = Arc::new(build_schema(dims));

    let id_arr = Arc::new(StringArray::from(vec![id]));
    let key_arr = Arc::new(StringArray::from(vec![key]));
    let content_arr = Arc::new(StringArray::from(vec![content]));
    let cat_arr = Arc::new(StringArray::from(vec![category]));
    let ts_arr = Arc::new(StringArray::from(vec![timestamp]));
    let sid_arr = Arc::new(StringArray::from(vec![session_id]));

    let mut columns: Vec<Arc<dyn Array>> =
        vec![id_arr, key_arr, content_arr, cat_arr, ts_arr, sid_arr];

    if dims > 0 {
        let vec_arr: Arc<dyn Array> = if let Some(v) = vector {
            let values = Arc::new(Float32Array::from(v.to_vec()));
            Arc::new(FixedSizeListArray::new(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dims as i32,
                values,
                None,
            ))
        } else {
            Arc::new(FixedSizeListArray::new_null(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dims as i32,
                1,
            ))
        };
        columns.push(vec_arr);
    }

    Ok(RecordBatch::try_new(schema, columns)?)
}

fn entry_from_batch(batch: &RecordBatch, score: Option<f64>) -> Vec<MemoryEntry> {
    let n = batch.num_rows();
    let mut out = Vec::with_capacity(n);

    let get_str = |col: &str| -> Vec<Option<String>> {
        batch
            .column_by_name(col)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .map(|a| {
                (0..n)
                    .map(|i| {
                        if a.is_null(i) {
                            None
                        } else {
                            Some(a.value(i).to_string())
                        }
                    })
                    .collect()
            })
            .unwrap_or_else(|| vec![None; n])
    };

    let ids = get_str("id");
    let keys = get_str("key");
    let contents = get_str("content");
    let categories = get_str("category");
    let timestamps = get_str("timestamp");
    let session_ids = get_str("session_id");

    for i in 0..n {
        let category = match categories[i].as_deref().unwrap_or("core") {
            "core" => MemoryCategory::Core,
            "daily" => MemoryCategory::Daily,
            "conversation" => MemoryCategory::Conversation,
            other => MemoryCategory::Custom(other.to_string()),
        };
        out.push(MemoryEntry {
            id: ids[i].clone().unwrap_or_default(),
            key: keys[i].clone().unwrap_or_default(),
            content: contents[i].clone().unwrap_or_default(),
            category,
            timestamp: timestamps[i].clone().unwrap_or_default(),
            session_id: session_ids[i].clone(),
            score,
        });
    }
    out
}

#[async_trait]
impl Memory for LanceDbMemory {
    fn name(&self) -> &str {
        "lancedb"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        self.table
            .delete(&format!("key = '{}'", key.replace('\'', "''")))
            .await?;

        let vector = if self.dims > 0 {
            self.embedder.embed_one(content).await.ok()
        } else {
            None
        };

        let id = Uuid::new_v4().to_string();
        let timestamp = Utc::now().to_rfc3339();
        let cat_str = category.to_string();

        let batch = batch_from_entry(
            &id,
            key,
            content,
            &cat_str,
            &timestamp,
            session_id,
            vector.as_deref(),
            self.dims,
        )?;

        let schema = batch.schema();
        let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
        self.table.add(Box::new(reader)).execute().await?;
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        if self.dims > 0 {
            let qvec = self.embedder.embed_one(query).await?;

            let mut q = self.table.vector_search(qvec)?.limit(limit * 2);
            if let Some(sid) = session_id {
                q = q.only_if(format!("session_id = '{}'", sid.replace('\'', "''")));
            }
            let batches: Vec<RecordBatch> = q.execute().await?.try_collect().await?;

            let mut entries = Vec::new();
            for batch in &batches {
                let dist_col = batch.column_by_name("_distance");
                let dists: Vec<Option<f32>> = dist_col
                    .and_then(|c| c.as_any().downcast_ref::<Float32Array>())
                    .map(|a| {
                        (0..a.len())
                            .map(|i| if a.is_null(i) { None } else { Some(a.value(i)) })
                            .collect()
                    })
                    .unwrap_or_else(|| vec![None; batch.num_rows()]);

                let row_entries = entry_from_batch(batch, None);
                for (mut e, d) in row_entries.into_iter().zip(dists.into_iter()) {
                    e.score = d.map(|dist| f64::from(1.0 - dist));
                    entries.push(e);
                }
            }

            entries.truncate(limit);
            return Ok(entries);
        }

        let entries = self.list(None, session_id).await?;
        let q_lower = query.to_lowercase();
        let mut matched: Vec<MemoryEntry> = entries
            .into_iter()
            .filter(|e| {
                e.content.to_lowercase().contains(&q_lower)
                    || e.key.to_lowercase().contains(&q_lower)
            })
            .take(limit)
            .map(|mut e| {
                e.score = Some(1.0);
                e
            })
            .collect();
        matched.truncate(limit);
        Ok(matched)
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let filter = format!("key = '{}'", key.replace('\'', "''"));
        let batches: Vec<RecordBatch> = self
            .table
            .query()
            .only_if(filter)
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        for batch in &batches {
            let entries = entry_from_batch(batch, None);
            if let Some(e) = entries.into_iter().next() {
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let mut filters: Vec<String> = Vec::new();
        if let Some(cat) = category {
            filters.push(format!(
                "category = '{}'",
                cat.to_string().replace('\'', "''")
            ));
        }
        if let Some(sid) = session_id {
            filters.push(format!("session_id = '{}'", sid.replace('\'', "''")));
        }

        let mut q = self.table.query();
        if !filters.is_empty() {
            q = q.only_if(filters.join(" AND "));
        }

        let batches: Vec<RecordBatch> = q.execute().await?.try_collect().await?;
        let mut entries = Vec::new();
        for batch in &batches {
            entries.extend(entry_from_batch(batch, None));
        }
        Ok(entries)
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let existing = self.get(key).await?;
        if existing.is_none() {
            return Ok(false);
        }
        self.table
            .delete(&format!("key = '{}'", key.replace('\'', "''")))
            .await?;
        Ok(true)
    }

    async fn count(&self) -> Result<usize> {
        Ok(self.table.count_rows(None).await?)
    }

    async fn health_check(&self) -> bool {
        self.table.count_rows(None).await.is_ok()
    }
}
