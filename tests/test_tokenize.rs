fn main() {
    // Import the tokenize function from our probe crate
    use probe_code::ranking::tokenize;

    // Test strings
    let test_strings = [
        "The quick brown fox jumps over the lazy dog",
        "function calculateTotal(items) { return items.reduce((sum, item) => sum + item.price, 0); }",
        "class UserController extends BaseController implements UserInterface",
        "Searching for files containing important information",
        "Fruitlessly searching for the missing variable in the codebase",
    ];

    println!("Testing tokenization with stop word removal and stemming:\n");

    for (i, test_str) in test_strings.iter().enumerate() {
        println!("Original text {}:\n{}", i + 1, test_str);

        // Tokenize with stop word removal and stemming
        let tokens = tokenize(test_str);

        println!("Tokens after stop word removal and stemming:");
        println!("{tokens:?}");
        println!("Number of tokens: {}\n", tokens.len());
    }

    // Specific test for stemming
    println!("Specific stemming test:");
    println!("'fruitlessly' stems to: {}", tokenize("fruitlessly")[0]);
}
