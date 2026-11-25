//! AI 工具与模型（AI tools & models）— the MangoDisk-inspired category for
//! the local-AI crowd: Ollama weights, HuggingFace caches, and the desktop
//! AI apps' own caches and update payloads.
//!
//! The discipline that separates this from every other rule: **a model is
//! the user's downloaded property, not a cache.** 16 GB of llama.cpp weights
//! "regenerate" as a three-hour download on a bad day. So model rows are
//! never pre-selected, carry their last-seen timestamp, and live beside the
//! genuinely disposable caches (chat-app Electron caches, update payloads)
//! that follow normal cache rules.
//!
//! Last-used honesty: Ollama records no access times we can trust, so the
//! newest mtime among a model's manifest and blobs is shown as 最近使用时间
//! — the moment the model was pulled or re-patched. That is displayed, never
//! used to auto-delete.

use squid_core::{Bytes, Category, CleanupItem, ItemKind};
use std::path::{Path, PathBuf};

/// mtime of a file in ms since epoch, created() as backup.
fn mtime_ms(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    meta.modified()
        .or_else(|_| meta.created())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

fn dir_size(path: &Path) -> (u64, Option<u64>) {
    let mut total = 0u64;
    let mut newest: Option<u64> = None;
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .max_depth(6)
        .into_iter()
        .filter_entry(|e| !e.file_type().is_symlink())
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        total += meta.len();
        if let Some(ms) = mtime_ms(entry.path()) {
            newest = Some(newest.map_or(ms, |cur: u64| cur.max(ms)));
        }
    }
    (total, newest)
}

fn age_days(ms: u64) -> Option<f64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((now.saturating_sub(ms)) as f64 / 86_400_000.0)
}

fn age_note(ms: Option<u64>) -> Option<String> {
    let days = ms.and_then(age_days)?;
    if days < 1.0 {
        Some("今天或昨天用过".into())
    } else if days < 30.0 {
        Some(format!("约 {} 天前用过", days as u64))
    } else if days < 365.0 {
        Some(format!(
            "约 {} 个月没用过",
            (days / 30.0).round().max(1.0) as u64
        ))
    } else {
        Some(format!(
            "约 {} 年没用过",
            (days / 365.0).round().max(1.0) as u64
        ))
    }
}

/// A model row: name, size, last seen. `selected=false` is the contract —
/// the user decides, never the scanner.
fn model_item(
    root: PathBuf,
    label: String,
    size_bytes: u64,
    last_ms: Option<u64>,
    rule: &str,
) -> CleanupItem {
    let mut item = CleanupItem::new(root, ItemKind::Path, Category::AiTools, label)
        .with_rule(rule)
        .with_note(age_note(last_ms).unwrap_or_default());
    item.size = Bytes(size_bytes);
    item.allocated = Bytes(size_bytes);
    item.selected = false;
    if let Some(ms) = last_ms {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        item.last_write_secs = Some(now_ms.saturating_sub(ms) / 1000);
    }
    item.evidence = vec![
        "record:manifest".into(),
        format!("age:{}", age_note(last_ms).unwrap_or_default()),
    ];
    item
}

/// Ollama, sized honestly. A model is its manifest plus the blobs **no other
/// model also uses** — shared layers stay on disk until every model that
/// references them is gone, so a row's size never overstates what deleting
/// it frees. One model therefore becomes a small cluster of File items
/// (manifest + exclusive blobs) sharing a per-model rule id, which the 空间
/// 清理 page renders as one 清理点 named 「Ollama · 模型名」.
fn note_digest(value: &serde_json::Value, out: &mut Vec<(String, u64)>) {
    if let Some(digest) = value.get("digest").and_then(|d| d.as_str()) {
        let size = value.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        out.push((digest.to_string(), size));
    }
}

fn ollama_models(home: &Path) -> Vec<CleanupItem> {
    let models_root = home.join(".ollama").join("models");
    let manifests = models_root.join("manifests");
    if !manifests.is_dir() {
        return Vec::new();
    }
    // Pass 1: read every manifest, map digest → how many models use it.
    type Manifest = (PathBuf, String, Vec<(String, u64)>);
    let mut files: Vec<Manifest> = Vec::new();
    let mut refs: std::collections::HashMap<String, usize> = Default::default();
    let mut stack = vec![manifests];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(family) = path.parent().map(|p| {
                p.file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }) else {
                continue;
            };
            let Some(tag) = path.file_name().map(|t| t.to_string_lossy().into_owned()) else {
                continue;
            };
            let name = if family == "library" || family.is_empty() {
                tag.clone()
            } else {
                format!("{family}/{tag}")
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let mut digests: Vec<(String, u64)> = Vec::new();
            if let Some(config) = json.get("config") {
                note_digest(config, &mut digests);
            }
            if let Some(layers) = json.get("layers").and_then(|l| l.as_array()) {
                for layer in layers {
                    note_digest(layer, &mut digests);
                }
            }
            for (digest, _) in &digests {
                *refs.entry(digest.clone()).or_insert(0) += 1;
            }
            files.push((path, name, digests));
        }
    }
    // Pass 2: one cluster of items per model, exclusive bytes only.
    let mut rows = Vec::new();
    for (manifest, name, digests) in files {
        let slug: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let rule = format!("ai-ollama-{slug}");
        let mut size = 0u64;
        let mut newest = mtime_ms(&manifest);
        let mut shared_skipped = 0usize;
        let mut blob_items: Vec<(PathBuf, u64, u64)> = Vec::new();
        for (digest, declared) in &digests {
            if refs.get(digest).copied().unwrap_or(0) > 1 {
                shared_skipped += 1;
                continue; // another model still needs this layer
            }
            let blob = models_root.join("blobs").join(digest.replace(':', "-"));
            let Ok(meta) = std::fs::metadata(&blob) else {
                continue;
            };
            let bytes = meta.len().max(*declared);
            let ms = mtime_ms(&blob).unwrap_or(0);
            size += bytes;
            newest = Some(newest.map_or(ms, |cur: u64| cur.max(ms)));
            blob_items.push((blob, bytes, ms));
        }
        // A model whose layers are all shared frees nothing today, but the
        // row still means 「从 ollama list 移除」 — keep it, sized honestly.
        let age = age_note(newest);
        let mut shared_note = String::new();
        if shared_skipped > 0 {
            shared_note = format!("；另有 {shared_skipped} 个共享层被其它模型使用，不计算在内");
        }
        let label = format!("Ollama · {name}");
        // The manifest is the anchor row and carries the cluster label. Every
        // row — manifest and blobs alike — carries **its own** size: the 清理点
        // row sums its members, so charging the total to the manifest on top
        // of the blobs would double-count the model.
        let manifest_bytes = std::fs::metadata(&manifest).map(|m| m.len()).unwrap_or(0);
        let mut item = model_item(manifest, label.clone(), manifest_bytes, newest, &rule);
        item.kind = ItemKind::File; // the manifest file itself
        item.note = Some(format!(
            "{}；整个模型可释放 {}，删除后 `ollama pull` 可原样恢复{shared_note}",
            age.clone().unwrap_or_else(|| "从未记录使用时间".into()),
            Bytes(size + manifest_bytes)
        ));
        rows.push(item);
        for (blob, bytes, ms) in blob_items {
            let mut part = CleanupItem::new(blob, ItemKind::File, Category::AiTools, label.clone())
                .with_rule(&rule);
            part.size = Bytes(bytes);
            part.allocated = Bytes(bytes);
            part.selected = false;
            if let Some(ms) = Some(ms).filter(|m| *m > 0) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                part.last_write_secs = Some(now_ms.saturating_sub(ms) / 1000);
            }
            part.evidence = vec!["record:blob".into()];
            rows.push(part);
        }
    }
    rows
}

/// HuggingFace hub cache: one row per repo directory (models--org--name).
fn huggingface_cache(home: &Path) -> Vec<CleanupItem> {
    let hub = home.join(".cache").join("huggingface").join("hub");
    let Ok(entries) = std::fs::read_dir(&hub) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let pretty = name
            .strip_prefix("models--")
            .unwrap_or(&name)
            .replace("--", "/");
        let (size, newest) = dir_size(&path);
        if size == 0 {
            continue;
        }
        let mut item = model_item(
            path,
            format!("HuggingFace · {pretty}"),
            size,
            newest,
            "ai-huggingface",
        );
        item.note = Some(format!(
            "{}；重新使用时会自动重新下载（可能占用流量）",
            age_note(newest).unwrap_or_else(|| "从未记录使用时间".into())
        ));
        rows.push(item);
    }
    rows
}

/// The genuinely disposable side: Electron caches, logs, update payloads the
/// desktop AI apps keep. These follow normal cache semantics (pre-selected).
fn tool_caches() -> Vec<CleanupItem> {
    let mut rows = Vec::new();
    let mut add = |label: &str, path: PathBuf, note: &str| {
        if !path.is_dir() {
            return;
        }
        let (size, _) = dir_size(&path);
        if size == 0 {
            return;
        }
        let mut item = CleanupItem::new(path, ItemKind::Contents, Category::AiTools, label)
            .with_rule("ai-cache")
            .with_note(note);
        item.size = Bytes(size);
        item.allocated = Bytes(size);
        rows.push(item);
    };
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return rows;
    };
    let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return rows;
    };
    add(
        "Codex 桌面缓存",
        local.join("Codex").join("Cache"),
        "登录与界面缓存，重新打开自动重建",
    );
    add(
        "Codex 更新包",
        local.join("Codex").join("Packages"),
        "升级完成后不再需要",
    );
    add(
        "Claude 桌面缓存",
        roaming.join("Claude").join("Cache"),
        "重新打开自动重建",
    );
    add(
        "Claude 日志",
        roaming.join("Claude").join("logs"),
        "旧日志可安全清空",
    );
    add(
        "ChatGPT 桌面缓存",
        roaming.join("ChatGPT").join("WebCache"),
        "重新打开自动重建",
    );
    add(
        "ChatGPT 更新包",
        roaming.join("ChatGPT").join("Update"),
        "升级完成后不再需要",
    );
    rows
}

/// One entry point, same shape as `updates::collect`.
pub fn collect(_engine: &crate::Engine, cancel: &squid_core::CancelToken) -> Vec<CleanupItem> {
    let mut rows = Vec::new();
    if cancel.is_cancelled() {
        return rows;
    }
    let home = squid_core::paths::home_dir();
    rows.extend(ollama_models(&home));
    if cancel.is_cancelled() {
        return rows;
    }
    rows.extend(huggingface_cache(&home));
    rows.extend(tool_caches());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Option<PathBuf> {
        let root = std::env::temp_dir().join(format!(
            "squid-ai-{}-{}",
            std::process::id(),
            squid_core::ledger::new_session_id()
        ));
        std::fs::create_dir_all(&root).ok()?;
        Some(root)
    }

    #[test]
    fn ollama_models_are_listed_never_selected_with_sizes() {
        let Some(root) = fixture() else { return };
        let manifests = root.join(".ollama/models/manifests/registry.ollama.ai/library/qwen3");
        let blobs = root.join(".ollama/models/blobs");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(
            manifests.join("8b"),
            r#"{"layers":[{"digest":"sha256:aaa","size":2048}]}"#,
        )
        .unwrap();
        std::fs::write(blobs.join("sha256-aaa"), vec![1u8; 2048]).unwrap();
        let rows = ollama_models(&root);
        // Cluster: manifest File + exclusive blob File, one rule id each row.
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().all(|r| r.rule_id == "ai-ollama-qwen3-8b"));
        assert_eq!(rows[0].label, "Ollama · qwen3/8b");
        // Each row carries its own size; the cluster total rides in the note.
        let total = rows.iter().map(|r| r.size.as_u64()).sum::<u64>();
        assert!(total >= 2048, "manifest + exclusive blob, no double count");
        assert!(rows[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("可释放")));
        assert!(rows.iter().all(|r| !r.selected), "models never pre-checked");
        assert!(rows[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("可原样恢复")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shared_blobs_are_not_billed_to_either_model() {
        let Some(root) = fixture() else { return };
        let library = root.join(".ollama/models/manifests/registry.ollama.ai/library");
        std::fs::create_dir_all(library.join("a")).unwrap();
        std::fs::create_dir_all(library.join("b")).unwrap();
        std::fs::create_dir_all(root.join(".ollama/models/blobs")).unwrap();
        std::fs::write(
            library.join("a").join("latest"),
            r#"{"layers":[{"digest":"sha256:shared","size":5000},{"digest":"sha256:own-a","size":100}]}"#,
        )
        .unwrap();
        std::fs::write(
            library.join("b").join("latest"),
            r#"{"layers":[{"digest":"sha256:shared","size":5000}]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join(".ollama/models/blobs/sha256-shared"),
            vec![0u8; 5000],
        )
        .unwrap();
        std::fs::write(
            root.join(".ollama/models/blobs/sha256-own-a"),
            vec![0u8; 100],
        )
        .unwrap();
        let rows = ollama_models(&root);
        let model_a: Vec<_> = rows
            .iter()
            .filter(|r| r.rule_id == "ai-ollama-a-latest")
            .collect();
        let model_b: Vec<_> = rows
            .iter()
            .filter(|r| r.rule_id == "ai-ollama-b-latest")
            .collect();
        // A: manifest (its own JSON bytes) + own blob sized 100; the shared
        // 5 GB belongs to nobody yet, so it is billed to neither row.
        assert_eq!(model_a.len(), 2, "{model_a:?}");
        assert_eq!(model_a[1].size.as_u64(), 100);
        assert!(
            model_a[0].size.as_u64() < 200,
            "manifest carries its own bytes"
        );
        assert!(model_a[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("共享层")));
        // B: only the manifest survives (its own JSON bytes, not the shared
        // 5 GB), and its note says the layers are held by other models.
        assert_eq!(model_b.len(), 1, "{model_b:?}");
        assert!(model_b[0].size.as_u64() < 200, "manifest bytes only");
        assert!(model_b[0]
            .note
            .as_deref()
            .is_some_and(|n| n.contains("共享层")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn library_prefix_is_stripped_from_official_models() {
        let Some(root) = fixture() else { return };
        let manifests = root.join(".ollama/models/manifests/registry.ollama.ai/library/llama3");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::create_dir_all(root.join(".ollama/models/blobs")).unwrap();
        std::fs::write(
            manifests.join("latest"),
            r#"{"layers":[{"digest":"sha256:bbb","size":10}]}"#,
        )
        .unwrap();
        std::fs::write(root.join(".ollama/models/blobs/sha256-bbb"), vec![0u8; 10]).unwrap();
        let rows = ollama_models(&root);
        assert_eq!(rows[0].label, "Ollama · llama3/latest");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn huggingface_repos_get_one_row_each() {
        let Some(root) = fixture() else { return };
        let hub = root.join(".cache/huggingface/hub/models--bert--base");
        std::fs::create_dir_all(hub.join("snapshots/x")).unwrap();
        std::fs::write(hub.join("snapshots/x/model.bin"), vec![7u8; 4096]).unwrap();
        let rows = huggingface_cache(&root);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "HuggingFace · bert/base");
        assert_eq!(rows[0].size.as_u64(), 4096);
        assert!(!rows[0].selected);
        let _ = std::fs::remove_dir_all(&root);
    }
}
