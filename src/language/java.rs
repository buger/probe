use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for Java
pub struct JavaLanguage;

impl Default for JavaLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaLanguage {
    pub fn new() -> Self {
        JavaLanguage
    }
}

impl LanguageImpl for JavaLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_java::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "java"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "method_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "constructor_declaration"
                | "field_declaration"
                | "variable_declaration"
                | "block"
                | "static_initializer"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // Java: Check method_declaration nodes with @Test annotation
        if node_type == "method_declaration" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "modifiers" {
                    let mut subcursor = child.walk();
                    for annotation in child.children(&mut subcursor) {
                        if annotation.kind() == "annotation" {
                            let annotation_text = annotation.utf8_text(source).unwrap_or("");
                            if annotation_text.contains("@Test") {
                                if debug_mode {
                                    println!("DEBUG: Test node detected (Java): @Test method");
                                }
                                return true;
                            }
                        }
                    }
                }
            }
        }

        false
    }
}
