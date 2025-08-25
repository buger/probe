use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for C#
pub struct CSharpLanguage;

impl Default for CSharpLanguage {
    fn default() -> Self {
        Self::new()
    }
}

impl CSharpLanguage {
    pub fn new() -> Self {
        CSharpLanguage
    }
}

impl LanguageImpl for CSharpLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn get_extension(&self) -> &'static str {
        "cs"
    }

    fn is_acceptable_parent(&self, node: &Node) -> bool {
        matches!(
            node.kind(),
            "method_declaration"
                | "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "namespace_declaration"
                | "property_declaration"
                | "constructor_declaration"
                | "delegate_declaration"
                | "event_declaration"
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // C#: Check for test methods with attributes
        if node_type == "method_declaration" {
            let mut cursor = node.walk();

            // Check for test attributes (NUnit, MSTest, xUnit)
            let mut has_test_attribute = false;

            for child in node.children(&mut cursor) {
                if child.kind() == "attribute_list" {
                    let attr_text = child.utf8_text(source).unwrap_or("");
                    if attr_text.contains("[Test")
                        || attr_text.contains("[TestMethod")
                        || attr_text.contains("[Fact")
                        || attr_text.contains("[Theory")
                    {
                        has_test_attribute = true;
                        break;
                    }
                }
            }

            if has_test_attribute {
                if debug_mode {
                    println!("DEBUG: Test node detected (C#): test attribute");
                }
                return true;
            }

            // Also check for method name starting with "Test"
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.starts_with("Test") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (C#): Test method");
                        }
                        return true;
                    }
                }
            }
        }

        // Check for test classes
        if node_type == "class_declaration" {
            let mut cursor = node.walk();

            // Check for test class attributes
            for child in node.children(&mut cursor) {
                if child.kind() == "attribute_list" {
                    let attr_text = child.utf8_text(source).unwrap_or("");
                    if attr_text.contains("[TestClass") || attr_text.contains("[TestFixture") {
                        if debug_mode {
                            println!("DEBUG: Test node detected (C#): test class attribute");
                        }
                        return true;
                    }
                }
            }

            // Check for class name ending with "Tests" or "Test"
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = child.utf8_text(source).unwrap_or("");
                    if name.ends_with("Tests") || name.ends_with("Test") {
                        if debug_mode {
                            println!(
                                "DEBUG: Test node detected (C#): Test class naming convention"
                            );
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
            if parent.kind() == "method_declaration" {
                return Some(parent);
            }
            current = parent;
        }

        None
    }
}
