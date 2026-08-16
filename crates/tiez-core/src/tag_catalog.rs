//! Tauri-independent tag catalog for native TieZ frontends.
//!
//! The catalog preserves the production `saved_tags` and `entry_tags` schema.
//! It owns tag metadata and produces global rename/delete plans, while callers
//! apply every entry mutation through [`crate::clipboard_history::ClipboardHistory`]
//! so encryption, OCR cleanup, tombstones, attachment lifetime, and session ID
//! semantics remain centralized in the history adapter.

use crate::clipboard_history::HistoryItem;
use crate::encryption::ENCRYPT_PREFIX;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_TAG_NAME_CHARS: usize = 64;
const MAX_TAG_ENTRIES: i64 = 1_000;
const SENSITIVE_TAGS: &[&str] = &["sensitive", "密码", "password"];
const SENSITIVE_PREVIEW: &str = "Sensitive entry — preview hidden";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagSummary {
    pub name: String,
    pub count: u64,
    pub color: Option<String>,
    pub protected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagCatalogSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub tags: Vec<TagSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagEntrySummary {
    pub id: i64,
    pub content_type: String,
    pub preview: String,
    pub source_app: String,
    pub captured_at: String,
    pub is_pinned: bool,
    pub tags: Vec<String>,
    pub is_sensitive: bool,
    pub use_count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagEntriesSnapshot {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub tag: String,
    pub total: u64,
    pub items: Vec<TagEntrySummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagCatalogMutation {
    pub adapter: &'static str,
    pub read_only: bool,
    pub generation: u64,
    pub action: &'static str,
    pub changed: bool,
    pub name: String,
    pub new_name: Option<String>,
    pub color: Option<String>,
    pub affected: u64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagEntryUpdate {
    pub id: i64,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagRenamePlan {
    pub old_name: String,
    pub new_name: String,
    pub entries: Vec<TagEntryUpdate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagDeletePlan {
    pub name: String,
    pub entry_ids: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagCatalogErrorKind {
    InvalidDatabase,
    NotFound,
    ReadOnly,
    Storage,
    Validation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagCatalogError {
    kind: TagCatalogErrorKind,
    message: String,
}

impl TagCatalogError {
    fn new(kind: TagCatalogErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> TagCatalogErrorKind {
        self.kind
    }
}

impl fmt::Display for TagCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TagCatalogError {}

#[derive(Debug, Default)]
struct MemoryTagCatalog {
    saved: BTreeMap<String, Option<String>>,
}

#[derive(Debug)]
enum TagCatalogAdapter {
    Memory(MemoryTagCatalog),
    Sqlite {
        database_path: PathBuf,
        read_only: bool,
    },
}

#[derive(Debug)]
pub struct TagCatalog {
    adapter: TagCatalogAdapter,
    generation: u64,
}

impl TagCatalog {
    pub fn in_memory() -> Self {
        Self {
            adapter: TagCatalogAdapter::Memory(MemoryTagCatalog::default()),
            generation: 1,
        }
    }

    pub fn open_sqlite(
        database_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, TagCatalogError> {
        let database_path = database_path.into();
        let connection = open_connection(&database_path, read_only)?;
        for table in ["saved_tags", "entry_tags", "clipboard_history"] {
            let exists = connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                     )",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| storage_error("failed to inspect tag schema", error))?;
            if exists != 1 {
                return Err(TagCatalogError::new(
                    TagCatalogErrorKind::InvalidDatabase,
                    format!("tag database is missing table {table}"),
                ));
            }
        }
        Ok(Self {
            adapter: TagCatalogAdapter::Sqlite {
                database_path,
                read_only,
            },
            generation: 1,
        })
    }

    pub fn snapshot(
        &self,
        history_items: &[HistoryItem],
    ) -> Result<TagCatalogSnapshot, TagCatalogError> {
        let mut tags = match &self.adapter {
            TagCatalogAdapter::Memory(memory) => memory_tags(memory, history_items),
            TagCatalogAdapter::Sqlite { database_path, .. } => sqlite_tags(database_path)?,
        };
        tags.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(TagCatalogSnapshot {
            adapter: self.adapter_name(),
            read_only: self.read_only(),
            generation: self.generation,
            tags,
        })
    }

    pub fn entries(
        &self,
        tag: &str,
        history_items: &[HistoryItem],
    ) -> Result<TagEntriesSnapshot, TagCatalogError> {
        let tag = existing_tag_name(tag)?;
        let (total, items) = match &self.adapter {
            TagCatalogAdapter::Memory(_) => {
                let items = history_items
                    .iter()
                    .filter(|item| item.tags.iter().any(|candidate| candidate == &tag))
                    .take(MAX_TAG_ENTRIES as usize)
                    .map(memory_entry_summary)
                    .collect::<Vec<_>>();
                let total = history_items
                    .iter()
                    .filter(|item| item.tags.iter().any(|candidate| candidate == &tag))
                    .count() as u64;
                (total, items)
            }
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                sqlite_tag_entries(database_path, &tag)?
            }
        };
        Ok(TagEntriesSnapshot {
            adapter: self.adapter_name(),
            read_only: self.read_only(),
            generation: self.generation,
            tag,
            total,
            items,
        })
    }

    pub fn create(&mut self, name: &str) -> Result<TagCatalogMutation, TagCatalogError> {
        self.ensure_writable()?;
        let name = normalized_tag_name(name)?;
        let changed = match &mut self.adapter {
            TagCatalogAdapter::Memory(memory) => {
                if memory.saved.contains_key(&name) {
                    false
                } else {
                    memory.saved.insert(name.clone(), None);
                    true
                }
            }
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, false)?;
                connection
                    .execute(
                        "INSERT OR IGNORE INTO saved_tags (name) VALUES (?1)",
                        [&name],
                    )
                    .map_err(|error| storage_error("failed to create tag", error))?
                    == 1
            }
        };
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        Ok(self.mutation(
            "create",
            changed,
            name,
            None,
            None,
            0,
            if changed {
                "已创建标签"
            } else {
                "标签已存在"
            },
        ))
    }

    pub fn set_color(
        &mut self,
        name: &str,
        color: Option<&str>,
    ) -> Result<TagCatalogMutation, TagCatalogError> {
        self.ensure_writable()?;
        let name = normalized_tag_name(name)?;
        let color = normalized_color(color)?;
        let changed = match &mut self.adapter {
            TagCatalogAdapter::Memory(memory) => {
                let previous = memory.saved.insert(name.clone(), color.clone()).flatten();
                previous != color
            }
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, false)?;
                let previous = connection
                    .query_row(
                        "SELECT color FROM saved_tags WHERE name = ?1",
                        [&name],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
                    .map_err(|error| storage_error("failed to read tag color", error))?
                    .flatten();
                connection
                    .execute(
                        "INSERT INTO saved_tags (name, color) VALUES (?1, ?2)
                         ON CONFLICT(name) DO UPDATE SET color = excluded.color",
                        rusqlite::params![name, color],
                    )
                    .map_err(|error| storage_error("failed to save tag color", error))?;
                previous != color
            }
        };
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        Ok(self.mutation(
            "set-color",
            changed,
            name,
            None,
            color,
            0,
            if changed {
                "标签颜色已更新"
            } else {
                "标签颜色未变化"
            },
        ))
    }

    pub fn rename_plan(
        &self,
        old_name: &str,
        new_name: &str,
        history_items: &[HistoryItem],
    ) -> Result<TagRenamePlan, TagCatalogError> {
        self.ensure_writable()?;
        let old_name = existing_tag_name(old_name)?;
        let new_name = normalized_tag_name(new_name)?;
        if old_name == new_name {
            return Ok(TagRenamePlan {
                old_name,
                new_name,
                entries: Vec::new(),
            });
        }
        if is_protected_tag(&old_name) || is_protected_tag(&new_name) {
            return Err(validation_error("内置敏感标签不能重命名或作为重命名目标"));
        }
        if !self.tag_exists(&old_name, history_items)? {
            return Err(TagCatalogError::new(
                TagCatalogErrorKind::NotFound,
                format!("标签不存在：{old_name}"),
            ));
        }

        let entries = match &self.adapter {
            TagCatalogAdapter::Memory(_) => history_items
                .iter()
                .filter(|item| item.tags.iter().any(|tag| tag == &old_name))
                .map(|item| TagEntryUpdate {
                    id: item.id,
                    tags: renamed_tags(&item.tags, &old_name, &new_name),
                })
                .collect(),
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                sqlite_rename_entries(database_path, &old_name, &new_name)?
            }
        };
        Ok(TagRenamePlan {
            old_name,
            new_name,
            entries,
        })
    }

    pub fn finish_rename(
        &mut self,
        plan: &TagRenamePlan,
    ) -> Result<TagCatalogMutation, TagCatalogError> {
        self.ensure_writable()?;
        if plan.old_name == plan.new_name {
            return Ok(self.mutation(
                "rename",
                false,
                plan.old_name.clone(),
                Some(plan.new_name.clone()),
                None,
                0,
                "标签名称未变化",
            ));
        }

        match &mut self.adapter {
            TagCatalogAdapter::Memory(memory) => {
                let old_color = memory.saved.remove(&plan.old_name).flatten();
                memory
                    .saved
                    .entry(plan.new_name.clone())
                    .or_insert(old_color);
            }
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, false)?;
                connection
                    .execute_batch("SAVEPOINT tiez_tag_catalog_rename")
                    .map_err(|error| storage_error("failed to start tag rename", error))?;
                let result = (|| {
                    let old_color = connection
                        .query_row(
                            "SELECT color FROM saved_tags WHERE name = ?1",
                            [&plan.old_name],
                            |row| row.get::<_, Option<String>>(0),
                        )
                        .optional()
                        .map_err(|error| storage_error("failed to read old tag color", error))?
                        .flatten();
                    connection
                        .execute(
                            "INSERT OR IGNORE INTO saved_tags (name, color) VALUES (?1, ?2)",
                            rusqlite::params![plan.new_name, old_color],
                        )
                        .map_err(|error| storage_error("failed to create renamed tag", error))?;
                    connection
                        .execute("DELETE FROM saved_tags WHERE name = ?1", [&plan.old_name])
                        .map_err(|error| storage_error("failed to remove old tag", error))?;
                    Ok(())
                })();
                finish_savepoint(&connection, "tiez_tag_catalog_rename", result)?;
            }
        }
        self.generation = self.generation.saturating_add(1);
        Ok(self.mutation(
            "rename",
            true,
            plan.old_name.clone(),
            Some(plan.new_name.clone()),
            None,
            plan.entries.len() as u64,
            "标签已重命名",
        ))
    }

    pub fn delete_plan(
        &self,
        name: &str,
        history_items: &[HistoryItem],
    ) -> Result<TagDeletePlan, TagCatalogError> {
        self.ensure_writable()?;
        let name = existing_tag_name(name)?;
        if is_protected_tag(&name) {
            return Err(validation_error("内置敏感标签不能删除"));
        }
        if !self.tag_exists(&name, history_items)? {
            return Err(TagCatalogError::new(
                TagCatalogErrorKind::NotFound,
                format!("标签不存在：{name}"),
            ));
        }
        let entry_ids = match &self.adapter {
            TagCatalogAdapter::Memory(_) => history_items
                .iter()
                .filter(|item| item.tags.iter().any(|tag| tag == &name))
                .map(|item| item.id)
                .collect(),
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, true)?;
                let mut statement = connection
                    .prepare("SELECT entry_id FROM entry_tags WHERE tag = ?1 ORDER BY entry_id")
                    .map_err(|error| storage_error("failed to prepare tag deletion", error))?;
                let rows = statement
                    .query_map([&name], |row| row.get::<_, i64>(0))
                    .map_err(|error| storage_error("failed to query tagged entries", error))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|error| storage_error("failed to read tagged entry", error))?
            }
        };
        Ok(TagDeletePlan { name, entry_ids })
    }

    pub fn finish_delete(
        &mut self,
        plan: &TagDeletePlan,
    ) -> Result<TagCatalogMutation, TagCatalogError> {
        self.ensure_writable()?;
        let metadata_removed = match &mut self.adapter {
            TagCatalogAdapter::Memory(memory) => memory.saved.remove(&plan.name).is_some(),
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, false)?;
                let remaining = connection
                    .query_row(
                        "SELECT COUNT(*) FROM entry_tags WHERE tag = ?1",
                        [&plan.name],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| storage_error("failed to verify tag deletion", error))?;
                if remaining != 0 {
                    return Err(validation_error(
                        "仍有记录使用该标签；不会删除标签元数据，请重试",
                    ));
                }
                connection
                    .execute("DELETE FROM saved_tags WHERE name = ?1", [&plan.name])
                    .map_err(|error| storage_error("failed to delete saved tag", error))?
                    > 0
            }
        };
        let changed = metadata_removed || !plan.entry_ids.is_empty();
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        Ok(self.mutation(
            "delete",
            changed,
            plan.name.clone(),
            None,
            None,
            plan.entry_ids.len() as u64,
            if changed {
                "标签及其记录已删除"
            } else {
                "标签已不存在"
            },
        ))
    }

    pub fn tags_for_entry(
        &self,
        entry_id: i64,
        history_items: &[HistoryItem],
    ) -> Result<Vec<String>, TagCatalogError> {
        if entry_id < 0 {
            return history_items
                .iter()
                .find(|item| item.id == entry_id)
                .map(|item| item.tags.clone())
                .ok_or_else(|| {
                    TagCatalogError::new(
                        TagCatalogErrorKind::NotFound,
                        format!("记录不存在：{entry_id}"),
                    )
                });
        }
        match &self.adapter {
            TagCatalogAdapter::Memory(_) => history_items
                .iter()
                .find(|item| item.id == entry_id)
                .map(|item| item.tags.clone())
                .ok_or_else(|| {
                    TagCatalogError::new(
                        TagCatalogErrorKind::NotFound,
                        format!("记录不存在：{entry_id}"),
                    )
                }),
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, true)?;
                let raw = connection
                    .query_row(
                        "SELECT tags FROM clipboard_history WHERE id = ?1",
                        [entry_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| storage_error("failed to read entry tags", error))?
                    .ok_or_else(|| {
                        TagCatalogError::new(
                            TagCatalogErrorKind::NotFound,
                            format!("记录不存在：{entry_id}"),
                        )
                    })?;
                Ok(parse_tags(&raw))
            }
        }
    }

    fn tag_exists(
        &self,
        name: &str,
        history_items: &[HistoryItem],
    ) -> Result<bool, TagCatalogError> {
        match &self.adapter {
            TagCatalogAdapter::Memory(memory) => Ok(memory.saved.contains_key(name)
                || history_items
                    .iter()
                    .any(|item| item.tags.iter().any(|tag| tag == name))),
            TagCatalogAdapter::Sqlite { database_path, .. } => {
                let connection = open_connection(database_path, true)?;
                connection
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM saved_tags WHERE name = ?1
                            UNION ALL
                            SELECT 1 FROM entry_tags WHERE tag = ?1
                         )",
                        [name],
                        |row| row.get::<_, i64>(0),
                    )
                    .map(|value| value == 1)
                    .map_err(|error| storage_error("failed to look up tag", error))
            }
        }
    }

    fn ensure_writable(&self) -> Result<(), TagCatalogError> {
        if self.read_only() {
            Err(TagCatalogError::new(
                TagCatalogErrorKind::ReadOnly,
                "只读数据副本不能修改标签",
            ))
        } else {
            Ok(())
        }
    }

    fn adapter_name(&self) -> &'static str {
        match &self.adapter {
            TagCatalogAdapter::Memory(_) => "memory",
            TagCatalogAdapter::Sqlite {
                read_only: true, ..
            } => "sqlite-read-only",
            TagCatalogAdapter::Sqlite {
                read_only: false, ..
            } => "sqlite",
        }
    }

    fn read_only(&self) -> bool {
        matches!(
            &self.adapter,
            TagCatalogAdapter::Sqlite {
                read_only: true,
                ..
            }
        )
    }

    fn mutation(
        &self,
        action: &'static str,
        changed: bool,
        name: String,
        new_name: Option<String>,
        color: Option<String>,
        affected: u64,
        message: impl Into<String>,
    ) -> TagCatalogMutation {
        TagCatalogMutation {
            adapter: self.adapter_name(),
            read_only: self.read_only(),
            generation: self.generation,
            action,
            changed,
            name,
            new_name,
            color,
            affected,
            message: message.into(),
        }
    }
}

pub fn normalized_tag_name(name: &str) -> Result<String, TagCatalogError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(validation_error("标签名称不能为空"));
    }
    if name.chars().count() > MAX_TAG_NAME_CHARS {
        return Err(validation_error("标签名称不能超过 64 个字符"));
    }
    if name.chars().any(char::is_control) {
        return Err(validation_error("标签名称不能包含控制字符"));
    }
    Ok(name.to_owned())
}

pub fn is_protected_tag(name: &str) -> bool {
    SENSITIVE_TAGS
        .iter()
        .any(|sensitive| sensitive.eq_ignore_ascii_case(name.trim()))
}

fn existing_tag_name(name: &str) -> Result<String, TagCatalogError> {
    normalized_tag_name(name)
}

fn normalized_color(color: Option<&str>) -> Result<Option<String>, TagCatalogError> {
    let Some(color) = color.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let bytes = color.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(validation_error("标签颜色必须使用 #RRGGBB 格式"));
    }
    Ok(Some(color.to_ascii_uppercase()))
}

fn memory_tags(memory: &MemoryTagCatalog, history_items: &[HistoryItem]) -> Vec<TagSummary> {
    let mut values = memory
        .saved
        .iter()
        .map(|(name, color)| {
            (
                name.clone(),
                TagSummary {
                    name: name.clone(),
                    count: 0,
                    color: color.clone(),
                    protected: is_protected_tag(name),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for item in history_items {
        for name in &item.tags {
            let summary = values.entry(name.clone()).or_insert_with(|| TagSummary {
                name: name.clone(),
                count: 0,
                color: None,
                protected: is_protected_tag(name),
            });
            summary.count = summary.count.saturating_add(1);
        }
    }
    values.into_values().collect()
}

fn sqlite_tags(database_path: &Path) -> Result<Vec<TagSummary>, TagCatalogError> {
    let connection = open_connection(database_path, true)?;
    let mut values = BTreeMap::<String, TagSummary>::new();
    {
        let mut statement = connection
            .prepare("SELECT name, color FROM saved_tags WHERE trim(name) != ''")
            .map_err(|error| storage_error("failed to prepare saved tags", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| storage_error("failed to query saved tags", error))?;
        for row in rows {
            let (name, color) =
                row.map_err(|error| storage_error("failed to read saved tag", error))?;
            values.insert(
                name.clone(),
                TagSummary {
                    name: name.clone(),
                    count: 0,
                    color,
                    protected: is_protected_tag(&name),
                },
            );
        }
    }
    {
        let mut statement = connection
            .prepare("SELECT tag, COUNT(*) FROM entry_tags WHERE trim(tag) != '' GROUP BY tag")
            .map_err(|error| storage_error("failed to prepare tag counts", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })
            .map_err(|error| storage_error("failed to query tag counts", error))?;
        for row in rows {
            let (name, count) =
                row.map_err(|error| storage_error("failed to read tag count", error))?;
            values
                .entry(name.clone())
                .and_modify(|summary| summary.count = count)
                .or_insert_with(|| TagSummary {
                    name: name.clone(),
                    count,
                    color: None,
                    protected: is_protected_tag(&name),
                });
        }
    }
    Ok(values.into_values().collect())
}

fn sqlite_tag_entries(
    database_path: &Path,
    tag: &str,
) -> Result<(u64, Vec<TagEntrySummary>), TagCatalogError> {
    let connection = open_connection(database_path, true)?;
    let total = connection
        .query_row(
            "SELECT COUNT(*) FROM entry_tags WHERE tag = ?1",
            [tag],
            |row| row.get::<_, u64>(0),
        )
        .map_err(|error| storage_error("failed to count tagged entries", error))?;
    let mut statement = connection
        .prepare(
            "SELECT h.id, h.content_type, h.preview, h.source_app, h.timestamp,
                    h.is_pinned, h.tags, h.use_count
             FROM clipboard_history h
             INNER JOIN entry_tags t ON t.entry_id = h.id
             WHERE t.tag = ?1
             ORDER BY h.is_pinned DESC, h.pinned_order DESC, h.timestamp DESC, h.id DESC
             LIMIT ?2",
        )
        .map_err(|error| storage_error("failed to prepare tagged entries", error))?;
    let rows = statement
        .query_map(rusqlite::params![tag, MAX_TAG_ENTRIES], |row| {
            let tags = parse_tags(&row.get::<_, String>(6)?);
            let preview = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let is_sensitive = tags.iter().any(|name| is_protected_tag(name))
                || preview.starts_with(ENCRYPT_PREFIX);
            Ok(TagEntrySummary {
                id: row.get(0)?,
                content_type: row.get(1)?,
                preview: if is_sensitive {
                    SENSITIVE_PREVIEW.to_owned()
                } else {
                    preview
                },
                source_app: row.get(3)?,
                captured_at: format_timestamp(row.get(4)?),
                is_pinned: row.get::<_, i32>(5)? == 1,
                tags,
                is_sensitive,
                use_count: row.get(7).unwrap_or(0),
            })
        })
        .map_err(|error| storage_error("failed to query tagged entries", error))?;
    let items = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage_error("failed to read tagged entry", error))?;
    Ok((total, items))
}

fn sqlite_rename_entries(
    database_path: &Path,
    old_name: &str,
    new_name: &str,
) -> Result<Vec<TagEntryUpdate>, TagCatalogError> {
    let connection = open_connection(database_path, true)?;
    let mut statement = connection
        .prepare(
            "SELECT h.id, h.tags
             FROM clipboard_history h
             INNER JOIN entry_tags t ON t.entry_id = h.id
             WHERE t.tag = ?1
             ORDER BY h.id",
        )
        .map_err(|error| storage_error("failed to prepare tag rename", error))?;
    let rows = statement
        .query_map([old_name], |row| {
            let id = row.get::<_, i64>(0)?;
            let tags = parse_tags(&row.get::<_, String>(1)?);
            Ok(TagEntryUpdate {
                id,
                tags: renamed_tags(&tags, old_name, new_name),
            })
        })
        .map_err(|error| storage_error("failed to query tag rename entries", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage_error("failed to read tag rename entry", error))
}

fn renamed_tags(tags: &[String], old_name: &str, new_name: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut renamed = Vec::new();
    let mut replaced = false;
    for tag in tags {
        let tag = if tag == old_name {
            replaced = true;
            new_name
        } else {
            tag
        };
        if seen.insert(tag.to_owned()) {
            renamed.push(tag.to_owned());
        }
    }
    if !replaced && seen.insert(new_name.to_owned()) {
        renamed.push(new_name.to_owned());
    }
    renamed
}

fn memory_entry_summary(item: &HistoryItem) -> TagEntrySummary {
    TagEntrySummary {
        id: item.id,
        content_type: item.content_type.clone(),
        preview: if item.is_sensitive {
            SENSITIVE_PREVIEW.to_owned()
        } else {
            item.preview.clone()
        },
        source_app: item.source_app.clone(),
        captured_at: item.captured_at.clone(),
        is_pinned: item.is_pinned,
        tags: item.tags.clone(),
        is_sensitive: item.is_sensitive,
        use_count: 0,
    }
}

fn parse_tags(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tag| {
            let tag = tag.trim();
            if tag.is_empty() || !seen.insert(tag.to_owned()) {
                None
            } else {
                Some(tag.to_owned())
            }
        })
        .collect()
}

fn format_timestamp(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let age = now.saturating_sub(timestamp.max(0));
    match age {
        age if age < 60_000 => "Just now".to_owned(),
        age if age < 3_600_000 => format!("{} minutes ago", age / 60_000),
        age if age < 86_400_000 => format!("{} hours ago", age / 3_600_000),
        age if age < 604_800_000 => format!("{} days ago", age / 86_400_000),
        _ => timestamp.to_string(),
    }
}

fn open_connection(database_path: &Path, read_only: bool) -> Result<Connection, TagCatalogError> {
    if !database_path.is_file() {
        return Err(TagCatalogError::new(
            TagCatalogErrorKind::InvalidDatabase,
            format!("database does not exist: {}", database_path.display()),
        ));
    }
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(database_path, flags)
        .map_err(|error| storage_error("failed to open tag database", error))?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| storage_error("failed to configure tag database", error))?;
    Ok(connection)
}

fn finish_savepoint(
    connection: &Connection,
    name: &str,
    result: Result<(), TagCatalogError>,
) -> Result<(), TagCatalogError> {
    match result {
        Ok(()) => connection
            .execute_batch(&format!("RELEASE SAVEPOINT {name}"))
            .map_err(|error| storage_error("failed to commit tag metadata", error)),
        Err(error) => {
            let _ = connection.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name}"
            ));
            Err(error)
        }
    }
}

fn validation_error(message: impl Into<String>) -> TagCatalogError {
    TagCatalogError::new(TagCatalogErrorKind::Validation, message)
}

fn storage_error(context: &str, error: impl fmt::Display) -> TagCatalogError {
    TagCatalogError::new(TagCatalogErrorKind::Storage, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;

    struct TestDatabase {
        root: PathBuf,
        path: PathBuf,
    }

    impl TestDatabase {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tiez-tag-catalog-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            let path = root.join("clipboard.db");
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE saved_tags (name TEXT PRIMARY KEY, color TEXT);
                     CREATE TABLE clipboard_history (
                        id INTEGER PRIMARY KEY,
                        content_type TEXT NOT NULL DEFAULT 'text',
                        preview TEXT,
                        source_app TEXT NOT NULL DEFAULT '',
                        timestamp INTEGER NOT NULL DEFAULT 0,
                        is_pinned INTEGER NOT NULL DEFAULT 0,
                        pinned_order INTEGER NOT NULL DEFAULT 0,
                        tags TEXT NOT NULL DEFAULT '[]',
                        use_count INTEGER NOT NULL DEFAULT 0
                     );
                     CREATE TABLE entry_tags (
                        entry_id INTEGER NOT NULL,
                        tag TEXT NOT NULL,
                        PRIMARY KEY (entry_id, tag)
                     );",
                )
                .unwrap();
            drop(connection);
            Self { root, path }
        }

        fn connection(&self) -> Connection {
            Connection::open(&self.path).unwrap()
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn history_item(id: i64, tags: &[&str]) -> HistoryItem {
        HistoryItem {
            id,
            content_type: "text".to_owned(),
            preview: format!("item {id}"),
            source_app: "tests".to_owned(),
            captured_at: "Just now".to_owned(),
            is_pinned: false,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            is_sensitive: tags.iter().any(|tag| is_protected_tag(tag)),
        }
    }

    #[test]
    fn sqlite_snapshot_combines_saved_tags_counts_colors_and_protection() {
        let database = TestDatabase::new("snapshot");
        database
            .connection()
            .execute_batch(
                "INSERT INTO saved_tags (name, color) VALUES
                    ('zero', '#123456'), ('work', '#ABCDEF'), ('密码', NULL);
                 INSERT INTO clipboard_history (id, tags) VALUES
                    (1, '[\"work\"]'), (2, '[\"work\",\"密码\"]');
                 INSERT INTO entry_tags (entry_id, tag) VALUES
                    (1, 'work'), (2, 'work'), (2, '密码');",
            )
            .unwrap();
        let catalog = TagCatalog::open_sqlite(&database.path, false).unwrap();

        let snapshot = catalog.snapshot(&[]).unwrap();

        assert_eq!(snapshot.tags[0].name, "work");
        assert_eq!(snapshot.tags[0].count, 2);
        assert_eq!(snapshot.tags[0].color.as_deref(), Some("#ABCDEF"));
        assert!(
            snapshot
                .tags
                .iter()
                .find(|tag| tag.name == "密码")
                .unwrap()
                .protected
        );
        assert_eq!(
            snapshot
                .tags
                .iter()
                .find(|tag| tag.name == "zero")
                .unwrap()
                .count,
            0
        );
    }

    #[test]
    fn rename_plan_merges_entry_tags_and_metadata_without_direct_history_writes() {
        let database = TestDatabase::new("rename");
        database
            .connection()
            .execute_batch(
                "INSERT INTO saved_tags (name, color) VALUES ('old', '#123456'), ('new', '#654321');
                 INSERT INTO clipboard_history (id, tags) VALUES
                    (1, '[\"old\",\"new\",\"other\"]');
                 INSERT INTO entry_tags (entry_id, tag) VALUES
                    (1, 'old'), (1, 'new'), (1, 'other');",
            )
            .unwrap();
        let mut catalog = TagCatalog::open_sqlite(&database.path, false).unwrap();

        let plan = catalog.rename_plan("old", "new", &[]).unwrap();

        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.entries[0].tags, vec!["new", "other"]);
        let before = database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM entry_tags WHERE tag = 'old'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(before, 1, "the history adapter must apply the plan");

        database
            .connection()
            .execute("DELETE FROM entry_tags WHERE tag = 'old'", [])
            .unwrap();
        let mutation = catalog.finish_rename(&plan).unwrap();
        assert_eq!(mutation.affected, 1);
        let connection = database.connection();
        let old_count = connection
            .query_row(
                "SELECT COUNT(*) FROM saved_tags WHERE name = 'old'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        let new_color = connection
            .query_row(
                "SELECT color FROM saved_tags WHERE name = 'new'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(old_count, 0);
        assert_eq!(new_color, "#654321", "target color wins during merge");
    }

    #[test]
    fn delete_metadata_waits_until_all_tagged_entries_are_removed() {
        let database = TestDatabase::new("delete");
        database
            .connection()
            .execute_batch(
                "INSERT INTO saved_tags (name) VALUES ('remove-me');
                 INSERT INTO clipboard_history (id, tags) VALUES (1, '[\"remove-me\"]');
                 INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'remove-me');",
            )
            .unwrap();
        let mut catalog = TagCatalog::open_sqlite(&database.path, false).unwrap();
        let plan = catalog.delete_plan("remove-me", &[]).unwrap();
        assert_eq!(plan.entry_ids, vec![1]);

        assert_eq!(
            catalog.finish_delete(&plan).unwrap_err().kind(),
            TagCatalogErrorKind::Validation
        );
        database
            .connection()
            .execute_batch(
                "DELETE FROM entry_tags WHERE entry_id = 1;
                 DELETE FROM clipboard_history WHERE id = 1;",
            )
            .unwrap();
        assert!(catalog.finish_delete(&plan).unwrap().changed);
    }

    #[test]
    fn sensitive_entries_are_redacted_and_read_only_mutations_are_rejected() {
        let database = TestDatabase::new("privacy");
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        database
            .connection()
            .execute(
                "INSERT INTO clipboard_history
                    (id, content_type, preview, source_app, timestamp, tags, use_count)
                 VALUES (1, 'text', 'dpapi:ciphertext', 'tests', ?1, '[\"password\"]', 4)",
                [timestamp],
            )
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO entry_tags (entry_id, tag) VALUES (1, 'password')",
                [],
            )
            .unwrap();
        let mut catalog = TagCatalog::open_sqlite(&database.path, true).unwrap();

        let entries = catalog.entries("password", &[]).unwrap();

        assert_eq!(entries.total, 1);
        assert!(entries.items[0].is_sensitive);
        assert_eq!(entries.items[0].preview, SENSITIVE_PREVIEW);
        assert_eq!(
            catalog.create("blocked").unwrap_err().kind(),
            TagCatalogErrorKind::ReadOnly
        );
    }

    #[test]
    fn memory_catalog_uses_history_counts_and_protects_builtin_tags() {
        let mut catalog = TagCatalog::in_memory();
        catalog.create("work").unwrap();
        catalog.set_color("work", Some("#12abEF")).unwrap();
        let items = vec![history_item(1, &["work"]), history_item(2, &["密码"])];

        let snapshot = catalog.snapshot(&items).unwrap();

        assert_eq!(snapshot.tags[0].count, 1);
        assert_eq!(
            snapshot
                .tags
                .iter()
                .find(|tag| tag.name == "work")
                .unwrap()
                .color
                .as_deref(),
            Some("#12ABEF")
        );
        assert_eq!(
            catalog.delete_plan("密码", &items).unwrap_err().kind(),
            TagCatalogErrorKind::Validation
        );
        assert_eq!(
            catalog
                .rename_plan("work", "password", &items)
                .unwrap_err()
                .kind(),
            TagCatalogErrorKind::Validation
        );
    }
}
