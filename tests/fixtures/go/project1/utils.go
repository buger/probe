package main

// Add performs addition of two integers
// This function should show incoming calls from Calculate
func Add(x, y int) int {
    return x + y
}

// Multiply performs multiplication of two integers  
// This function should show incoming calls from Calculate
func Multiply(x, y int) int {
    return x * y
}

// Subtract performs subtraction of two integers
// This function should show incoming calls from Calculate
func Subtract(x, y int) int {
    return x - y
}

// Divide performs division of two integers
// This function might not have incoming calls in our test
func Divide(x, y int) int {
    if y == 0 {
        return 0
    }
    return x / y
}

// UtilityHelper demonstrates a function that calls other utilities
func UtilityHelper(a, b int) int {
    temp := Add(a, b)      // Outgoing call to Add
    return Multiply(temp, 3) // Outgoing call to Multiply
}