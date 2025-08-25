use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for Python
pub struct PythonLanguage;

impl Default for PythonLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonLanguage {
    pub fn new() -> Self {
        PythonLanguage
    }
}

impl LanguageImpl for PythonLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_python::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "py"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_definition"
                | "class_definition"
                | "module"
                | "assignment"
                | "expression_statement"
                | "block"
                | "decorated_definition"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // Python: Check function_definition nodes with names starting with test_
        if node_type == "function_definition" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("test_") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Python): test_ function");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }
}
