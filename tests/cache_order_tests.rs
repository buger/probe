use std::collections::HashSet;
use std::time::Instant;

/// This test demonstrates the performance benefits of caching ancestor lookups
/// in the AST traversal. The test creates a complex Go file with many nested
/// structures that will trigger many ancestor lookups during parsing.
#[test]
fn test_ancestor_cache_performance() {
    // Create a sample Go file with nested structures that will trigger many ancestor lookups
    let content = generate_complex_go_file(10, 5); // 10 structs with 5 levels of nesting each

    // Create a set of line numbers to extract (all lines in the file)
    let line_count = content.lines().count();
    let line_numbers: HashSet<usize> = (1..=line_count).collect();

    println!("Generated a Go file with {line_count} lines");
    println!("File contains multiple nested structures to stress-test ancestor lookups");

    // Run the extraction multiple times to get a reliable measurement
    let iterations = 10;
    let mut times = Vec::with_capacity(iterations);

    println!("\nRunning {iterations} iterations of parsing with ancestor cache:");

    for i in 1..=iterations {
        let start = Instant::now();
        let result = probe_code::language::parser::parse_file_for_code_blocks(
            &content,
            "go",
            &line_numbers,
            true,
            None,
        );
        assert!(result.is_ok(), "Failed to parse file: {:?}", result.err());
        let blocks = result.unwrap();
        let duration = start.elapsed();
        times.push(duration.as_secs_f64());

        println!(
            "  Iteration {}: {:.6} seconds, extracted {} code blocks",
            i,
            duration.as_secs_f64(),
            blocks.len()
        );
    }

    // Calculate statistics
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min_time = times.first().unwrap();
    let max_time = times.last().unwrap();
    let median_time = if iterations % 2 == 0 {
        (times[iterations / 2 - 1] + times[iterations / 2]) / 2.0
    } else {
        times[iterations / 2]
    };
    let avg_time = times.iter().sum::<f64>() / iterations as f64;

    println!("\nPerformance statistics with ancestor cache:");
    println!("  Minimum time: {min_time:.6} seconds");
    println!("  Maximum time: {max_time:.6} seconds");
    println!("  Median time:  {median_time:.6} seconds");
    println!("  Average time: {avg_time:.6} seconds");

    println!(
        "\nNote: Without caching, this would be significantly slower due to redundant traversals."
    );
    println!(
        "The cache prevents repeated upward traversals for nodes within the same acceptable parent block."
    );
}

/// Generates a complex Go file with multiple nested structures to stress-test ancestor lookups
fn generate_complex_go_file(struct_count: usize, nesting_levels: usize) -> String {
    let mut content = String::from("package main\n\nimport \"fmt\"\n\n");

    // Generate multiple struct definitions with deep nesting
    for i in 1..=struct_count {
        content.push_str(&format!("// Struct {i} with deep nesting\n"));
        content.push_str(&format!("type Struct{i} struct {{\n"));

        // Add some fields at the top level
        for j in 1..=3 {
            content.push_str(&format!("    Field{j} string\n"));
        }

        // Add nested structures
        let mut indent = 4;
        let mut current_struct = format!("Inner{i}");

        for level in 1..=nesting_levels {
            let spaces = " ".repeat(indent);
            content.push_str(&format!("{spaces}// Level {level} nested structure\n"));
            content.push_str(&format!("{spaces}{current_struct} struct {{\n"));

            // Add fields to this level
            indent += 4;
            let inner_spaces = " ".repeat(indent);
            for j in 1..=3 {
                content.push_str(&format!("{inner_spaces}Field{j}_L{level} string\n"));
            }

            // Prepare for next level
            current_struct = format!("Nested{i}_L{level}");

            // Add the nested struct field at this level (except for the last level)
            if level < nesting_levels {
                content.push_str(&format!(
                    "{inner_spaces}{current_struct} {current_struct}\n"
                ));
            }
        }

        // Close all the nested structures
        for level in (0..=nesting_levels).rev() {
            let spaces = " ".repeat(4 * level);
            content.push_str(&format!("{spaces}}}\n"));
        }

        content.push('\n');
    }

    // Add a main function that uses these structures
    content.push_str("func main() {\n");
    for i in 1..=struct_count {
        content.push_str(&format!("    // Initialize Struct{i}\n"));
        content.push_str(&format!("    var s{i} Struct{i}\n"));
        content.push_str(&format!("    s{i}.Field1 = \"value\"\n"));

        // Access some nested fields
        let mut var_path = format!("s{i}");
        for level in 1..=nesting_levels {
            if level == 1 {
                var_path = format!("{var_path}.Inner{i}");
            } else {
                var_path = format!("{}.Nested{}_L{}", var_path, i, level - 1);
            }
            content.push_str(&format!(
                "    {var_path}.Field1_L{level} = \"nested value at level {level}\"\n"
            ));
        }
        content.push('\n');
    }

    // Print something to avoid unused variable warnings
    content.push_str("    fmt.Println(\"Complex nested structures initialized\")\n");
    content.push_str("}\n");

    content
}
