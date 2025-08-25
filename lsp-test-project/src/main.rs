use std::collections::HashMap;

fn main() {
    println!("LSP Test Project");

    let data = setup_data();
    process_data(&data);

    let result = calculate_result(10, 20);
    display_result(result);

    let numbers = vec![1, 2, 3, 4, 5];
    let processed = process_numbers(numbers);
    println!("Processed numbers: {processed:?}");
}

fn setup_data() -> HashMap<String, i32> {
    let mut map = HashMap::new();
    map.insert("first".to_string(), 1);
    map.insert("second".to_string(), 2);
    map.insert("third".to_string(), 3);

    // This function calls helper functions
    let additional_data = create_additional_data();
    map.extend(additional_data);

    map
}

fn create_additional_data() -> HashMap<String, i32> {
    let mut additional = HashMap::new();
    additional.insert("fourth".to_string(), 4);
    additional.insert("fifth".to_string(), 5);
    additional
}

fn process_data(data: &HashMap<String, i32>) {
    println!("Processing data with {} entries", data.len());

    for (key, value) in data {
        validate_entry(key, *value);
    }

    let sum = calculate_sum(data);
    println!("Total sum: {sum}");
}

fn validate_entry(key: &str, value: i32) {
    if value < 0 {
        println!("Warning: negative value for key '{key}'");
    }

    // Call utility function
    let formatted = format_entry(key, value);
    println!("Formatted: {formatted}");
}

fn format_entry(key: &str, value: i32) -> String {
    format!("{key}={value}")
}

fn calculate_sum(data: &HashMap<String, i32>) -> i32 {
    data.values().sum()
}

fn calculate_result(a: i32, b: i32) -> i32 {
    let intermediate = perform_calculation(a, b);
    apply_modifier(intermediate)
}

fn perform_calculation(x: i32, y: i32) -> i32 {
    x + y + get_bonus()
}

fn get_bonus() -> i32 {
    42
}

fn apply_modifier(value: i32) -> i32 {
    value * 2
}

fn display_result(result: i32) {
    println!("Final result: {result}");

    if result > 100 {
        print_large_result(result);
    } else {
        print_small_result(result);
    }
}

fn print_large_result(value: i32) {
    println!("That's a large result: {value}");
}

fn print_small_result(value: i32) {
    println!("That's a small result: {value}");
}

fn process_numbers(numbers: Vec<i32>) -> Vec<i32> {
    numbers
        .into_iter()
        .map(transform_number)
        .filter(|&n| filter_number(n))
        .collect()
}

fn transform_number(n: i32) -> i32 {
    n * 3 + 1
}

fn filter_number(n: i32) -> bool {
    n % 2 == 0
}

// Additional utility functions that create a complex call graph
pub fn public_api_function(input: &str) -> String {
    let processed = internal_processor(input);
    finalize_output(processed)
}

fn internal_processor(input: &str) -> String {
    let step1 = preprocessing_step(input);
    let step2 = main_processing_step(&step1);
    postprocessing_step(step2)
}

fn preprocessing_step(input: &str) -> String {
    format!("preprocessed_{input}")
}

fn main_processing_step(input: &str) -> String {
    let helper_result = processing_helper(input);
    format!("main_processed_{helper_result}")
}

fn processing_helper(input: &str) -> String {
    format!("helper_{input}")
}

fn postprocessing_step(input: String) -> String {
    format!("postprocessed_{input}")
}

fn finalize_output(input: String) -> String {
    format!("final_{input}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_result() {
        let result = calculate_result(5, 10);
        assert_eq!(result, 114); // (5 + 10 + 42) * 2 = 114
    }

    #[test]
    fn test_public_api_function() {
        let result = public_api_function("test");
        assert_eq!(
            result,
            "final_postprocessed_main_processed_helper_preprocessed_test"
        );
    }

    #[test]
    fn test_process_numbers() {
        let numbers = vec![1, 2, 3, 4];
        let result = process_numbers(numbers);
        // Transform: 1*3+1=4, 2*3+1=7, 3*3+1=10, 4*3+1=13
        // Filter evens: 4, 10 (7 and 13 are odd)
        assert_eq!(result, vec![4, 10]);
    }
}
