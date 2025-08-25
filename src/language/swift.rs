use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for Swift
pub struct SwiftLanguage;

impl Default for SwiftLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl SwiftLanguage {
    pub fn new() -> Self {
        SwiftLanguage
    }
}

impl LanguageImpl for SwiftLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "swift"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "function_declaration"
                | "class_declaration"
                | "struct_declaration"
                | "enum_declaration"
                | "protocol_declaration"
                | "extension_declaration"
                | "typealias_declaration"
                | "variable_declaration"
                | "constant_declaration"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // Swift: Check for test functions in XCTest
        if node_type == "function_declaration" {
            let mut cursor = node.walk();

            // Look for function name starting with "test"
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Swift): test function");
                        }
                        return true;
                    }
                }
            }

            // Also check for @Test attribute
            for child in node.children(&mut cursor) {
                if child.kind() == "attribute" {
                    let attr_text = child.utf8_text(source).unwrap_or("");
                    if attr_text.contains("@Test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Swift): @Test attribute");
                        }
                        return true;
                    }
                }
            }
        }

        // Also check for XCTestCase class declarations
        if node_type == "class_declaration" {
            let mut cursor = node.walk();

            // Check if class inherits from XCTestCase
            for child in node.children(&mut cursor) {
                if child.kind() == "type_inheritance_clause" {
                    let inheritance_text = child.utf8_text(source).unwrap_or("");
                    if inheritance_text.contains("XCTestCase") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (Swift): XCTestCase class");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }

    fn find_parent_function<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        let mut current = node;

        while let Some(parent) = current.parent() {
            if parent.kind() == "function_declaration" {
                return Some(parent);
            }
            current = parent;
        }

        None
    }
}
