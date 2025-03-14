use anyhow::Result;
use tree_sitter::Parser as TSParser;

// Helper function to parse Go code and get the root node
fn parse_go_code(code: &str) -> tree_sitter::Tree {
    let mut parser = TSParser::new();
    let language = tree_sitter_go::LANGUAGE.into();
    parser
        .set_language(&language)
        .expect("Error loading Go grammar");
    parser.parse(code, None).expect("Failed to parse code")
}

fn print_ast_structure(node: tree_sitter::Node, depth: usize) {
    let indent = " ".repeat(depth * 2);
    println!(
        "{}[{}-{}] {}",
        indent,
        node.start_position().row + 1,
        node.end_position().row + 1,
        node.kind()
    );

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_ast_structure(child, depth + 1);
    }
}

// Helper function to find a comment node in the tree
fn find_comment_node(tree: &tree_sitter::Tree, line: usize) -> Option<tree_sitter::Node<'_>> {
    let root_node = tree.root_node();

    println!("\nAST Structure:");
    print_ast_structure(root_node, 0);

    // Helper function to find a comment node recursively
    fn find_comment_recursive(
        node: tree_sitter::Node<'_>,
        target_line: usize,
    ) -> Option<tree_sitter::Node<'_>> {
        if (node.kind() == "comment"
            || node.kind() == "line_comment"
            || node.kind() == "block_comment")
            && node.start_position().row + 1 == target_line
        {
            return Some(node);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(comment) = find_comment_recursive(child, target_line) {
                return Some(comment);
            }
        }
        None
    }

    find_comment_recursive(root_node, line)
}

#[test]
fn test_go_struct_comments() -> Result<()> {
    let code = r#"
package main

// First struct represents something
type First struct {
    Field string `json:"field"`
}

// Second struct also represents something
type Second struct {
    Data string `json:"data"`
}
"#;

    let tree = parse_go_code(code);

    // Test first comment - should only relate to First struct
    let first_comment = find_comment_node(&tree, 4).expect("Failed to find first comment");
    let first_related = probe::language::parser::find_related_code_node(first_comment, "go")
        .expect("Failed to find related node");
    assert_eq!(first_related.kind(), "type_declaration");
    assert_eq!(first_related.start_position().row + 1, 5); // First struct starts on line 5

    // Test second comment - should only relate to Second struct
    let second_comment = find_comment_node(&tree, 9).expect("Failed to find second comment");
    let second_related = probe::language::parser::find_related_code_node(second_comment, "go")
        .expect("Failed to find related node");
    assert_eq!(second_related.kind(), "type_declaration");
    assert_eq!(second_related.start_position().row + 1, 10); // Second struct starts on line 10

    // Verify the two structs are different
    assert_ne!(
        first_related.start_position().row,
        second_related.start_position().row
    );

    Ok(())
}

#[test]
fn test_go_nested_structs() -> Result<()> {
    let code = r#"
package main

// OuterType represents a container
type OuterType struct {
    // InnerType represents nested data
    InnerType struct {
        Field string `json:"field"`
    }
}
"#;

    let tree = parse_go_code(code);

    // Test outer comment
    let outer_comment = find_comment_node(&tree, 4).expect("Failed to find outer comment");
    let outer_related = probe::language::parser::find_related_code_node(outer_comment, "go")
        .expect("Failed to find related node");
    assert_eq!(outer_related.kind(), "type_declaration");

    // Test inner comment
    let inner_comment = find_comment_node(&tree, 6).expect("Failed to find inner comment");
    let inner_related = probe::language::parser::find_related_code_node(inner_comment, "go")
        .expect("Failed to find related node");
    // The inner comment should be associated with its struct_type
    assert_eq!(inner_related.kind(), "struct_type");

    Ok(())
}

#[test]
fn test_go_mixed_declarations() -> Result<()> {
    let code = r#"
package main

// CommentA describes interface
type InterfaceA interface {
    Method()
}

// CommentB describes struct
type StructB struct {
    Field string
}
"#;

    let tree = parse_go_code(code);

    // Test interface comment
    let interface_comment = find_comment_node(&tree, 4).expect("Failed to find interface comment");
    let interface_related =
        probe::language::parser::find_related_code_node(interface_comment, "go")
            .expect("Failed to find related node");
    assert_eq!(interface_related.kind(), "type_declaration");
    assert_eq!(interface_related.start_position().row + 1, 5);

    // Test struct comment
    let struct_comment = find_comment_node(&tree, 9).expect("Failed to find struct comment");
    let struct_related = probe::language::parser::find_related_code_node(struct_comment, "go")
        .expect("Failed to find related node");
    assert_eq!(struct_related.kind(), "type_declaration");
    assert_eq!(struct_related.start_position().row + 1, 10);

    // Verify they're different declarations
    assert_ne!(
        interface_related.start_position().row,
        struct_related.start_position().row
    );

    Ok(())
}

#[test]
fn test_go_comment_code_block_extraction() -> Result<()> {
    use probe::language::parser::parse_file_for_code_blocks;
    use std::collections::HashSet;

    // Sample code with a comment and struct
    let code = r#"
package main

// DatasourceResponse represents the response for datasource-related operations
// @Description Datasource response model
type DatasourceResponse struct {
    Type       string `json:"type"`
    ID         string `json:"id"`
    Attributes struct {
        Name string `json:"name"`
    }
}
"#;

    println!("Code lines:");
    for (i, line) in code.lines().enumerate() {
        println!("{}: {}", i + 1, line);
    }

    // Create a set with the line number of the comment
    let mut line_numbers = HashSet::new();
    line_numbers.insert(5); // Try the second comment line instead

    // Enable debug mode for this test
    std::env::set_var("DEBUG", "1");

    // Parse the file for code blocks
    let blocks = parse_file_for_code_blocks(code, "go", &line_numbers, true, None)?;

    println!("Found {} blocks:", blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        println!(
            "Block {}: type={}, lines={}-{}",
            i,
            block.node_type,
            block.start_row + 1,
            block.end_row + 1
        );
    }

    // We should have exactly 1 block: the merged comment and struct
    assert_eq!(
        blocks.len(),
        1,
        "Expected exactly 1 block, got {}",
        blocks.len()
    );

    // The block should be a type_declaration (the comment is merged with it)
    assert_eq!(
        blocks[0].node_type, "type_declaration",
        "Block should be a type_declaration"
    );
    assert_eq!(
        blocks[0].start_row + 1,
        5, // Should start at the second comment line
        "Block should start at line 5 (second comment line)"
    );
    assert_eq!(blocks[0].end_row + 1, 12, "Block should end at line 12");

    Ok(())
}
