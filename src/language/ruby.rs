use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for Ruby
pub struct RubyLanguage;

impl Default for RubyLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl RubyLanguage {
    pub fn new() -> Self {
        RubyLanguage
    }
}

impl LanguageImpl for RubyLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_ruby::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "rb"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "method" | "class" | "module" | "singleton_method"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // Ruby: Check method nodes with test_ prefix or describe/it blocks
        if node_type == "method" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("test_") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Ruby): test_ method");
                        }
                        return true;
                    }
                }
            }
        } else if node_type == "call" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name == "describe" || name == "it" || name == "context" || name == "specify"
                    {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Ruby): {name} block");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }
}
