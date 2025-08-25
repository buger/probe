/// Function to merge overlapping code blocks
#[cfg(test)]
pub fn merge_code_blocks(
    code_blocks: Vec<crate::models::CodeBlock>,
) -> Vec<crate::models::CodeBlock> {
    let mut merged_blocks: Vec<crate::models::CodeBlock> = Vec::new();
    let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

    for block in code_blocks {
        if let Some(last) = merged_blocks.last_mut() {
            // Use a consistent threshold of 10 lines for all block types
            let threshold = 10;

            if block.start_row <= last.end_row + threshold {
                if debug_mode {
                    println!(
                        "DEBUG: Merging blocks: {} ({}-{}) with {} ({}-{})",
                        last.node_type,
                        last.start_row + 1,
                        last.end_row + 1,
                        block.node_type,
                        block.start_row + 1,
                        block.end_row + 1
                    );
                }
                last.end_row = last.end_row.max(block.end_row);
                last.end_byte = last.end_byte.max(block.end_byte);
                last.start_row = last.start_row.min(block.start_row);
                last.start_byte = last.start_byte.min(block.start_byte);
                continue;
            }
        }
        merged_blocks.push(block);
    }

    if debug_mode {
        println!(
            "DEBUG: After merging: {len} blocks",
            len = merged_blocks.len()
        );
        for (i, block) in merged_blocks.iter().enumerate() {
            println!(
                "DEBUG:   Block {}: type={}, lines={}-{}",
                i + 1,
                block.node_type,
                block.start_row + 1,
                block.end_row + 1
            );
        }
    }
    merged_blocks
}
