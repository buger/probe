// TypeScript test file for comprehensive LSP call hierarchy testing

import { calculate, advancedCalculation, BusinessLogic } from './calculator';
import { add, multiply, utilityHelper } from './utils';

/**
 * Main entry point of the application
 */
function main(): void {
    console.log("TypeScript LSP Test Project");
    
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
function processNumbers(numbers: number[]): number[] {
    return numbers.map(num => calculate(num, 2)); // Incoming call to calculate
}

/**
 * Calculator class for testing method call hierarchy
 */
class Calculator {
    private multiplier: number;
    
    constructor(multiplier: number) {
        this.multiplier = multiplier;
    }
    
    /**
     * Instance method that calls calculate function
     * @param value Input value
     * @returns Processed value
     */
    processValue(value: number): number {
        return calculate(value, this.multiplier); // Another incoming call to calculate
    }
    
    /**
     * Static method for additional testing
     * @param x Input value
     * @returns Processed value
     */
    static staticProcess(x: number): number {
        return multiply(x, 4); // Incoming call to multiply
    }
}

// Export main function and other exports
export { main, processNumbers, Calculator };

// Run main if this is the entry point
if (require.main === module) {
    main();
}