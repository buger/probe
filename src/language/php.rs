use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for PHP
pub struct PhpLanguage;

impl Default for PhpLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl PhpLanguage {
    pub fn new() -> Self {
        PhpLanguage
    }
}

impl LanguageImpl for PhpLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_php::LANGUAGE_PHP.into()
    }

    fn get_extension(&self) -> &'static str {
        "php"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_definition"
                | "method_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "trait_declaration"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // PHP: Check method_declaration nodes with test prefix or PHPUnit annotations
        if node_type == "method_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "name" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (PHP): test method");
                        }
                        return true;
                    }
                } else if child.kind() == "comment" {
                    let comment = child.utf8_text(source).unwrap_or("");
                    if comment.contains("@test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (PHP): @test annotation");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }
}
