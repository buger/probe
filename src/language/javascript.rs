use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for JavaScript
pub struct JavaScriptLanguage;

impl Default for JavaScriptLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaScriptLanguage {
    pub fn new() -> Self {
        JavaScriptLanguage
    }
}

impl LanguageImpl for JavaScriptLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "js"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_declaration"
                | "object"
                | "array"
                | "function_expression"
                | "method_definition"
                | "class_declaration"
                | "arrow_function"
                | "function"
                | "export_statement"
                | "jsx_element"
                | "jsx_self_closing_element"
                | "property_identifier"
                | "class_body"
                | "class"
                | "expression_statement"
                | "block"
                | "statement_block"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // JavaScript: Check for describe/it/test blocks
        if node_type == "function_declaration"
            || node_type == "method_definition"
            || node_type == "arrow_function"
        {
            let mut cursor = node.walk();

            // Try to find the function/method name
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.contains("test") || name.contains("Test") {
                        if debug_mode {
                            println!(
                                "DEBUG: Test node detected (JavaScript): test function/method"
                            );
                        }
                        return true;
                    }
                }
            }
        }

        // Check for call expressions like describe(), it(), test()
        if node_type == "call_expression" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name == "describe" || name == "it" || name == "test" || name == "expect" {
                        if debug_mode {
                            println!("DEBUG: Test node detected (JavaScript): {name} call");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }
}
