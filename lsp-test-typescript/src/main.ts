// TypeScript test file for LSP call hierarchy testing

/**
 * Adds two numbers together
 * @param a First number
 * @param b Second number
 * @returns Sum of a and b
 */
function add(a: number, b: number): number {
    return a + b;
}

/**
 * Multiplies two numbers
 * @param a First number
 * @param b Second number
 * @returns Product of a and b
 */
function multiply(a: number, b: number): number {
    return a * b;
}

/**
 * Calculates a complex result using add and multiply functions
 * This function should show up in call hierarchy with incoming/outgoing calls
 * @param x First input
 * @param y Second input
 * @returns Calculated result
 */
function calculate(x: number, y: number): number {
    const sum = add(x, y);           // Outgoing call to add()
    const result = multiply(sum, 2); // Outgoing call to multiply()
    return result;
}

/**
 * Main function that calls calculate
 * This should show as an incoming call to calculate()
 */
function main(): void {
    console.log("TypeScript LSP Test");
    
    const result = calculate(5, 3);  // Outgoing call to calculate()
    console.log(`Result: ${result}`);
    
    // Additional calls for testing
    const directSum = add(10, 20);
    const directProduct = multiply(4, 7);
    
    console.log(`Direct sum: ${directSum}`);
    console.log(`Direct product: ${directProduct}`);
}

/**
 * Another function that calls calculate for testing multiple incoming calls
 */
function processData(data: number[]): number[] {
    return data.map(value => calculate(value, 1)); // Another incoming call to calculate()
}

// Export functions for module system
export { add, multiply, calculate, main, processData };

// Run main if this is the entry point
if (require.main === module) {
    main();
}