// JavaScript test file for comprehensive LSP call hierarchy testing

const { calculate, advancedCalculation, BusinessLogic } = require('./calculator');
const { add, multiply, utilityHelper } = require('./utils');

/**
 * Main entry point of the application
 */
function main() {
    console.log("JavaScript LSP Test Project");
    
    // Test calculate function with call hierarchy
    const result = calculate(10, 5);
    console.log(`Calculate result: ${result}`);
    
    // Test utility functions directly
    const sum = add(15, 25);
    const product = multiply(4, 8);
    
    console.log(`Direct add result: ${sum}`);
    console.log(`Direct multiply result: ${product}`);
    
    // Test business logic
    const processedData = processNumbers([1, 2, 3, 4, 5]);
    console.log(`Processed data: ${processedData}`);
    
    // Test class-based functionality
    const businessLogic = new BusinessLogic(3);
    const classResult = businessLogic.processValue(7);
    console.log(`Class result: ${classResult}`);
    
    // Test advanced calculation
    const advancedResult = advancedCalculation([2, 4, 6]);
    console.log(`Advanced result: ${advancedResult}`);
}

/**
 * Processes an array of numbers using calculate function
 * This creates another incoming call to calculate
 * @param {number[]} numbers Array of numbers to process
 * @returns {number[]} Processed array
 */
function processNumbers(numbers) {
    return numbers.map(num => calculate(num, 2)); // Incoming call to calculate
}

/**
 * Calculator class for testing method call hierarchy
 */
class Calculator {
    /**
     * @param {number} multiplier The multiplier value
     */
    constructor(multiplier) {
        this.multiplier = multiplier;
    }
    
    /**
     * Instance method that calls calculate function
     * @param {number} value Input value
     * @returns {number} Processed value
     */
    processValue(value) {
        return calculate(value, this.multiplier); // Another incoming call to calculate
    }
    
    /**
     * Static method for additional testing
     * @param {number} x Input value
     * @returns {number} Processed value
     */
    static staticProcess(x) {
        return multiply(x, 4); // Incoming call to multiply
    }
}

/**
 * Demonstrates function composition and call chains
 * @param {number} input Initial input
 * @returns {number} Final result
 */
function compositeFunction(input) {
    const step1 = utilityHelper(input, 5); // Outgoing call to utilityHelper
    const step2 = calculate(step1, 3);     // Outgoing call to calculate
    return multiply(step2, 2);             // Outgoing call to multiply
}

// Export functions and classes
module.exports = {
    main,
    processNumbers,
    Calculator,
    compositeFunction
};

// Run main if this is the entry point
if (require.main === module) {
    main();
}