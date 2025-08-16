// JavaScript test file for LSP call hierarchy testing

/**
 * Adds two numbers together
 * @param {number} a First number
 * @param {number} b Second number
 * @returns {number} Sum of a and b
 */
function add(a, b) {
    return a + b;
}

/**
 * Multiplies two numbers
 * @param {number} a First number
 * @param {number} b Second number
 * @returns {number} Product of a and b
 */
function multiply(a, b) {
    return a * b;
}

/**
 * Calculates a complex result using add and multiply functions
 * This function should show up in call hierarchy with incoming/outgoing calls
 * @param {number} x First input
 * @param {number} y Second input
 * @returns {number} Calculated result
 */
function calculate(x, y) {
    const sum = add(x, y);           // Outgoing call to add()
    const result = multiply(sum, 2); // Outgoing call to multiply()
    return result;
}

/**
 * Main function that calls calculate
 * This should show as an incoming call to calculate()
 */
function main() {
    console.log("JavaScript LSP Test");
    
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
 * @param {number[]} data Array of numbers to process
 * @returns {number[]} Processed array
 */
function processData(data) {
    return data.map(value => calculate(value, 1)); // Another incoming call to calculate()
}

/**
 * Class-based example for testing method call hierarchy
 */
class Calculator {
    /**
     * Instance method that calls calculate function
     * @param {number} a First number
     * @param {number} b Second number
     * @returns {number} Result
     */
    compute(a, b) {
        return calculate(a, b); // Call to calculate function
    }
    
    /**
     * Static method for additional testing
     * @param {number} x Input value
     * @returns {number} Processed value
     */
    static process(x) {
        return multiply(x, 3); // Call to multiply function
    }
}

// Export functions for module system
module.exports = {
    add,
    multiply,
    calculate,
    main,
    processData,
    Calculator
};

// Run main if this is the entry point
if (require.main === module) {
    main();
}