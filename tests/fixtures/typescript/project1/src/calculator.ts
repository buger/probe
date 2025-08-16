// Calculator module with core business logic

import { add, multiply, subtract } from './utils';

/**
 * Calculate performs a complex calculation using utility functions
 * This function should show both incoming calls (from main, processNumbers, Calculator)
 * and outgoing calls (to add, multiply, subtract)
 * @param a First operand
 * @param b Second operand
 * @returns Calculated result
 */
export function calculate(a: number, b: number): number {
    const sum = add(a, b);        // Outgoing call to add
    let result = multiply(sum, 2); // Outgoing call to multiply
    
    // Additional logic for testing
    if (result > 50) {
        result = subtract(result, 10); // Outgoing call to subtract
    }
    
    return result;
}

/**
 * Advanced calculation that also uses the calculate function
 * This creates another pathway in the call hierarchy
 * @param values Array of values to process
 * @returns Advanced result
 */
export function advancedCalculation(values: number[]): number {
    let total = 0;
    for (const value of values) {
        total += calculate(value, 1); // Incoming call to calculate
    }
    return total;
}

/**
 * Business logic interface
 */
export interface IBusinessLogic {
    processValue(value: number): number;
}

/**
 * Complex business logic class
 */
export class BusinessLogic implements IBusinessLogic {
    private readonly multiplier: number;
    
    constructor(multiplier: number) {
        this.multiplier = multiplier;
    }
    
    /**
     * Process a value using internal logic
     * @param value Input value
     * @returns Processed result
     */
    processValue(value: number): number {
        return calculate(value, this.multiplier); // Another incoming call to calculate
    }
    
    /**
     * Complex processing that chains multiple calls
     * @param data Array of input data
     * @returns Processed results
     */
    processArray(data: number[]): number[] {
        return data.map(item => {
            const intermediate = add(item, 5); // Outgoing call to add
            return calculate(intermediate, 2); // Outgoing call to calculate
        });
    }
}