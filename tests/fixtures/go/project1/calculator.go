package main

// Calculate performs a complex calculation using utility functions
// This function should show both incoming calls (from main, ProcessNumbers)
// and outgoing calls (to Add, Multiply)
func Calculate(a, b int) int {
    sum := Add(a, b)        // Outgoing call to Add
    result := Multiply(sum, 2) // Outgoing call to Multiply
    
    // Additional logic for testing
    if result > 50 {
        result = Subtract(result, 10) // Outgoing call to Subtract
    }
    
    return result
}

// BusinessLogic contains more complex business operations
type BusinessLogic struct {
    multiplier int
}

// ProcessValue method that calls Calculate
func (bl *BusinessLogic) ProcessValue(value int) int {
    return Calculate(value, bl.multiplier) // Another incoming call to Calculate
}

// NewBusinessLogic creates a new BusinessLogic instance
func NewBusinessLogic(multiplier int) *BusinessLogic {
    return &BusinessLogic{multiplier: multiplier}
}