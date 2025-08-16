package main

import "fmt"

// Main function - entry point of the application
func main() {
    fmt.Println("Go LSP Test Project")
    
    // Test calculate function with call hierarchy
    result := Calculate(10, 5)
    fmt.Printf("Calculate result: %d\n", result)
    
    // Test utility functions directly
    sum := Add(15, 25)
    product := Multiply(4, 8)
    
    fmt.Printf("Direct add result: %d\n", sum)
    fmt.Printf("Direct multiply result: %d\n", product)
    
    // Test business logic
    processedData := ProcessNumbers([]int{1, 2, 3, 4, 5})
    fmt.Printf("Processed data: %v\n", processedData)
}

// ProcessNumbers processes an array of numbers using Calculate
// This creates another incoming call to Calculate
func ProcessNumbers(numbers []int) []int {
    var results []int
    for _, num := range numbers {
        result := Calculate(num, 2) // Incoming call to Calculate
        results = append(results, result)
    }
    return results
}