//! Cache Management Module
//!
//! This module provides comprehensive cache management capabilities for the LSP daemon,
//! including statistics reporting, cache clearing, import/export functionality,
//! and database compaction.

use crate::call_graph_cache::CallGraphCache;
use crate::language_detector::Language;
use crate::lsp_cache::LspCache;
use crate::persistent_cache::PersistentCallGraphCache;
use crate::protocol::{
    AgeDistribution, CacheStatistics, ClearFilter, ClearResult, CompactOptions, CompactResult,
    ExportOptions, HotSpot, ImportResult, MemoryUsage,
};
use anyhow::{Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Export format for cache data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheExport {
    /// Export metadata
    pub metadata: ExportMetadata,
    /// Exported cache entries
    pub entries: Vec<ExportEntry>,
}

/// Metadata about a cache export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMetadata {
    /// Export timestamp
    pub export_date: SystemTime,
    /// Export statistics
    pub total_entries: u64,
    pub total_size_bytes: u64,
    /// Format version for compatibility
    pub format_version: u32,
}

/// Individual cache entry for export
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportEntry {
    /// Cache key information
    pub file_path: PathBuf,
    pub symbol: String,
    pub line: u32,
    pub column: u32,
    pub content_hash: String,
    /// Cache metadata
    pub created_at: SystemTime,
    pub language: Language,
    /// Call hierarchy data
    pub call_hierarchy: crate::cache_types::CallHierarchyInfo,
}

/// Manages all cache operations including statistics, clearing, import/export, and compaction
pub struct CacheManager {
    /// Main call graph cache (in-memory)
    call_graph_cache: Arc<CallGraphCache>,
    /// Persistent storage backend
    persistent_store: Arc<PersistentCallGraphCache>,
    /// Individual LSP caches
    definition_cache: Arc<LspCache<crate::cache_types::DefinitionInfo>>,
    references_cache: Arc<LspCache<crate::cache_types::ReferencesInfo>>,
    hover_cache: Arc<LspCache<crate::cache_types::HoverInfo>>,
    /// Statistics tracking
    stats: Arc<RwLock<CacheManagerStats>>,
}

/// Internal statistics for cache manager
#[derive(Debug, Default)]
struct CacheManagerStats {
    /// Operation counters
    stats_requests: u64,
    clear_operations: u64,
    export_operations: u64,
    import_operations: u64,
    compact_operations: u64,
    /// Timing metrics
    last_clear_duration_ms: u64,
    last_export_duration_ms: u64,
    last_import_duration_ms: u64,
    last_compact_duration_ms: u64,
}

impl CacheManager {
    /// Create a new cache manager instance
    pub fn new(
        call_graph_cache: Arc<CallGraphCache>,
        persistent_store: Arc<PersistentCallGraphCache>,
        definition_cache: Arc<LspCache<crate::cache_types::DefinitionInfo>>,
        references_cache: Arc<LspCache<crate::cache_types::ReferencesInfo>>,
        hover_cache: Arc<LspCache<crate::cache_types::HoverInfo>>,
    ) -> Self {
        Self {
            call_graph_cache,
            persistent_store,
            definition_cache,
            references_cache,
            hover_cache,
            stats: Arc::new(RwLock::new(CacheManagerStats::default())),
        }
    }

    /// Get comprehensive cache statistics
    pub async fn get_stats(&self, detailed: bool, git_stats: bool) -> Result<CacheStatistics> {
        let start_time = Instant::now();

        info!(
            "Generating cache statistics (detailed: {}, git: {})",
            detailed, git_stats
        );

        // Get persistent cache stats
        let persistent_stats = self
            .persistent_store
            .get_stats()
            .await
            .context("Failed to get persistent cache statistics")?;

        // Get in-memory cache stats
        let call_graph_stats = self.call_graph_cache.stats().await;
        let definition_stats = self.definition_cache.stats().await;
        let references_stats = self.references_cache.stats().await;
        let hover_stats = self.hover_cache.stats().await;

        // Calculate totals
        let total_entries = persistent_stats.total_nodes
            + call_graph_stats.total_nodes as u64
            + definition_stats.total_entries as u64
            + references_stats.total_entries as u64
            + hover_stats.total_entries as u64;

        let total_size_bytes = persistent_stats.total_size_bytes
            + call_graph_stats.persistent_size_bytes.unwrap_or(0)
            + definition_stats.memory_usage_estimate as u64
            + references_stats.memory_usage_estimate as u64
            + hover_stats.memory_usage_estimate as u64;

        // Calculate hit rates
        let total_hits =
            definition_stats.hit_count + references_stats.hit_count + hover_stats.hit_count;
        let total_misses =
            definition_stats.miss_count + references_stats.miss_count + hover_stats.miss_count;
        let total_requests = total_hits + total_misses;

        let hit_rate = if total_requests > 0 {
            total_hits as f64 / total_requests as f64
        } else {
            0.0
        };

        // Build entries per file map
        let entries_per_file = if detailed {
            self.build_entries_per_file_map().await?
        } else {
            HashMap::new()
        };

        // Build entries per language map
        let entries_per_language = if detailed {
            self.build_entries_per_language_map().await?
        } else {
            HashMap::new()
        };

        // Git statistics are no longer available
        if git_stats {
            warn!("Git statistics requested but git integration was removed");
        }
        let (_entries_per_branch, _entries_per_commit): (
            HashMap<String, u64>,
            HashMap<String, u64>,
        ) = (HashMap::new(), HashMap::new());

        // Build age distribution
        let age_distribution = self.build_age_distribution().await?;

        // Build hot spots (most accessed entries)
        let most_accessed = if detailed {
            self.build_hot_spots().await?
        } else {
            Vec::new()
        };

        // Build memory usage breakdown
        let memory_usage = MemoryUsage {
            in_memory_cache_bytes: call_graph_stats.persistent_size_bytes.unwrap_or(0)
                + definition_stats.memory_usage_estimate as u64
                + references_stats.memory_usage_estimate as u64
                + hover_stats.memory_usage_estimate as u64,
            persistent_cache_bytes: persistent_stats.total_size_bytes,
            metadata_bytes: persistent_stats.total_size_bytes / 10, // Estimate metadata as 10% of total
            index_bytes: persistent_stats.total_size_bytes / 20, // Estimate indexes as 5% of total
        };

        let statistics = CacheStatistics {
            total_size_bytes,
            disk_size_bytes: persistent_stats.disk_size_bytes,
            total_entries,
            entries_per_file,
            entries_per_language,
            hit_rate,
            miss_rate: 1.0 - hit_rate,
            age_distribution,
            most_accessed,
            memory_usage,
        };

        // Update internal stats
        {
            let mut stats = self.stats.write().await;
            stats.stats_requests += 1;
        }

        let duration = start_time.elapsed();
        info!("Cache statistics generated in {}ms", duration.as_millis());

        Ok(statistics)
    }

    /// Clear cache entries based on filter criteria
    pub async fn clear(&self, filter: ClearFilter) -> Result<ClearResult> {
        let start_time = Instant::now();

        info!("Clearing cache with filter: {:?}", filter);

        let mut entries_removed = 0u64;
        let mut files_affected = 0u64;
        let branches_affected = 0u64;
        let commits_affected = 0u64;
        let mut bytes_reclaimed = 0u64;

        // Clear all if requested
        if filter.all {
            info!("Clearing all cache entries");

            // Clear persistent cache
            let persistent_stats_before = self.persistent_store.get_stats().await?;
            self.persistent_store
                .clear()
                .await
                .context("Failed to clear persistent cache")?;

            // Clear in-memory caches
            self.call_graph_cache.clear();
            self.definition_cache.clear().await;
            self.references_cache.clear().await;
            self.hover_cache.clear().await;

            entries_removed = persistent_stats_before.total_nodes;
            files_affected = persistent_stats_before.total_files as u64;
            bytes_reclaimed = persistent_stats_before.total_size_bytes;
        } else {
            // Clear by specific filters
            if let Some(days) = filter.older_than_days {
                info!("Clearing entries older than {} days", days);
                let removed = self.persistent_store.cleanup_expired().await?;
                entries_removed += removed as u64;
            }

            if let Some(ref file_path) = filter.file_path {
                info!("Clearing entries for file: {:?}", file_path);
                let nodes_before = self.persistent_store.get_by_file(file_path).await?;
                entries_removed += nodes_before.len() as u64;
                files_affected = 1;

                // Remove from persistent cache
                for node in nodes_before {
                    self.persistent_store.remove(&node.key).await?;
                }

                // Remove from in-memory caches
                self.call_graph_cache.invalidate_file(file_path);
                self.definition_cache.invalidate_file(file_path).await;
                self.references_cache.invalidate_file(file_path).await;
                self.hover_cache.invalidate_file(file_path).await;
            }

            if let Some(ref commit_hash) = filter.commit_hash {
                warn!(
                    "Clearing by commit hash ignored - git integration removed: {}",
                    commit_hash
                );
            }
        }

        let duration = start_time.elapsed();

        // Update internal stats
        {
            let mut stats = self.stats.write().await;
            stats.clear_operations += 1;
            stats.last_clear_duration_ms = duration.as_millis() as u64;
        }

        let result = ClearResult {
            entries_removed,
            files_affected,
            branches_affected,
            commits_affected,
            bytes_reclaimed,
            duration_ms: duration.as_millis() as u64,
        };

        info!("Cache clear completed: {:?}", result);
        Ok(result)
    }

    /// Export cache to file
    pub async fn export(&self, path: &Path, options: ExportOptions) -> Result<(usize, bool)> {
        let start_time = Instant::now();

        info!("Exporting cache to: {:?} (options: {:?})", path, options);

        // Get all cached nodes from persistent storage
        let all_nodes = self
            .persistent_store
            .iter_nodes()
            .await
            .context("Failed to iterate cache nodes")?;

        // Note: current_branch_only option is ignored since git integration was removed
        if options.current_branch_only {
            warn!("current_branch_only option ignored - git integration removed");
        }
        let filtered_nodes = all_nodes;

        // Convert to export format
        let export_entries: Vec<ExportEntry> = filtered_nodes
            .into_iter()
            .map(|(key, node)| ExportEntry {
                file_path: key.file,
                symbol: key.symbol,
                line: 0,   // TODO: Extract from node key if available
                column: 0, // TODO: Extract from node key if available
                content_hash: key.content_md5,
                created_at: node.created_at,
                language: node.language,
                call_hierarchy: node.info,
            })
            .collect();

        let total_entries = export_entries.len() as u64;
        let total_size_bytes = export_entries
            .iter()
            .map(|entry| {
                bincode::serialize(entry)
                    .map(|data| data.len())
                    .unwrap_or(0)
            })
            .sum::<usize>() as u64;

        // Create export data
        let export_data = CacheExport {
            metadata: ExportMetadata {
                export_date: SystemTime::now(),
                total_entries,
                total_size_bytes,
                format_version: 1,
            },
            entries: export_entries,
        };

        // Serialize and write to file
        let serialized =
            bincode::serialize(&export_data).context("Failed to serialize export data")?;

        if options.compress {
            // Write compressed data
            let file = File::create(path).context("Failed to create export file")?;
            let mut encoder = GzEncoder::new(BufWriter::new(file), Compression::default());
            encoder
                .write_all(&serialized)
                .context("Failed to write compressed export data")?;
            encoder.finish().context("Failed to finalize compression")?;
        } else {
            // Write uncompressed data
            let mut file = File::create(path).context("Failed to create export file")?;
            file.write_all(&serialized)
                .context("Failed to write export data")?;
        }

        let duration = start_time.elapsed();

        // Update internal stats
        {
            let mut stats = self.stats.write().await;
            stats.export_operations += 1;
            stats.last_export_duration_ms = duration.as_millis() as u64;
        }

        info!(
            "Cache export completed: {} entries exported to {:?} in {}ms (compressed: {})",
            total_entries,
            path,
            duration.as_millis(),
            options.compress
        );

        Ok((total_entries as usize, options.compress))
    }

    /// Import cache from file
    pub async fn import(&self, path: &Path, merge: bool) -> Result<ImportResult> {
        let start_time = Instant::now();

        info!("Importing cache from: {:?} (merge: {})", path, merge);

        // Read and deserialize import file
        let file = File::open(path).context("Failed to open import file")?;
        let mut reader = BufReader::new(file);

        // Try to detect if file is compressed by reading magic bytes
        let mut magic_bytes = [0u8; 2];
        reader.read_exact(&mut magic_bytes)?;
        let is_compressed = magic_bytes == [0x1f, 0x8b]; // gzip magic

        // Reset reader
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let export_data: CacheExport = if is_compressed {
            let decoder = GzDecoder::new(reader);
            bincode::deserialize_from(decoder)
                .context("Failed to deserialize compressed import data")?
        } else {
            bincode::deserialize_from(reader).context("Failed to deserialize import data")?
        };

        // Validate import data
        let mut validation_errors = Vec::new();
        if export_data.metadata.format_version != 1 {
            validation_errors.push(format!(
                "Unsupported format version: {}",
                export_data.metadata.format_version
            ));
        }

        // Clear existing cache if not merging
        if !merge {
            info!("Replacing existing cache (merge=false)");
            self.persistent_store.clear().await?;
        }

        let mut entries_imported = 0u64;
        let mut entries_merged = 0u64;
        let mut entries_replaced = 0u64;
        let mut bytes_imported = 0u64;

        // Import each entry
        for entry in export_data.entries {
            // Calculate size before the entry is potentially moved
            let entry_size = bincode::serialize(&entry)
                .map(|data| data.len())
                .unwrap_or(0) as u64;

            let key = crate::cache_types::NodeKey::new(
                entry.symbol.clone(),
                entry.file_path.clone(),
                entry.content_hash.clone(),
            );

            // Check if entry already exists
            let existing = self.persistent_store.get(&key).await?;

            match existing {
                Some(_) if merge => {
                    // Entry exists and we're merging - keep newer one
                    entries_merged += 1;
                }
                Some(_) => {
                    // Entry exists and we're replacing
                    self.persistent_store.remove(&key).await?;
                    self.persistent_store
                        .insert(key, entry.call_hierarchy, entry.language)
                        .await?;
                    entries_replaced += 1;
                }
                None => {
                    // New entry
                    self.persistent_store
                        .insert(key, entry.call_hierarchy, entry.language)
                        .await?;
                    entries_imported += 1;
                }
            }

            bytes_imported += entry_size;
        }

        let duration = start_time.elapsed();

        // Update internal stats
        {
            let mut stats = self.stats.write().await;
            stats.import_operations += 1;
            stats.last_import_duration_ms = duration.as_millis() as u64;
        }

        let result = ImportResult {
            entries_imported,
            entries_merged,
            entries_replaced,
            validation_errors,
            bytes_imported,
            duration_ms: duration.as_millis() as u64,
        };

        info!("Cache import completed: {:?}", result);
        Ok(result)
    }

    /// Compact the cache database
    pub async fn compact(&self, options: CompactOptions) -> Result<CompactResult> {
        let start_time = Instant::now();

        info!("Compacting cache with options: {:?}", options);

        let stats_before = self.persistent_store.get_stats().await?;
        let size_before_bytes = stats_before.total_size_bytes;

        let mut expired_entries_removed = 0u64;
        let mut size_based_entries_removed = 0u64;

        // Clean up expired entries if requested
        if options.clean_expired {
            info!("Removing expired entries during compaction");
            expired_entries_removed = self.persistent_store.cleanup_expired().await? as u64;
        }

        // Size-based cleanup if target size specified
        if let Some(target_size_mb) = options.target_size_mb {
            let target_size_bytes = (target_size_mb * 1024 * 1024) as u64;
            let current_stats = self.persistent_store.get_stats().await?;

            if current_stats.total_size_bytes > target_size_bytes {
                info!(
                    "Current cache size ({} bytes) exceeds target ({} bytes), removing oldest entries",
                    current_stats.total_size_bytes,
                    target_size_bytes
                );

                // This is a simplified approach - in a real implementation,
                // you'd want to implement LRU-based eviction in the persistent cache
                let excess_bytes = current_stats.total_size_bytes - target_size_bytes;
                let estimated_entries_to_remove =
                    (excess_bytes * current_stats.total_nodes) / current_stats.total_size_bytes;

                warn!(
                    "Size-based cleanup not fully implemented - would remove approximately {} entries",
                    estimated_entries_to_remove
                );

                // For now, we'll just record what would be removed
                size_based_entries_removed =
                    estimated_entries_to_remove.min(current_stats.total_nodes);
            }
        }

        // Perform database compaction
        info!("Performing database compaction");
        self.persistent_store
            .compact()
            .await
            .context("Failed to compact database")?;

        let stats_after = self.persistent_store.get_stats().await?;
        let size_after_bytes = stats_after.total_size_bytes;
        let bytes_reclaimed = size_before_bytes.saturating_sub(size_after_bytes);

        // Calculate fragmentation reduction (simplified metric)
        let fragmentation_reduced = if size_before_bytes > 0 {
            (bytes_reclaimed as f64 / size_before_bytes as f64) * 100.0
        } else {
            0.0
        };

        let duration = start_time.elapsed();

        // Update internal stats
        {
            let mut stats = self.stats.write().await;
            stats.compact_operations += 1;
            stats.last_compact_duration_ms = duration.as_millis() as u64;
        }

        let result = CompactResult {
            expired_entries_removed,
            size_based_entries_removed,
            size_before_bytes,
            size_after_bytes,
            bytes_reclaimed,
            fragmentation_reduced,
            duration_ms: duration.as_millis() as u64,
        };

        info!("Cache compaction completed: {:?}", result);
        Ok(result)
    }

    // Helper methods for statistics building

    async fn build_entries_per_file_map(&self) -> Result<HashMap<PathBuf, u64>> {
        let mut map = HashMap::new();
        let nodes = self.persistent_store.iter_nodes().await?;

        for (key, _) in nodes {
            *map.entry(key.file).or_insert(0) += 1;
        }

        Ok(map)
    }

    async fn build_entries_per_language_map(&self) -> Result<HashMap<String, u64>> {
        let mut map = HashMap::new();
        let nodes = self.persistent_store.iter_nodes().await?;

        for (_, node) in nodes {
            let language_str = format!("{:?}", node.language);
            *map.entry(language_str).or_insert(0) += 1;
        }

        Ok(map)
    }

    async fn build_age_distribution(&self) -> Result<AgeDistribution> {
        let now = SystemTime::now();
        let mut distribution = AgeDistribution {
            entries_last_hour: 0,
            entries_last_day: 0,
            entries_last_week: 0,
            entries_last_month: 0,
            entries_older: 0,
        };

        let nodes = self.persistent_store.iter_nodes().await?;

        for (_, node) in nodes {
            if let Ok(age) = now.duration_since(node.created_at) {
                if age < Duration::from_secs(3600) {
                    distribution.entries_last_hour += 1;
                } else if age < Duration::from_secs(86400) {
                    distribution.entries_last_day += 1;
                } else if age < Duration::from_secs(604800) {
                    distribution.entries_last_week += 1;
                } else if age < Duration::from_secs(2592000) {
                    distribution.entries_last_month += 1;
                } else {
                    distribution.entries_older += 1;
                }
            }
        }

        Ok(distribution)
    }

    async fn build_hot_spots(&self) -> Result<Vec<HotSpot>> {
        // This is a simplified implementation - in a real system,
        // you'd track access counts and timestamps
        let mut hot_spots = Vec::new();
        let nodes = self.persistent_store.iter_nodes().await?;

        // Take first 10 nodes as "hot spots" for demonstration
        for (key, node) in nodes.into_iter().take(10) {
            hot_spots.push(HotSpot {
                file_path: key.file,
                symbol: key.symbol,
                access_count: 1,       // Simplified - would track real access counts
                hit_rate: 1.0,         // Simplified - would calculate real hit rate
                branches_seen: vec![], // Would track real branch history
                commits_seen: 0,       // Would track real commit count
                first_seen: node
                    .created_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                last_accessed: node
                    .created_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }

        Ok(hot_spots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_types::LspOperation;
    use crate::call_graph_cache::{CallGraphCache, CallGraphCacheConfig};
    use crate::lsp_cache::{LspCache, LspCacheConfig};
    use crate::persistent_cache::{PersistentCacheConfig, PersistentCallGraphCache};
    use std::time::Duration;
    use tempfile::tempdir;

    async fn create_test_manager() -> CacheManager {
        let temp_dir = tempdir().unwrap();
        let config = PersistentCacheConfig {
            cache_directory: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        };

        let persistent_store = Arc::new(PersistentCallGraphCache::new(config).await.unwrap());

        let call_graph_config = CallGraphCacheConfig {
            persistence_enabled: false,
            ..Default::default()
        };
        let call_graph_cache = Arc::new(CallGraphCache::new(call_graph_config));

        let lsp_config = LspCacheConfig {
            capacity_per_operation: 100,
            ttl: Duration::from_secs(3600),
            eviction_check_interval: Duration::from_secs(300),
            persistent: false,
            cache_directory: None,
        };

        let definition_cache =
            Arc::new(LspCache::new(LspOperation::Definition, lsp_config.clone()).unwrap());
        let references_cache =
            Arc::new(LspCache::new(LspOperation::References, lsp_config.clone()).unwrap());
        let hover_cache = Arc::new(LspCache::new(LspOperation::Hover, lsp_config).unwrap());

        CacheManager::new(
            call_graph_cache,
            persistent_store,
            definition_cache,
            references_cache,
            hover_cache,
        )
    }

    #[tokio::test]
    async fn test_cache_manager_creation() {
        let manager = create_test_manager().await;
        let stats = manager.get_stats(false, false).await.unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    #[tokio::test]
    async fn test_clear_all() {
        let manager = create_test_manager().await;

        let filter = ClearFilter {
            older_than_days: None,
            file_path: None,
            commit_hash: None,
            all: true,
        };

        let result = manager.clear(filter).await.unwrap();
        assert_eq!(result.entries_removed, 0); // Empty cache
        assert!(result.duration_ms > 0);
    }

    #[tokio::test]
    async fn test_export_import_roundtrip() {
        let manager = create_test_manager().await;
        let temp_dir = tempdir().unwrap();
        let export_path = temp_dir.path().join("cache_export.bin");

        // Export (empty cache)
        let options = ExportOptions {
            current_branch_only: false,
            compress: false,
        };
        manager.export(&export_path, options).await.unwrap();

        // Import
        let result = manager.import(&export_path, false).await.unwrap();
        assert_eq!(result.entries_imported, 0); // Empty export
        assert!(result.validation_errors.is_empty());
    }

    #[tokio::test]
    async fn test_compact() {
        let manager = create_test_manager().await;

        let options = CompactOptions {
            clean_expired: true,
            target_size_mb: None,
        };

        let result = manager.compact(options).await.unwrap();
        assert_eq!(result.expired_entries_removed, 0); // No expired entries
        assert!(result.duration_ms > 0);
    }
}
