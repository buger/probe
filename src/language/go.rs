use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for Go
pub struct GoLanguage;

impl Default for GoLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl GoLanguage {
    pub fn new() -> Self {
        GoLanguage
    }
}

impl LanguageImpl for GoLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_go::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "go"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_declaration" |
            "method_declaration" |
            "type_declaration" |
            "struct_type" |
            "interface_type" |
            // "const_declaration" |
            // "var_declaration" |
            // "const_spec" |
            // "var_spec" |
            // "short_var_declaration" |
            "type_spec" // Added for type definitions
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // Go: Check function_declaration nodes with names starting with Test
        if node_type == "function_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("Test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Go): Test function");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }

    fn find_parent_function<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";

        if debug_mode {
            println!(
                "DEBUG: Finding parent function for {node_kind}",
                node_kind = node.kind()
            );
        }

        let mut current = node;

        while let Some(parent) = current.parent() {
            if parent.kind() == "function_declaration" || parent.kind() == "method_declaration" {
                if debug_mode {
                    println!(
                        "DEBUG: Found parent function: {parent_kind}",
                        parent_kind = parent.kind()
                    );
                }
                return Some(parent);
            }
            current = parent;
        }

        if debug_mode {
            println!(
                "DEBUG: No parent function found for {node_kind}",
                node_kind = node.kind()
            );
        }

        None
    }
}
