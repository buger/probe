use super::language_trait::LanguageImpl;
use tree_sitter::{Language as TSLanguage, Node};

/// Implementation of LanguageImpl for TypeScript
pub struct TypeScriptLanguage {
    tsx: bool,
}

impl TypeScriptLanguage {
    pub fn new_typescript() -> Self {
        TypeScriptLanguage { tsx: false }
    }

    pub fn new_tsx() -> Self {
        TypeScriptLanguage { tsx: true }
    }
}

impl LanguageImpl for TypeScriptLanguage {
    fn get_tree_sitter_language(&self) -> TSLanguage {
        if self.tsx {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        } else {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        }
    }

    fn get_extension(&self) -> &'static str {
        if self.tsx {
            "tsx"
        } else {
            "ts"
        }
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
                | "interface_declaration"  // TypeScript specific
                | "type_alias_declaration" // TypeScript specific
                | "enum_declaration" // TypeScript specific
                | "declare_statement" // TypeScript declare functions/variables
                | "namespace_declaration" // TypeScript namespaces
                | "module_declaration" // Alternative name for namespace in some parsers
        )
    }

    fn is_test_node(&self, node: &Node, source: &[u8]) -> bool {
        // TypeScript test detection is the same as JavaScript
        let debug_mode = std::env::var("PROBE_DEBUG").unwrap_or_default() == "1";
        let node_type = node.kind();

        // TypeScript: Check for describe/it/test blocks
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
                                "DEBUG: Test node detected (TypeScript): test function/method"
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
                            println!("DEBUG: Test node detected (TypeScript): {name} call");
                        }
                        return true;
                    }
                }
            }
        }

        false
    }
}
