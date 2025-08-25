//! BERT-based reranking module
//!
//! This module provides BERT reranking functionality using the ms-marco-TinyBERT model
//! for more semantic relevance scoring compared to traditional BM25 ranking.

#[cfg(feature = "bert-reranker")]
use anyhow::{Context, Result};
#[cfg(feature = "bert-reranker")]
use candle_core::{Device, IndexOp, Tensor};
#[cfg(feature = "bert-reranker")]
use candle_nn::{linear, Linear, Module, VarBuilder};
#[cfg(feature = "bert-reranker")]
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
#[cfg(feature = "bert-reranker")]
use hf_hub::{api::tokio::Api, Repo, RepoType};
#[cfg(feature = "bert-reranker")]
use serde_json;
#[cfg(feature = "bert-reranker")]
use std::path::Path;
#[cfg(feature = "bert-reranker")]
use tokenizers::Tokenizer;

use crate::models::SearchResult;

#[cfg(feature = "bert-reranker")]
pub struct BertReranker {
    bert: BertModel,
    classifier: Linear,
    tokenizer: Tokenizer,
    device: Device,
    max_length: usize,
}

#[cfg(feature = "bert-reranker")]
impl BertReranker {
    pub async fn new(model_name: &str) -> Result<Self> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        if debug_mode {
            println!("DEBUG: Loading BERT model: {model_name}");
        }

        // Check if we have local model files first
        let model_dir_name = match model_name {
            "cross-encoder/ms-marco-TinyBERT-L-2-v2" => "ms-marco-TinyBERT-L-2-v2",
            "cross-encoder/ms-marco-MiniLM-L-6-v2" => "ms-marco-MiniLM-L-6-v2",
            "cross-encoder/ms-marco-MiniLM-L-12-v2" => "ms-marco-MiniLM-L-12-v2",
            "cross-encoder/ms-marco-MiniLM-L-2-v2" => "ms-marco-MiniLM-L-2-v2",
            _ => "ms-marco-MiniLM-L-2-v2", // Default fallback
        };

        let local_model_dir = Path::new("examples/reranker/models").join(model_dir_name);
        let (config_path, tokenizer_path, weights_path) = if local_model_dir.exists() {
            if debug_mode {
                println!("DEBUG: Using local model files from: {local_model_dir:?}");
            }
            (
                local_model_dir.join("config.json"),
                local_model_dir.join("tokenizer.json"),
                local_model_dir.join("pytorch_model.bin"),
            )
        } else {
            if debug_mode {
                println!("DEBUG: Downloading model files from HuggingFace Hub...");
            }

            let api = Api::new()?;
            let repo = api.repo(Repo::with_revision(
                model_name.to_string(),
                RepoType::Model,
                "main".to_string(),
            ));

            // Download model files
            let config_path = repo
                .get("config.json")
                .await
                .context("Failed to download config.json")?;
            let tokenizer_path = repo
                .get("tokenizer.json")
                .await
                .context("Failed to download tokenizer.json")?;

            // Try different weight file formats
            let weights_path = match repo.get("model.safetensors").await {
                Ok(path) => {
                    if debug_mode {
                        println!("DEBUG: Using model.safetensors");
                    }
                    path
                }
                Err(_) => match repo.get("pytorch_model.bin").await {
                    Ok(path) => {
                        if debug_mode {
                            println!("DEBUG: Using pytorch_model.bin");
                        }
                        path
                    }
                    Err(e) => {
                        if debug_mode {
                            println!("DEBUG: Trying model.bin as fallback...");
                        }
                        repo.get("model.bin")
                            .await
                            .context(format!("Could not find model weights: {e}"))?
                    }
                },
            };

            (config_path, tokenizer_path, weights_path)
        };

        if debug_mode {
            println!("DEBUG: Loading configuration...");
        }
        // Load configuration
        let config_content =
            std::fs::read_to_string(&config_path).context("Failed to read config file")?;
        let config: Config =
            serde_json::from_str(&config_content).context("Failed to parse model configuration")?;

        let max_length = config.max_position_embeddings.min(512); // Limit for performance
        if debug_mode {
            println!(
                "DEBUG: Model config loaded - max_length: {}, hidden_size: {}",
                max_length, config.hidden_size
            );
        }

        if debug_mode {
            println!("DEBUG: Loading tokenizer...");
        }
        // Load tokenizer
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;

        if debug_mode {
            println!("DEBUG: Setting up device and loading model weights...");
        }
        // Setup device (CPU for compatibility, could be GPU if available)
        let device = Device::Cpu;
        let dtype = DTYPE;

        // Load model weights
        let vb = if weights_path.extension() == Some(std::ffi::OsStr::new("safetensors")) {
            unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path], dtype, &device)? }
        } else {
            VarBuilder::from_pth(&weights_path, dtype, &device)?
        };

        if debug_mode {
            println!("DEBUG: Creating BERT model...");
        }
        // Create BERT model
        // MS MARCO models typically have the BERT weights at root level
        let bert = match BertModel::load(vb.clone(), &config) {
            Ok(model) => {
                if debug_mode {
                    println!("DEBUG: Loaded BERT model from root level");
                }
                model
            }
            Err(_) => {
                if debug_mode {
                    println!("DEBUG: Failed to load BERT from root, trying 'bert' prefix");
                }
                BertModel::load(vb.pp("bert"), &config)
                    .context("Failed to load BERT model from any path")?
            }
        };

        if debug_mode {
            println!("DEBUG: Creating classification head...");
        }
        // Create classification head (linear layer for sequence classification)
        // Try different paths for the classifier weights
        let classifier = match linear(config.hidden_size, 1, vb.pp("classifier")) {
            Ok(classifier) => {
                if debug_mode {
                    println!("DEBUG: Successfully loaded classifier from 'classifier' path");
                }
                classifier
            }
            Err(_) => {
                // Try alternative paths
                if debug_mode {
                    println!(
                        "DEBUG: Failed to load classifier from 'classifier' path, trying 'pooler'"
                    );
                }
                match linear(config.hidden_size, 1, vb.pp("pooler")) {
                    Ok(classifier) => {
                        if debug_mode {
                            println!("DEBUG: Successfully loaded classifier from 'pooler' path");
                        }
                        classifier
                    }
                    Err(_) => {
                        if debug_mode {
                            println!("DEBUG: Failed to load classifier from 'pooler' path, trying root level");
                        }
                        // Try root level
                        linear(config.hidden_size, 1, vb.clone())
                            .context("Failed to create classification head from any path")?
                    }
                }
            }
        };

        if debug_mode {
            println!("DEBUG: BERT reranker loaded successfully!");
        }

        Ok(Self {
            bert,
            classifier,
            tokenizer,
            device,
            max_length,
        })
    }

    pub async fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<(usize, f32)>> {
        let mut scores = Vec::new();

        for (idx, document) in documents.iter().enumerate() {
            let score = self
                .score_pair(query, document)
                .context(format!("Failed to score document {idx}"))?;
            scores.push((idx, score));
        }

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scores)
    }

    fn score_pair(&self, query: &str, document: &str) -> Result<f32> {
        // Truncate document if too long (keep query + document under max_length)
        let max_doc_length = self
            .max_length
            .saturating_sub(query.len() / 4)
            .saturating_sub(10); // rough estimate
        let doc_truncated = if document.len() > max_doc_length {
            // Find a valid UTF-8 character boundary at or before max_doc_length
            let mut truncate_at = max_doc_length;
            while truncate_at > 0 && !document.is_char_boundary(truncate_at) {
                truncate_at -= 1;
            }
            &document[..truncate_at]
        } else {
            document
        };

        // Prepare input for cross-encoder using encode_pair
        // This ensures proper token type IDs are generated automatically
        let mut encoding = self
            .tokenizer
            .encode((query, doc_truncated), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        // Truncate if too long
        if encoding.len() > self.max_length {
            encoding.truncate(self.max_length, 0, tokenizers::TruncationDirection::Right);
        }

        // Pad to max length for consistent batching
        use tokenizers::PaddingDirection;
        encoding.pad(self.max_length, 0, 0, "[PAD]", PaddingDirection::Right);

        // Convert to tensors
        let input_ids = Tensor::new(encoding.get_ids().to_vec(), &self.device)?.unsqueeze(0)?; // Add batch dimension [1, seq_len]

        let attention_mask =
            Tensor::new(encoding.get_attention_mask().to_vec(), &self.device)?.unsqueeze(0)?; // Add batch dimension [1, seq_len]

        // Token type IDs should be automatically generated by encode_pair
        let token_type_ids = if !encoding.get_type_ids().is_empty() {
            Some(Tensor::new(encoding.get_type_ids().to_vec(), &self.device)?.unsqueeze(0)?)
        } else {
            // If tokenizer doesn't provide type IDs, we might have an issue
            // Log a warning in debug mode
            let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
            if debug_mode {
                println!("WARNING: Tokenizer did not generate token type IDs. This may affect model performance.");
            }
            None
        };

        // Forward pass through BERT
        let bert_outputs =
            self.bert
                .forward(&input_ids, &attention_mask, token_type_ids.as_ref())?;

        // Get [CLS] token representation (first token)
        let cls_output = bert_outputs.i((.., 0, ..))?; // [batch_size, hidden_size]

        // Pass through classification head
        let logits = self.classifier.forward(&cls_output)?; // [batch_size, 1]

        // Get the relevance score
        let raw_score = logits.i((0, 0))?.to_scalar::<f32>()?;

        // Note: MS MARCO cross-encoder models output raw logits without sigmoid
        // The config shows "sbert_ce_default_activation_function": "torch.nn.modules.linear.Identity"
        // So we use the raw score directly
        let score = raw_score;

        // Debug: Log raw score
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        if debug_mode {
            println!(
                "DEBUG: Raw BERT score for query '{}' (first 50 chars): {:.6}",
                &query[..query.len().min(50)],
                score
            );
        }

        Ok(score)
    }
}

// Non-feature version that provides error message
#[cfg(not(feature = "bert-reranker"))]
pub struct BertReranker;

#[cfg(not(feature = "bert-reranker"))]
impl BertReranker {
    pub async fn new(_model_name: &str) -> Result<Self, anyhow::Error> {
        Err(anyhow::anyhow!(
            "BERT reranker is not available. Build with --features bert-reranker to enable."
        ))
    }

    pub async fn rerank(
        &self,
        _query: &str,
        _documents: &[&str],
    ) -> Result<Vec<(usize, f32)>, anyhow::Error> {
        Err(anyhow::anyhow!(
            "BERT reranker is not available. Build with --features bert-reranker to enable."
        ))
    }
}

/// Rerank search results using BERT-based semantic similarity with parallelization
#[cfg(feature = "bert-reranker")]
#[allow(dead_code)]
pub async fn rerank_with_bert(
    results: &mut [SearchResult],
    queries: &[String],
    model_name: &str,
    question: Option<&str>,
) -> Result<(), anyhow::Error> {
    if results.is_empty() {
        return Ok(());
    }

    // Use the existing ParallelBertReranker for better performance
    // Create a parallel reranker with auto-detected thread count
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8); // Cap at 8 threads to avoid overwhelming the system

    let parallel_reranker = ParallelBertReranker::new(model_name, Some(num_threads)).await?;

    // Use the question if provided, otherwise join the queries
    let combined_query = if let Some(q) = question {
        q.to_string()
    } else {
        queries.join(" ")
    };

    // Extract document texts for ranking, including filename
    let documents: Vec<String> = results
        .iter()
        .map(|r| {
            let mut doc = String::with_capacity(r.file.len() + r.code.len() + 15);
            doc.push_str("// Filename: ");
            doc.push_str(&r.file);
            doc.push('\n');
            doc.push_str(&r.code);
            doc
        })
        .collect();

    let documents_refs: Vec<&str> = documents.iter().map(|s| s.as_str()).collect();

    // Get BERT scores using parallel processing
    let ranked_indices = parallel_reranker.rerank_parallel(&combined_query, &documents_refs)?;

    // Update results with BERT scores and rankings
    for (rank_index, (original_index, bert_score)) in ranked_indices.iter().enumerate() {
        if let Some(result) = results.get_mut(*original_index) {
            result.rank = Some(rank_index + 1); // 1-based rank
            result.score = Some(*bert_score as f64);
            result.bm25_score = Some(*bert_score as f64); // Store BERT score in bm25_score field for consistency
        }
    }

    // Debug: Show scores before sorting
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
    if debug_mode {
        println!("\nDEBUG: BERT scores before sorting:");
        for (i, result) in results.iter().enumerate() {
            println!(
                "  [{}] {} - Score: {:.6}",
                i,
                &result.file[result.file.len().saturating_sub(50)..], // Last 50 chars of filename
                result.score.unwrap_or(0.0)
            );
        }
    }

    // Sort results by BERT score (descending)
    results.sort_by(|a, b| {
        let score_a = a.score.unwrap_or(0.0);
        let score_b = b.score.unwrap_or(0.0);
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if debug_mode {
        println!("\nDEBUG: BERT scores after sorting:");
        for (i, result) in results.iter().enumerate() {
            println!(
                "  [{}] {} - Score: {:.6}",
                i,
                &result.file[result.file.len().saturating_sub(50)..], // Last 50 chars of filename
                result.score.unwrap_or(0.0)
            );
        }
    }

    // Update ranks after sorting
    for (rank, result) in results.iter_mut().enumerate() {
        result.rank = Some(rank + 1);
        result.bm25_rank = Some(rank + 1);
    }

    Ok(())
}

/// Non-feature version of rerank_with_bert
#[cfg(not(feature = "bert-reranker"))]
#[allow(dead_code)]
pub async fn rerank_with_bert(
    _results: &mut [SearchResult],
    _queries: &[String],
    _model_name: &str,
    _question: Option<&str>,
) -> Result<(), anyhow::Error> {
    Err(anyhow::anyhow!(
        "BERT reranker is not available. Build with --features bert-reranker to enable."
    ))
}

#[cfg(feature = "bert-reranker")]
pub struct ParallelBertReranker {
    engines: Vec<std::sync::Arc<parking_lot::Mutex<BertInferenceEngine>>>,
    num_threads: usize,
}

#[cfg(feature = "bert-reranker")]
pub struct BertInferenceEngine {
    reranker: BertReranker,
}

#[cfg(feature = "bert-reranker")]
impl BertInferenceEngine {
    pub async fn new(model_name: &str) -> Result<Self> {
        let reranker = BertReranker::new(model_name).await?;
        Ok(Self { reranker })
    }

    pub fn score_pair(&self, query: &str, document: &str) -> Result<f32> {
        self.reranker.score_pair(query, document)
    }
}

#[cfg(feature = "bert-reranker")]
impl ParallelBertReranker {
    pub async fn new(model_name: &str, num_threads: Option<usize>) -> Result<Self> {
        let num_threads = num_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8) // Cap at 8 threads
        });

        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        if debug_mode {
            println!("DEBUG: Creating parallel BERT reranker with {num_threads} engines");
        }

        let mut engines = Vec::new();
        for i in 0..num_threads {
            if debug_mode {
                println!(
                    "DEBUG: Loading BERT model for engine {} of {}",
                    i + 1,
                    num_threads
                );
            }
            let engine = BertInferenceEngine::new(model_name).await?;
            engines.push(std::sync::Arc::new(parking_lot::Mutex::new(engine)));
        }

        Ok(Self {
            engines,
            num_threads,
        })
    }

    pub fn rerank_parallel(&self, query: &str, documents: &[&str]) -> Result<Vec<(usize, f32)>> {
        use rayon::prelude::*;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let total_docs = documents.len();

        if debug_mode {
            println!(
                "DEBUG: Starting BERT reranking for {} documents with {} engines (num_threads={})",
                total_docs,
                self.engines.len(),
                self.num_threads
            );
        }

        // Progress tracking variables - only create when in debug mode
        let processed_count = if debug_mode {
            Some(Arc::new(AtomicUsize::new(0)))
        } else {
            None
        };
        let processing_complete = if debug_mode {
            Some(Arc::new(AtomicBool::new(false)))
        } else {
            None
        };
        let start_time = Instant::now();

        // Spawn a progress reporter thread if in debug mode
        let progress_reporter = if let (Some(ref processed_count), Some(ref processing_complete)) =
            (&processed_count, &processing_complete)
        {
            let processed_count_clone = processed_count.clone();
            let processing_complete_clone = processing_complete.clone();
            let handle = std::thread::spawn(move || {
                let mut last_reported = 0;
                let mut iterations = 0;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    iterations += 1;

                    // Safety timeout - exit after 60 seconds
                    if iterations > 60 {
                        eprintln!("WARNING: Progress reporter timeout after 60 seconds");
                        break;
                    }

                    // Check if processing is complete
                    if processing_complete_clone.load(Ordering::Acquire) {
                        // Report final progress if needed
                        let current = processed_count_clone.load(Ordering::Relaxed);
                        if current > last_reported && current > 0 {
                            let elapsed = start_time.elapsed();
                            let rate = current as f64 / elapsed.as_secs_f64();
                            println!("DEBUG: BERT reranking progress: {current}/{total_docs} documents processed ({rate:.1} docs/sec)");
                        }
                        break;
                    }

                    let current = processed_count_clone.load(Ordering::Relaxed);
                    if current > last_reported {
                        let elapsed = start_time.elapsed();
                        let rate = current as f64 / elapsed.as_secs_f64();
                        println!("DEBUG: BERT reranking progress: {current}/{total_docs} documents processed ({rate:.1} docs/sec)");
                        last_reported = current;
                    }
                }
            });
            Some(handle)
        } else {
            None
        };

        if debug_mode {
            eprintln!("DEBUG: About to start parallel processing of {total_docs} documents");
        }

        // Create chunks for parallel processing - each chunk is processed by a dedicated engine
        let chunk_size = total_docs.div_ceil(self.engines.len());
        let chunks: Vec<_> = documents
            .iter()
            .enumerate()
            .collect::<Vec<_>>()
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        if debug_mode {
            println!(
                "DEBUG: Created {} chunks of size ~{}",
                chunks.len(),
                chunk_size
            );
        }

        // Process chunks in parallel - each chunk is assigned to a specific engine
        let query = query.to_string(); // Clone for thread safety
        let engines = Arc::new(&self.engines);

        let results: Result<Vec<Vec<(usize, f32)>>> = chunks
            .into_par_iter()
            .enumerate()
            .map(|(chunk_idx, chunk)| {
                let engine_idx = chunk_idx % self.engines.len();
                let engine = &engines[engine_idx];

                if debug_mode {
                    println!(
                        "DEBUG: Thread {} processing chunk {} with {} documents using engine {}",
                        chunk_idx,
                        chunk_idx,
                        chunk.len(),
                        engine_idx
                    );
                }

                let mut chunk_results = Vec::new();

                // Lock the engine for this entire chunk to avoid contention
                let engine_guard = engine.lock();

                for (doc_idx, document) in chunk {
                    let score = engine_guard
                        .score_pair(&query, document)
                        .with_context(|| format!("Failed to score document {doc_idx}"))?;
                    chunk_results.push((doc_idx, score));

                    // Update progress counter
                    if let Some(ref count) = processed_count {
                        let new_count = count.fetch_add(1, Ordering::Relaxed) + 1;
                        if debug_mode && new_count % 10 == 0 {
                            eprintln!("DEBUG: Processed {new_count} documents so far");
                        }
                    }
                }

                drop(engine_guard); // Explicitly release lock

                if debug_mode {
                    println!(
                        "DEBUG: Chunk {} completed processing {} documents",
                        chunk_idx,
                        chunk_results.len()
                    );
                }

                Ok(chunk_results)
            })
            .collect();

        if debug_mode {
            eprintln!("DEBUG: All chunks processed, flattening results...");
        }

        // Signal that processing is complete
        if let Some(ref complete) = processing_complete {
            complete.store(true, Ordering::Release);
        }

        // Wait for progress reporter to finish
        if let Some(handle) = progress_reporter {
            let _ = handle.join();
        }

        // Flatten results and sort
        let mut all_scores: Vec<(usize, f32)> = results?.into_iter().flatten().collect();

        if debug_mode {
            let elapsed = start_time.elapsed();
            let rate = total_docs as f64 / elapsed.as_secs_f64();
            println!(
                "DEBUG: BERT reranking completed: {} documents in {:.2}s ({:.1} docs/sec)",
                total_docs,
                elapsed.as_secs_f64(),
                rate
            );
        }

        // Sort by score descending
        all_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if debug_mode {
            println!(
                "DEBUG: Parallel processing complete, {} results sorted",
                all_scores.len()
            );
        }

        Ok(all_scores)
    }
}
