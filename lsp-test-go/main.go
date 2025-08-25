package main

import "fmt"

func main() {
    result := Calculate(5, 3)
    fmt.Printf("Result: %d\n", result)
}

func Calculate(a, b int) int {
    sum := Add(a, b)
    return Multiply(sum, 2)
}

func Add(x, y int) int {
    return x + y
}

func Multiply(x, y int) int {
    return x * y
}
