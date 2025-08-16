"use strict";
// TypeScript test file for comprehensive LSP call hierarchy testing
Object.defineProperty(exports, "__esModule", { value: true });
exports.Calculator = void 0;
exports.main = main;
exports.processNumbers = processNumbers;
const calculator_1 = require("./calculator");
const utils_1 = require("./utils");
/**
 * Main entry point of the application
 */
function main() {
    console.log("TypeScript LSP Test Project");
    // Test calculate function with call hierarchy
    const result = (0, calculator_1.calculate)(10, 5);
    console.log(`Calculate result: ${result}`);
    // Test utility functions directly
    const sum = (0, utils_1.add)(15, 25);
    const product = (0, utils_1.multiply)(4, 8);
    console.log(`Direct add result: ${sum}`);
    console.log(`Direct multiply result: ${product}`);
    // Test business logic
    const processedData = processNumbers([1, 2, 3, 4, 5]);
    console.log(`Processed data: ${processedData}`);
    // Test class-based functionality
    const calculator = new Calculator(3);
    const classResult = calculator.processValue(7);
    console.log(`Class result: ${classResult}`);
}
/**
 * Processes an array of numbers using calculate function
 * This creates another incoming call to calculate
 * @param numbers Array of numbers to process
 * @returns Processed array
 */
function processNumbers(numbers) {
    return numbers.map(num => (0, calculator_1.calculate)(num, 2)); // Incoming call to calculate
}
/**
 * Calculator class for testing method call hierarchy
 */
class Calculator {
    constructor(multiplier) {
        this.multiplier = multiplier;
    }
    /**
     * Instance method that calls calculate function
     * @param value Input value
     * @returns Processed value
     */
    processValue(value) {
        return (0, calculator_1.calculate)(value, this.multiplier); // Another incoming call to calculate
    }
    /**
     * Static method for additional testing
     * @param x Input value
     * @returns Processed value
     */
    static staticProcess(x) {
        return (0, utils_1.multiply)(x, 4); // Incoming call to multiply
    }
}
exports.Calculator = Calculator;
// Run main if this is the entry point
if (require.main === module) {
    main();
}
//# sourceMappingURL=main.js.map