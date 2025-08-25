use std::path::Path;

fn main() {
    let content = std::fs::read_to_string("src/lsp_integration/client.rs").unwrap();
    let lines: Vec<&str> = content.lines().collect();
    
    // Print the area around line 451
    for i in 448..455 {
        if i < lines.len() {
            println!("Line {}: '{}'", i + 1, lines[i]);
        }
    }
    
    // Now let's see what the tree-sitter node contains
    println!("\nAnalyzing with tree-sitter...");
    
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_rust::language();
    parser.set_language(language).unwrap();
    
    let tree = parser.parse(content.as_bytes(), None).unwrap();
    let root = tree.root_node();
    
    // Find the get_symbol_info function
    find_function(&root, "get_symbol_info", content.as_bytes());
}

fn find_function(node: tree_sitter::Node, target_name: &str, content: &[u8]) {
    if node.kind() == "function_item" {
        // Try to find the function name
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" {
                if let Ok(name) = child.utf8_text(content) {
                    if name == target_name {
                        println!("Found function '{}' at node:", target_name);
                        println!("  Node range: {}:{} - {}:{}", 
                            node.start_position().row + 1, node.start_position().column + 1,
                            node.end_position().row + 1, node.end_position().column + 1);
                        println!("  Identifier range: {}:{} - {}:{}", 
                            child.start_position().row + 1, child.start_position().column + 1,
                            child.end_position().row + 1, child.end_position().column + 1);
                        
                        // Show the node text
                        let node_text = &content[node.start_byte()..node.end_byte()];
                        if let Ok(text) = std::str::from_utf8(node_text) {
                            let lines: Vec<&str> = text.lines().collect();
                            println!("  Node starts with:");
                            for (i, line) in lines.iter().enumerate().take(5) {
                                println!("    Line {}: '{}'", i + 1, line);
                            }
                        }
                        return;
                    }
                }
            }
        }
    }
    
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_function(child, target_name, content);
    }
}
